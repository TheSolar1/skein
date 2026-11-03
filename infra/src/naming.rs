//! Names: `something.skein`, first come first served, owned by a key. Records are signed and gossiped; no central registry.
use crate::identity::{addr_of, verify, Addr};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NameRecord {
    pub name: String,
    pub owner: [u8; 32],
    pub seq: u64,
    pub sig: Vec<u8>,
}

/// 1 to 63 characters: a-z, 0-9 and '-', not starting or ending with '-'. (The ".skein" suffix is implied.)
pub fn valid_name(n: &str) -> bool {
    (1..=63).contains(&n.len())
        && n.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !n.starts_with('-')
        && !n.ends_with('-')
}

impl NameRecord {
    fn signed_bytes(name: &str, owner: &[u8; 32], seq: u64) -> Vec<u8> {
        [&b"skein name v1"[..], name.as_bytes(), &[0], owner, &seq.to_be_bytes()].concat()
    }
    pub fn new(name: &str, owner: [u8; 32], seq: u64, sign: impl Fn(&[u8]) -> Vec<u8>) -> Self {
        NameRecord { name: name.into(), owner, seq, sig: sign(&Self::signed_bytes(name, &owner, seq)) }
    }
    pub fn valid(&self) -> bool {
        valid_name(&self.name) && verify(&self.owner, &Self::signed_bytes(&self.name, &self.owner, self.seq), &self.sig)
    }
    /// The node this name points to: the one whose key owns it.
    pub fn addr(&self) -> Addr {
        addr_of(&self.owner)
    }
}

#[derive(Default)]
pub struct Names(pub HashMap<String, NameRecord>);

impl Names {
    /// Accept a record if it is valid and either new, or a newer version from the same owner. True if the store changed.
    pub fn insert(&mut self, r: NameRecord) -> bool {
        if !r.valid() {
            return false;
        }
        match self.0.get(&r.name) {
            Some(cur) if cur.owner != r.owner || cur.seq >= r.seq => false,
            _ => {
                self.0.insert(r.name.clone(), r);
                true
            }
        }
    }
    pub fn resolve(&self, name: &str) -> Option<Addr> {
        self.0.get(name).map(NameRecord::addr)
    }
}
