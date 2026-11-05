//! A transport moves opaque datagrams between neighbours. UDP is implemented; radio adapters plug in the same way.
use async_trait::async_trait;
use std::{io, net::SocketAddr, sync::Mutex};
use tokio::net::UdpSocket;

#[async_trait]
pub trait Transport: Send + Sync {
    fn name(&self) -> &str;
    /// Largest datagram this link carries (UDP ~1200 bytes; LoRa/Meshtastic ~230).
    fn mtu(&self) -> usize;
    async fn send(&self, peer: &str, data: &[u8]) -> io::Result<()>;
    async fn recv(&self) -> io::Result<(String, Vec<u8>)>;
    /// Neighbours we know about.
    fn peers(&self) -> Vec<String>;
    /// Where a flooded packet is sent. A broadcast medium (radio) sends once to everyone instead of once per neighbour.
    fn flood_peers(&self) -> Vec<String> {
        self.peers()
    }
}

pub struct Udp {
    sock: UdpSocket,
    peers: Mutex<Vec<String>>,
    /// Random id put in our beacons so we ignore our own broadcast.
    beacon_id: [u8; 8],
}

/// Local-network discovery needs no internet and no setup: every few seconds a node broadcasts `SKEINHI` + id + its port,
/// and whoever hears it adds the sender as a peer.
const BEACON: &[u8; 7] = b"SKEINHI";
pub const DISCOVERY_PORT: u16 = 7777;

impl Udp {
    pub async fn bind(listen: SocketAddr, peers: Vec<String>) -> io::Result<Self> {
        let mut beacon_id = [0u8; 8];
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut beacon_id);
        Ok(Udp { sock: UdpSocket::bind(listen).await?, peers: Mutex::new(peers), beacon_id })
    }
    /// Start announcing ourselves to the local network (Wi-Fi, Ethernet, a hotspot) with no internet needed.
    pub fn discover(self: &std::sync::Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            let _ = me.sock.set_broadcast(true);
            let mut msg = BEACON.to_vec();
            msg.extend_from_slice(&me.beacon_id);
            msg.extend_from_slice(&me.local_addr().port().to_be_bytes());
            loop {
                let _ = me.sock.send_to(&msg, ("255.255.255.255", DISCOVERY_PORT)).await;
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.sock.local_addr().expect("bound socket")
    }
}

#[async_trait]
impl Transport for Udp {
    fn name(&self) -> &str {
        "udp"
    }
    fn mtu(&self) -> usize {
        1200
    }
    async fn send(&self, peer: &str, data: &[u8]) -> io::Result<()> {
        let addr = tokio::net::lookup_host(peer).await?.next().ok_or_else(|| io::Error::other("unresolvable peer"))?;
        self.sock.send_to(data, addr).await.map(|_| ())
    }
    async fn recv(&self) -> io::Result<(String, Vec<u8>)> {
        let mut buf = vec![0u8; 2048];
        let (n, from) = loop {
            let (n, from) = self.sock.recv_from(&mut buf).await?;
            if n == BEACON.len() + 10 && buf.starts_with(BEACON) {
                if buf[7..15] != self.beacon_id {
                    let port = u16::from_be_bytes([buf[15], buf[16]]);
                    let peer = SocketAddr::new(from.ip(), port).to_string();
                    let mut p = self.peers.lock().unwrap();
                    if !p.contains(&peer) {
                        p.push(peer);
                    }
                }
                continue;
            }
            break (n, from);
        };
        buf.truncate(n);
        let from = from.to_string();
        // Learn peers that talk to us, so nodes behind NAT or without a configured peer list still get announcements.
        let mut p = self.peers.lock().unwrap();
        if !p.contains(&from) {
            p.push(from.clone());
        }
        Ok((from, buf))
    }
    fn peers(&self) -> Vec<String> {
        self.peers.lock().unwrap().clone()
    }
}
