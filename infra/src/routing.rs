//! Distance-vector routing learned from signed announcements, with sequence numbers (loop-free, like DSDV).
use crate::identity::{addr_of, verify, Addr};
use crate::packet::Announce;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A route not refreshed by an announcement for this long is dropped (announcements are sent every 30 s).
pub const ROUTE_TTL: Duration = Duration::from_secs(150);

/// A way to reach a neighbour: which transport, and the peer's address on it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    pub transport: usize,
    pub peer: String,
}

#[derive(Clone, Debug)]
pub struct Route {
    pub link: Link,
    pub hops: u8,
    pub seq: u64,
    pub vk: [u8; 32],
    pub xpub: [u8; 32],
    pub sig: Vec<u8>,
    pub heard: Instant,
}

#[derive(Default)]
pub struct Routes(pub HashMap<Addr, Route>);

impl Routes {
    /// Handle an announcement heard on `link`. Returns the announced address if the table changed (so it must be forwarded).
    pub fn update(&mut self, a: &Announce, link: &Link, me: &Addr) -> Option<Addr> {
        let addr = addr_of(&a.vk);
        if addr == *me || !verify(&a.vk, &Announce::signed_bytes(&a.vk, &a.xpub, a.seq), &a.sig) {
            return None;
        }
        let hops = a.hops.saturating_add(1);
        let better = match self.0.get(&addr) {
            None => true,
            Some(r) => a.seq > r.seq || (a.seq == r.seq && hops < r.hops),
        };
        if better {
            self.0.insert(addr, Route { link: link.clone(), hops, seq: a.seq, vk: a.vk, xpub: a.xpub, sig: a.sig.clone(), heard: Instant::now() });
        }
        if !better {
            // Same announcement arriving again over the current link: the route is still alive.
            if let Some(r) = self.0.get_mut(&addr) {
                if r.seq == a.seq && r.link == *link {
                    r.heard = Instant::now();
                }
            }
        }
        better.then_some(addr)
    }

    /// Drop routes that have gone silent. Returns how many were removed.
    pub fn expire(&mut self) -> usize {
        let before = self.0.len();
        self.0.retain(|_, r| r.heard.elapsed() < ROUTE_TTL);
        before - self.0.len()
    }
}
