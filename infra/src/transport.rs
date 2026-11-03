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
    /// Neighbours to flood announcements to.
    fn peers(&self) -> Vec<String>;
}

pub struct Udp {
    sock: UdpSocket,
    peers: Mutex<Vec<String>>,
}

impl Udp {
    pub async fn bind(listen: SocketAddr, peers: Vec<String>) -> io::Result<Self> {
        Ok(Udp { sock: UdpSocket::bind(listen).await?, peers: Mutex::new(peers) })
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
        let (n, from) = self.sock.recv_from(&mut buf).await?;
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
