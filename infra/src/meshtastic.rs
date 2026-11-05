//! Meshtastic transport: carries Skein packets as `PRIVATE_APP` (port 256) payloads through a Meshtastic radio.
//!
//! The radio is reached over the Meshtastic stream API on TCP (a node with Wi-Fi listens on port 4403, as does
//! `meshtasticd`). Messages are protobuf, framed as `0x94 0xC3 <len u16 BE> <protobuf>`. Only the handful of fields we need
//! are encoded here by hand, so there is no protobuf dependency. USB serial uses the same framing but is not wired up yet
//! (bridge it with `ser2net`/`socat` for now).
//!
//! Skein does all the routing itself, so packets are sent with `hop_limit` 0 (the radio never re-floods them).
//! The Meshtastic channel key still encrypts the radio hop; Skein's own end-to-end encryption sits on top of it.
use crate::transport::Transport;
use async_trait::async_trait;
use std::{io, sync::{atomic::{AtomicU32, Ordering}, Mutex}};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{tcp::OwnedWriteHalf, TcpStream},
    sync::{mpsc, Mutex as AsyncMutex},
};

pub const PRIVATE_APP: u64 = 256;
/// Largest `Data.payload` a Meshtastic packet carries.
pub const MAX_PAYLOAD: usize = 233;
pub const BROADCAST: u32 = 0xFFFF_FFFF;

// ── Minimal protobuf encoding ────────────────────────────────────────────────
fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}
fn put_tag(out: &mut Vec<u8>, field: u32, wire: u8) {
    put_varint(out, ((field << 3) | wire as u32) as u64);
}
fn put_uint(out: &mut Vec<u8>, field: u32, v: u64) {
    put_tag(out, field, 0);
    put_varint(out, v);
}
fn put_fixed32(out: &mut Vec<u8>, field: u32, v: u32) {
    put_tag(out, field, 5);
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
    put_tag(out, field, 2);
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

enum Val<'a> {
    Var(u64),
    Fix32(u32),
    Bytes(&'a [u8]),
}

fn get_varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let x = *b.get(*i)?;
        *i += 1;
        v |= ((x & 0x7f) as u64) << shift;
        if x & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

fn parse(b: &[u8]) -> Vec<(u32, Val<'_>)> {
    let (mut i, mut out) = (0, vec![]);
    while i < b.len() {
        let Some(t) = get_varint(b, &mut i) else { break };
        let val = match t & 7 {
            0 => get_varint(b, &mut i).map(Val::Var),
            1 => {
                i += 8;
                None
            }
            2 => get_varint(b, &mut i).and_then(|n| {
                let end = i.checked_add(n as usize).filter(|e| *e <= b.len())?;
                let s = &b[i..end];
                i = end;
                Some(Val::Bytes(s))
            }),
            5 => b.get(i..i + 4).map(|x| {
                i += 4;
                Val::Fix32(u32::from_le_bytes(x.try_into().unwrap()))
            }),
            _ => None,
        };
        match val {
            Some(v) => out.push(((t >> 3) as u32, v)),
            None => break,
        }
    }
    out
}

// ── Messages ─────────────────────────────────────────────────────────────────
/// `MeshPacket { from?, to, decoded: Data { portnum: PRIVATE_APP, payload }, id }`
fn mesh_packet(from: Option<u32>, to: u32, id: u32, payload: &[u8]) -> Vec<u8> {
    let mut data = vec![];
    put_uint(&mut data, 1, PRIVATE_APP);
    put_bytes(&mut data, 2, payload);
    let mut p = vec![];
    if let Some(f) = from {
        put_fixed32(&mut p, 1, f);
    }
    put_fixed32(&mut p, 2, to);
    put_bytes(&mut p, 4, &data);
    put_fixed32(&mut p, 6, id);
    p // hop_limit stays 0: Skein routes, the radio must not re-flood
}

/// `ToRadio { packet }`
pub fn to_radio_packet(to: u32, id: u32, payload: &[u8]) -> Vec<u8> {
    let mut o = vec![];
    put_bytes(&mut o, 1, &mesh_packet(None, to, id, payload));
    o
}
/// `ToRadio { want_config_id }`: asks the radio to start streaming to us.
pub fn to_radio_hello(nonce: u32) -> Vec<u8> {
    let mut o = vec![];
    put_uint(&mut o, 3, nonce as u64);
    o
}
/// `FromRadio { packet }` as a radio would deliver a received message (used by the tests' fake radio).
pub fn from_radio_packet(from: u32, to: u32, id: u32, payload: &[u8]) -> Vec<u8> {
    let mut o = vec![];
    put_bytes(&mut o, 2, &mesh_packet(Some(from), to, id, payload));
    o
}
pub fn frame(msg: &[u8]) -> Vec<u8> {
    let mut f = vec![0x94, 0xC3, (msg.len() >> 8) as u8, msg.len() as u8];
    f.extend_from_slice(msg);
    f
}

#[derive(Debug, PartialEq)]
pub enum FromRadio {
    /// A `PRIVATE_APP` packet from node `from`.
    Packet { from: u32, to: u32, payload: Vec<u8> },
    /// Our own node number (from `MyNodeInfo`).
    MyNum(u32),
    Other,
}

pub fn decode_from_radio(buf: &[u8]) -> FromRadio {
    for (f, v) in parse(buf) {
        match (f, v) {
            (2, Val::Bytes(p)) => {
                let (mut from, mut to, mut payload, mut port) = (0, 0, None, 0);
                for (pf, pv) in parse(p) {
                    match (pf, pv) {
                        (1, Val::Fix32(x)) => from = x,
                        (2, Val::Fix32(x)) => to = x,
                        (4, Val::Bytes(d)) => {
                            for (df, dv) in parse(d) {
                                match (df, dv) {
                                    (1, Val::Var(x)) => port = x,
                                    (2, Val::Bytes(b)) => payload = Some(b.to_vec()),
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
                return match payload {
                    Some(payload) if port == PRIVATE_APP => FromRadio::Packet { from, to, payload },
                    _ => FromRadio::Other,
                };
            }
            (3, Val::Bytes(info)) => {
                if let Some((_, Val::Var(n))) = parse(info).into_iter().find(|(f, _)| *f == 1) {
                    return FromRadio::MyNum(n as u32);
                }
            }
            _ => {}
        }
    }
    FromRadio::Other
}

/// What a radio receives from the app: `(to, id, payload)` of a `PRIVATE_APP` packet (used by the tests' fake radio).
pub fn decode_to_radio_packet(buf: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let Some((_, Val::Bytes(p))) = parse(buf).into_iter().find(|(f, _)| *f == 1) else { return None };
    let (mut to, mut id, mut payload) = (0, 0, None);
    for (f, v) in parse(p) {
        match (f, v) {
            (2, Val::Fix32(x)) => to = x,
            (6, Val::Fix32(x)) => id = x,
            (4, Val::Bytes(d)) => payload = parse(d).into_iter().find_map(|(f, v)| if let (2, Val::Bytes(b)) = (f, v) { Some(b.to_vec()) } else { None }),
            _ => {}
        }
    }
    payload.map(|pl| (to, id, pl))
}

/// Read one frame, skipping noise (the radio also prints debug text on the stream).
pub async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> io::Result<Vec<u8>> {
    loop {
        if r.read_u8().await? != 0x94 || r.read_u8().await? != 0xC3 {
            continue;
        }
        let len = r.read_u16().await? as usize;
        if len > 512 {
            continue;
        }
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf).await?;
        return Ok(buf);
    }
}

// ── The transport ────────────────────────────────────────────────────────────
pub struct Meshtastic {
    writer: AsyncMutex<OwnedWriteHalf>,
    rx: AsyncMutex<mpsc::Receiver<(String, Vec<u8>)>>,
    peers: std::sync::Arc<Mutex<Vec<String>>>,
    next_id: AtomicU32,
}

/// Peers are named after the radio node number: `!0a1b2c3d`.
fn peer_name(n: u32) -> String {
    format!("!{n:08x}")
}
fn peer_num(s: &str) -> Option<u32> {
    if s == "broadcast" {
        return Some(BROADCAST);
    }
    u32::from_str_radix(s.strip_prefix('!')?, 16).ok()
}

impl Meshtastic {
    /// Connect to a radio's TCP stream API, e.g. `192.168.1.50:4403`.
    pub async fn connect(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        let (mut rd, mut wr) = stream.into_split();
        wr.write_all(&frame(&to_radio_hello(rand_u32()))).await?;
        let (tx, rx) = mpsc::channel(256);
        let peers = std::sync::Arc::new(Mutex::new(vec![]));
        let known = peers.clone();
        tokio::spawn(async move {
            let my = AtomicU32::new(0);
            while let Ok(buf) = read_frame(&mut rd).await {
                match decode_from_radio(&buf) {
                    FromRadio::MyNum(n) => my.store(n, Ordering::Relaxed),
                    FromRadio::Packet { from, to, payload } if from != my.load(Ordering::Relaxed) => {
                        let me = my.load(Ordering::Relaxed);
                        if to == BROADCAST || me == 0 || to == me {
                            let name = peer_name(from);
                            {
                                let mut k = known.lock().unwrap();
                                if !k.contains(&name) {
                                    k.push(name.clone());
                                }
                            }
                            if tx.send((name, payload)).await.is_err() {
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok(Meshtastic { writer: AsyncMutex::new(wr), rx: AsyncMutex::new(rx), peers, next_id: AtomicU32::new(rand_u32()) })
    }
}

fn rand_u32() -> u32 {
    use rand_core::{OsRng, RngCore};
    OsRng.next_u32()
}

#[async_trait]
impl Transport for Meshtastic {
    fn name(&self) -> &str {
        "meshtastic"
    }
    fn mtu(&self) -> usize {
        MAX_PAYLOAD
    }
    async fn send(&self, peer: &str, data: &[u8]) -> io::Result<()> {
        if data.len() > MAX_PAYLOAD {
            return Err(io::Error::other("packet larger than a Meshtastic payload"));
        }
        let to = peer_num(peer).ok_or_else(|| io::Error::other("bad Meshtastic peer"))?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.writer.lock().await.write_all(&frame(&to_radio_packet(to, id, data))).await
    }
    async fn recv(&self) -> io::Result<(String, Vec<u8>)> {
        self.rx.lock().await.recv().await.ok_or_else(|| io::Error::new(io::ErrorKind::ConnectionAborted, "radio connection closed"))
    }
    fn peers(&self) -> Vec<String> {
        self.peers.lock().unwrap().clone()
    }
    /// One broadcast reaches every neighbour in radio range.
    fn flood_peers(&self) -> Vec<String> {
        vec!["broadcast".into()]
    }
}
