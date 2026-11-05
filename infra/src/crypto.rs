//! Sealed boxes: anonymous-sender public-key encryption (ephemeral X25519 + ChaCha20-Poly1305).
//! A fresh ephemeral key per message means every message key is unique, so a fixed nonce is safe.
use chacha20poly1305::{aead::{Aead, Payload}, ChaCha20Poly1305, KeyInit};
use rand_core::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};

fn cipher(shared: &[u8; 32], eph: &[u8; 32], rcpt: &[u8; 32]) -> ChaCha20Poly1305 {
    let key = blake3::derive_key("skein seal v1", &[&shared[..], &eph[..], &rcpt[..]].concat());
    ChaCha20Poly1305::new(&key.into())
}

/// Output: ephemeral public key (32 bytes) followed by ciphertext + 16-byte tag. `aad` is authenticated but not encrypted.
pub fn seal(rcpt_pub: &[u8; 32], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let eph = StaticSecret::random_from_rng(OsRng);
    let eph_pub = PublicKey::from(&eph).to_bytes();
    let shared = eph.diffie_hellman(&PublicKey::from(*rcpt_pub));
    let ct = cipher(shared.as_bytes(), &eph_pub, rcpt_pub).encrypt(&[0u8; 12].into(), Payload { msg: plain, aad }).expect("encrypt");
    [&eph_pub[..], &ct].concat()
}

pub fn open(my_secret: &StaticSecret, aad: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 32 + 16 {
        return None;
    }
    let eph_pub: [u8; 32] = data[..32].try_into().ok()?;
    let me = PublicKey::from(my_secret).to_bytes();
    let shared = my_secret.diffie_hellman(&PublicKey::from(eph_pub));
    cipher(shared.as_bytes(), &eph_pub, &me).decrypt(&[0u8; 12].into(), Payload { msg: &data[32..], aad }).ok()
}

/// Session key shared by two nodes, from their static X25519 keys (identical on both sides). Possessing it proves
/// the holder owns one of the two private keys, so radio packets need no per-message signature.
pub fn session_key(my: &StaticSecret, their_xpub: &[u8; 32], my_vk: &[u8; 32], their_vk: &[u8; 32]) -> [u8; 32] {
    let shared = my.diffie_hellman(&PublicKey::from(*their_xpub));
    let (a, b) = if my_vk < their_vk { (my_vk, their_vk) } else { (their_vk, my_vk) };
    blake3::derive_key("skein session v1", &[&shared.as_bytes()[..], &a[..], &b[..]].concat())
}

/// `dir` is 0 or 1 so the two directions never reuse a nonce; `ctr` must never repeat for a given key and direction.
fn nonce(dir: u8, ctr: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[0] = dir;
    n[4..].copy_from_slice(&ctr.to_be_bytes());
    n
}

pub fn session_seal(key: &[u8; 32], dir: u8, ctr: u64, aad: &[u8], plain: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(key.into()).encrypt(&nonce(dir, ctr).into(), Payload { msg: plain, aad }).expect("encrypt")
}

pub fn session_open(key: &[u8; 32], dir: u8, ctr: u64, aad: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(key.into()).decrypt(&nonce(dir, ctr).into(), Payload { msg: ct, aad }).ok()
}
