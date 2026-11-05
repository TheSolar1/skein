//! Wire format. The routing header is clear (relays need it); everything else is signed and/or encrypted.
use crate::identity::Addr;
use bincode::Options;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub const KIND_ANNOUNCE: u8 = 1;
pub const KIND_NAME: u8 = 2;
pub const KIND_DATA: u8 = 3;
/// Compact data packet for radio links: 8-byte counter + AEAD under a session key (no signature, no ephemeral key).
pub const KIND_SESSION: u8 = 4;
pub const DEFAULT_TTL: u8 = 16;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Packet {
    pub kind: u8,
    pub ttl: u8,
    pub id: u32,
    pub src: Addr,
    pub dst: Addr,
    pub body: Vec<u8>,
}

/// Signed by the node: "this key lives at this address, sequence N". `hops` is not signed (relays increment it).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Announce {
    pub vk: [u8; 32],
    pub xpub: [u8; 32],
    pub seq: u64,
    pub hops: u8,
    pub sig: Vec<u8>,
}

impl Announce {
    pub fn signed_bytes(vk: &[u8; 32], xpub: &[u8; 32], seq: u64) -> Vec<u8> {
        [&b"skein announce v1"[..], vk, xpub, &seq.to_be_bytes()].concat()
    }
}

/// Inside the sealed box: who sent it (key + signature over header and message) and the application message.
#[derive(Serialize, Deserialize)]
pub struct Inner {
    pub from_vk: [u8; 32],
    pub sig: Vec<u8>,
    pub msg: Vec<u8>,
}

/// Application messages (a tiny request/response protocol with chunked, resumable replies).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Msg {
    Get { req: u32, path: String },
    Chunk { req: u32, idx: u32, total: u32, data: Vec<u8> },
    Resend { req: u32, missing: Vec<u32> },
    NotFound { req: u32 },
}

fn opts() -> impl Options {
    bincode::DefaultOptions::new().with_varint_encoding().with_limit(1 << 20)
}
pub fn enc<T: Serialize>(v: &T) -> Vec<u8> {
    opts().serialize(v).expect("serialize")
}
pub fn dec<T: DeserializeOwned>(b: &[u8]) -> Option<T> {
    opts().deserialize(b).ok()
}

/// What the sealed box authenticates besides the content: the routing header.
pub fn aad(src: &Addr, dst: &Addr, id: u32) -> Vec<u8> {
    [&src[..], &dst[..], &id.to_be_bytes()].concat()
}
