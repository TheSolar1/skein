//! Distance-vector routing learned from signed announcements, with sequence numbers (loop-free, like DSDV).
use crate::identity::{addr_of, verify, Addr};
use crate::packet::Announce;
use std::collections::HashMap;

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
            self.0.insert(addr, Route { link: link.clone(), hops, seq: a.seq, vk: a.vk, xpub: a.xpub, sig: a.sig.clone() });
        }
        better.then_some(addr)
    }
}
