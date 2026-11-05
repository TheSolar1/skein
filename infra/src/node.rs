//! A Skein node: signed announcements -> routes, name gossip, encrypted request/response with chunked bulk transfer.
use crate::crypto::{open, seal, session_key, session_open, session_seal};
use crate::identity::{addr_of, fmt_addr, verify, Addr, Identity};
use crate::naming::{valid_name, NameRecord, Names};
use crate::packet::*;
use crate::routing::{Link, Routes};
use crate::transport::Transport;
use rand_core::{OsRng, RngCore};
use std::{
    collections::HashMap,
    fmt,
    path::{Component, Path, PathBuf},
    sync::{atomic::{AtomicU64, Ordering}, Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_FILE: u64 = 512 << 20;
/// Bytes of a chunk message that are not file data: packet header, sealed box, signature, key, framing.
const OVERHEAD: usize = 220;
/// Radio links (below this MTU) use compact session packets: header 34 + counter 8 + tag 16 + message framing.
const RADIO_MTU: usize = 1000;
const OVERHEAD_SESSION: usize = 80;

#[derive(Debug, PartialEq)]
pub enum FetchError {
    NoRoute,
    NotFound,
    Timeout,
}
impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            FetchError::NoRoute => "no route to that node",
            FetchError::NotFound => "not found",
            FetchError::Timeout => "timed out",
        })
    }
}

struct Reasm {
    from: Addr,
    total: u32,
    chunks: Vec<Option<Vec<u8>>>,
    got: u32,
    not_found: bool,
    progress: Instant,
}

pub struct Node {
    pub id: Identity,
    transports: Vec<Arc<dyn Transport>>,
    routes: Mutex<Routes>,
    names: Mutex<Names>,
    my_names: Mutex<Vec<NameRecord>>,
    site: Mutex<Option<PathBuf>>,
    incoming: Mutex<HashMap<u32, Reasm>>,
    served: Mutex<HashMap<(Addr, u32), (Instant, Vec<Vec<u8>>)>>,
    sess_tx: Mutex<HashMap<Addr, u64>>,
    /// Per peer: highest counter seen and a bitmap of the 64 before it (replay protection).
    sess_rx: Mutex<HashMap<Addr, (u64, u64)>>,
    radio_pace_ms: AtomicU64,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Node {
    pub fn new(id: Identity, transports: Vec<Arc<dyn Transport>>) -> Arc<Self> {
        Arc::new(Node {
            id,
            transports,
            routes: Default::default(),
            names: Default::default(),
            my_names: Default::default(),
            site: Default::default(),
            incoming: Default::default(),
            served: Default::default(),
            sess_tx: Default::default(),
            sess_rx: Default::default(),
            radio_pace_ms: AtomicU64::new(300),
        })
    }

    /// Start receiving on every transport and announce ourselves periodically.
    pub fn start(self: &Arc<Self>) {
        for (i, t) in self.transports.iter().enumerate() {
            let (node, t) = (self.clone(), t.clone());
            tokio::spawn(async move {
                while let Ok((peer, data)) = t.recv().await {
                    let node = node.clone();
                    tokio::spawn(async move { node.handle(Link { transport: i, peer }, data).await });
                }
            });
        }
        let node = self.clone();
        tokio::spawn(async move {
            loop {
                node.announce().await;
                node.routes.lock().unwrap().expire();
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
    }

    /// Share a folder and register `name.skein` for it.
    pub async fn publish(self: &Arc<Self>, dir: PathBuf, name: &str) -> Result<(), String> {
        if !valid_name(name) {
            return Err("name must be 1-63 characters: a-z, 0-9, '-'".into());
        }
        *self.site.lock().unwrap() = Some(dir);
        let rec = NameRecord::new(name, self.id.vk(), now_secs(), |m| self.id.sign(m));
        if !self.names.lock().unwrap().insert(rec.clone()) {
            return Err(format!("{name}.skein is already owned by another key"));
        }
        self.my_names.lock().unwrap().push(rec.clone());
        self.flood(&self.packet(KIND_NAME, [0; 16], enc(&rec)), None).await;
        Ok(())
    }

    pub fn resolve(&self, name: &str) -> Option<Addr> {
        self.names.lock().unwrap().resolve(name)
    }
    pub fn known_names(&self) -> Vec<(String, Addr)> {
        let mut v: Vec<_> = self.names.lock().unwrap().0.values().map(|r| (r.name.clone(), r.addr())).collect();
        v.sort();
        v
    }
    /// Snapshot for the control panel.
    pub fn status(&self) -> serde_json::Value {
        let routes: Vec<_> = self.routes.lock().unwrap().0.iter().map(|(a, r)| serde_json::json!({
            "addr": fmt_addr(a), "hops": r.hops, "via": format!("{} {}", self.transports[r.link.transport].name(), r.link.peer), "age": r.heard.elapsed().as_secs(),
        })).collect();
        let links: Vec<_> = self.transports.iter().map(|t| serde_json::json!({"name": t.name(), "mtu": t.mtu(), "peers": t.peers().len()})).collect();
        let mine: Vec<_> = self.my_names.lock().unwrap().iter().map(|r| r.name.clone()).collect();
        let names: Vec<_> = self.known_names().into_iter().map(|(n, a)| serde_json::json!({"name": n, "addr": fmt_addr(&a), "mine": mine.contains(&n)})).collect();
        serde_json::json!({"addr": fmt_addr(&self.id.addr()), "links": links, "routes": routes, "names": names})
    }
    pub fn route_count(&self) -> usize {
        self.routes.lock().unwrap().0.len()
    }

    // ── Control plane ────────────────────────────────────────────────────────
    fn packet(&self, kind: u8, dst: Addr, body: Vec<u8>) -> Packet {
        Packet { kind, ttl: DEFAULT_TTL, id: OsRng.next_u32(), src: self.id.addr(), dst, body }
    }

    fn own_announce(&self) -> Announce {
        let (vk, xpub, seq) = (self.id.vk(), self.id.xpub(), now_secs());
        Announce { vk, xpub, seq, hops: 0, sig: self.id.sign(&Announce::signed_bytes(&vk, &xpub, seq)) }
    }

    async fn announce(&self) {
        self.flood(&self.packet(KIND_ANNOUNCE, [0; 16], enc(&self.own_announce())), None).await;
        let mine = self.my_names.lock().unwrap().clone();
        for r in mine {
            self.flood(&self.packet(KIND_NAME, [0; 16], enc(&r)), None).await;
        }
    }

    async fn raw_send(&self, link: &Link, p: &Packet) {
        let _ = self.transports[link.transport].send(&link.peer, &enc(p)).await;
    }

    /// Send to every neighbour on every transport, except `except`.
    async fn flood(&self, p: &Packet, except: Option<&Link>) {
        for (i, t) in self.transports.iter().enumerate() {
            for peer in t.flood_peers() {
                let link = Link { transport: i, peer };
                if Some(&link) != except {
                    self.raw_send(&link, p).await;
                }
            }
        }
    }

    /// New neighbour: give it everything we know (our announcement, all routes, all names).
    async fn hello(&self, link: &Link) {
        self.raw_send(link, &self.packet(KIND_ANNOUNCE, [0; 16], enc(&self.own_announce()))).await;
        let routes: Vec<_> = self.routes.lock().unwrap().0.values().cloned().collect();
        for r in routes {
            let a = Announce { vk: r.vk, xpub: r.xpub, seq: r.seq, hops: r.hops, sig: r.sig };
            self.raw_send(link, &Packet { kind: KIND_ANNOUNCE, ttl: DEFAULT_TTL, id: 0, src: addr_of(&r.vk), dst: [0; 16], body: enc(&a) }).await;
        }
        let names: Vec<_> = self.names.lock().unwrap().0.values().cloned().collect();
        for n in names {
            self.raw_send(link, &self.packet(KIND_NAME, [0; 16], enc(&n))).await;
        }
    }

    async fn handle(self: Arc<Self>, link: Link, bytes: Vec<u8>) {
        let Some(p) = dec::<Packet>(&bytes) else { return };
        match p.kind {
            KIND_ANNOUNCE => {
                let Some(mut a) = dec::<Announce>(&p.body) else { return };
                let (is_new, changed) = {
                    let mut r = self.routes.lock().unwrap();
                    let is_new = !r.0.contains_key(&addr_of(&a.vk));
                    (is_new, r.update(&a, &link, &self.id.addr()).is_some())
                };
                if changed && p.ttl > 1 {
                    a.hops = a.hops.saturating_add(1);
                    let fwd = Packet { ttl: p.ttl - 1, body: enc(&a), ..p };
                    self.flood(&fwd, Some(&link)).await;
                    if is_new {
                        self.hello(&link).await;
                    }
                }
            }
            KIND_NAME => {
                let Some(r) = dec::<NameRecord>(&p.body) else { return };
                if p.ttl > 1 && self.names.lock().unwrap().insert(r) {
                    self.flood(&Packet { ttl: p.ttl - 1, ..p }, Some(&link)).await;
                }
            }
            KIND_DATA if p.dst == self.id.addr() => self.receive_data(p).await,
            KIND_SESSION if p.dst == self.id.addr() => self.receive_session(p).await,
            KIND_DATA | KIND_SESSION if p.ttl > 1 => {
                let next = self.routes.lock().unwrap().0.get(&p.dst).map(|r| r.link.clone());
                if let Some(next) = next {
                    self.raw_send(&next, &Packet { ttl: p.ttl - 1, ..p }).await;
                }
            }
            _ => {}
        }
    }

    // ── Data plane: end-to-end encrypted and signed ──────────────────────────
    async fn send_msg(&self, dst: Addr, msg: &Msg) -> Result<usize, FetchError> {
        let (link, xpub, vk) = {
            let r = self.routes.lock().unwrap();
            let route = r.0.get(&dst).ok_or(FetchError::NoRoute)?;
            (route.link.clone(), route.xpub, route.vk)
        };
        let mtu = self.transports[link.transport].mtu();
        if mtu < RADIO_MTU {
            // Radio link: every byte counts. No signature or ephemeral key per packet; an AEAD key derived from both
            // static keys authenticates the pair, and a strictly increasing counter (seeded from the clock) stops replays.
            let src = self.id.addr();
            let ctr = {
                let mut t = self.sess_tx.lock().unwrap();
                let c = t.entry(dst).or_insert_with(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0));
                *c += 1;
                *c
            };
            let key = session_key(self.id.xsecret(), &xpub, &self.id.vk(), &vk);
            let mut body = ctr.to_be_bytes().to_vec();
            body.extend(session_seal(&key, (src > dst) as u8, ctr, &aad(&src, &dst, 0), &enc(msg)));
            self.raw_send(&link, &Packet { kind: KIND_SESSION, ttl: DEFAULT_TTL, id: 0, src, dst, body }).await;
            return Ok(mtu);
        }
        let (src, id) = (self.id.addr(), OsRng.next_u32());
        let aad = aad(&src, &dst, id);
        let msg = enc(msg);
        let inner = Inner { from_vk: self.id.vk(), sig: self.id.sign(&[&aad[..], &msg].concat()), msg };
        let body = seal(&xpub, &aad, &enc(&inner));
        self.raw_send(&link, &Packet { kind: KIND_DATA, ttl: DEFAULT_TTL, id, src, dst, body }).await;
        Ok(self.transports[link.transport].mtu())
    }

    async fn receive_data(self: Arc<Self>, p: Packet) {
        let aad = aad(&p.src, &p.dst, p.id);
        let Some(inner) = open(self.id.xsecret(), &aad, &p.body).and_then(|b| dec::<Inner>(&b)) else { return };
        if addr_of(&inner.from_vk) != p.src || !verify(&inner.from_vk, &[&aad[..], &inner.msg].concat(), &inner.sig) {
            return;
        }
        let Some(msg) = dec::<Msg>(&inner.msg) else { return };
        self.dispatch(p.src, msg).await;
    }

    async fn receive_session(self: Arc<Self>, p: Packet) {
        if p.body.len() < 8 + 16 {
            return;
        }
        let ctr = u64::from_be_bytes(p.body[..8].try_into().unwrap());
        let (xpub, vk) = match self.routes.lock().unwrap().0.get(&p.src) {
            Some(r) => (r.xpub, r.vk),
            None => return, // we have not heard its announcement yet
        };
        let key = session_key(self.id.xsecret(), &xpub, &self.id.vk(), &vk);
        let Some(plain) = session_open(&key, (p.src > p.dst) as u8, ctr, &aad(&p.src, &p.dst, 0), &p.body[8..]) else { return };
        if !self.fresh_counter(p.src, ctr) {
            return;
        }
        let Some(msg) = dec::<Msg>(&plain) else { return };
        self.dispatch(p.src, msg).await;
    }

    /// Sliding-window replay check: true the first time a counter is seen (and not older than 64 behind the newest).
    fn fresh_counter(&self, src: Addr, ctr: u64) -> bool {
        let mut w = self.sess_rx.lock().unwrap();
        let e = w.entry(src).or_insert((0, 0));
        if ctr > e.0 {
            let shift = ctr - e.0;
            e.1 = if shift >= 64 { 0 } else { e.1 << shift };
            e.1 |= 1;
            e.0 = ctr;
            true
        } else {
            let off = e.0 - ctr;
            if off >= 64 || (e.1 >> off) & 1 == 1 {
                return false;
            }
            e.1 |= 1 << off;
            true
        }
    }

    async fn pace(&self, mtu: usize) {
        let ms = self.radio_pace_ms.load(Ordering::Relaxed);
        if mtu < RADIO_MTU && ms > 0 {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }

    /// Delay between packets on radio links (default 300 ms; the radio's own duty-cycle limits still apply).
    pub fn set_radio_pace(&self, d: Duration) {
        self.radio_pace_ms.store(d.as_millis() as u64, Ordering::Relaxed);
    }

    async fn dispatch(self: Arc<Self>, src: Addr, msg: Msg) {
        match msg {
            Msg::Get { req, path } => {
                tokio::spawn(async move { self.serve(src, req, path).await });
            }
            Msg::Resend { req, missing } => {
                let chunks = self.served.lock().unwrap().get(&(src, req)).map(|(_, c)| c.clone());
                if let Some(chunks) = chunks {
                    for idx in missing.into_iter().filter(|i| (*i as usize) < chunks.len()).take(200) {
                        let m = Msg::Chunk { req, idx, total: chunks.len() as u32, data: chunks[idx as usize].clone() };
                        if let Ok(mtu) = self.send_msg(src, &m).await {
                            self.pace(mtu).await;
                        }
                    }
                }
            }
            Msg::NotFound { req } => {
                if let Some(r) = self.incoming.lock().unwrap().get_mut(&req).filter(|r| r.from == src) {
                    r.not_found = true;
                }
            }
            Msg::Chunk { req, idx, total, data } => {
                if let Some(r) = self.incoming.lock().unwrap().get_mut(&req).filter(|r| r.from == src) {
                    if r.chunks.is_empty() && total > 0 && total <= 1 << 22 {
                        r.total = total;
                        r.chunks = vec![None; total as usize];
                    }
                    if total == r.total && (idx as usize) < r.chunks.len() && r.chunks[idx as usize].is_none() {
                        r.chunks[idx as usize] = Some(data);
                        r.got += 1;
                        r.progress = Instant::now();
                    }
                }
            }
        }
    }

    /// Resolve a request path inside the shared folder (no escaping with `..`).
    fn file_path(&self, path: &str) -> Option<PathBuf> {
        let root = self.site.lock().unwrap().clone()?;
        let mut rel = PathBuf::new();
        for c in Path::new(path.trim_start_matches('/')).components() {
            match c {
                Component::Normal(s) => rel.push(s),
                _ => return None,
            }
        }
        if rel.as_os_str().is_empty() || path.ends_with('/') {
            rel.push("index.html");
        }
        Some(root.join(rel))
    }

    async fn serve(&self, to: Addr, req: u32, path: String) {
        let data = match self.file_path(&path) {
            Some(f) => match tokio::fs::metadata(&f).await {
                Ok(m) if m.is_file() && m.len() <= MAX_FILE => tokio::fs::read(&f).await.ok(),
                _ => None,
            },
            None => None,
        };
        let Some(data) = data else {
            let _ = self.send_msg(to, &Msg::NotFound { req }).await;
            return;
        };
        let Ok(mtu) = self.mtu_to(&to) else { return };
        let size = mtu.saturating_sub(if mtu < RADIO_MTU { OVERHEAD_SESSION } else { OVERHEAD }).max(16);
        let mut chunks: Vec<Vec<u8>> = data.chunks(size).map(<[u8]>::to_vec).collect();
        if chunks.is_empty() {
            chunks.push(vec![]);
        }
        let total = chunks.len() as u32;
        {
            let mut s = self.served.lock().unwrap();
            s.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(120));
            s.insert((to, req), (Instant::now(), chunks.clone()));
        }
        for (i, c) in chunks.into_iter().enumerate() {
            if self.send_msg(to, &Msg::Chunk { req, idx: i as u32, total, data: c }).await.is_err() {
                return;
            }
            self.pace(mtu).await;
            if i % 32 == 31 {
                tokio::time::sleep(Duration::from_millis(2)).await; // gentle pacing; lost chunks are re-requested
            }
        }
    }

    fn mtu_to(&self, dst: &Addr) -> Result<usize, FetchError> {
        let r = self.routes.lock().unwrap();
        let route = r.0.get(dst).ok_or(FetchError::NoRoute)?;
        Ok(self.transports[route.link.transport].mtu())
    }

    /// Download `path` from the node at `dst`. Missing chunks are re-requested until the file is complete.
    pub async fn fetch(self: &Arc<Self>, dst: Addr, path: &str) -> Result<Vec<u8>, FetchError> {
        let req = OsRng.next_u32();
        self.incoming.lock().unwrap().insert(req, Reasm { from: dst, total: 0, chunks: vec![], got: 0, not_found: false, progress: Instant::now() });
        let result = self.fetch_inner(dst, req, path).await;
        self.incoming.lock().unwrap().remove(&req);
        result
    }

    async fn fetch_inner(&self, dst: Addr, req: u32, path: &str) -> Result<Vec<u8>, FetchError> {
        self.send_msg(dst, &Msg::Get { req, path: path.into() }).await?;
        let deadline = Instant::now() + Duration::from_secs(120);
        // A Resend message must fit in one packet: about 3 bytes per missing index.
        let limit = self.mtu_to(&dst).map(|m| if m < RADIO_MTU { (m.saturating_sub(100) / 3).clamp(4, 150) } else { 150 }).unwrap_or(150);
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let missing = {
                let mut inc = self.incoming.lock().unwrap();
                let r = inc.get_mut(&req).ok_or(FetchError::Timeout)?;
                if r.not_found {
                    return Err(FetchError::NotFound);
                }
                if r.total > 0 && r.got == r.total {
                    return Ok(r.chunks.iter().flatten().flatten().copied().collect());
                }
                if r.progress.elapsed() < Duration::from_millis(1500) {
                    continue;
                }
                r.progress = Instant::now();
                (0..r.total).filter(|i| r.chunks[*i as usize].is_none()).take(limit).collect::<Vec<_>>()
            };
            if Instant::now() > deadline {
                return Err(FetchError::Timeout);
            }
            let again = if missing.is_empty() { Msg::Get { req, path: path.into() } } else { Msg::Resend { req, missing } };
            self.send_msg(dst, &again).await?;
        }
    }
}
