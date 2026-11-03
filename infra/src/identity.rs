//! Node identity: one Ed25519 key (signatures, address) and one X25519 key (encryption), both from a 32-byte seed.
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use std::path::Path;
use x25519_dalek::{PublicKey as XPublic, StaticSecret};

/// A Skein address: 16 bytes, always starting with 5c1e, derived from the public key (like an IPv6 address).
pub type Addr = [u8; 16];

pub fn addr_of(vk: &[u8; 32]) -> Addr {
    let h = blake3::hash(vk);
    let mut a = [0u8; 16];
    a[..2].copy_from_slice(&[0x5c, 0x1e]);
    a[2..].copy_from_slice(&h.as_bytes()[..14]);
    a
}

/// `5c1e:xxxx:xxxx:xxxx:xxxx:xxxx:xxxx:xxxx`
pub fn fmt_addr(a: &Addr) -> String {
    a.chunks(2).map(|c| format!("{:02x}{:02x}", c[0], c[1])).collect::<Vec<_>>().join(":")
}

pub fn parse_addr(s: &str) -> Option<Addr> {
    let groups: Vec<&str> = s.split(':').collect();
    if groups.len() != 8 || groups.iter().any(|g| g.len() != 4) {
        return None;
    }
    let mut a = [0u8; 16];
    for (i, g) in groups.iter().enumerate() {
        let v = u16::from_str_radix(g, 16).ok()?;
        a[i * 2..i * 2 + 2].copy_from_slice(&v.to_be_bytes());
    }
    (a[0] == 0x5c && a[1] == 0x1e).then_some(a)
}

pub fn verify(vk: &[u8; 32], msg: &[u8], sig: &[u8]) -> bool {
    let (Ok(vk), Ok(sig)) = (VerifyingKey::from_bytes(vk), Signature::from_slice(sig)) else { return false };
    vk.verify(msg, &sig).is_ok()
}

pub struct Identity {
    sk: SigningKey,
    xs: StaticSecret,
}

impl Identity {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let xs = StaticSecret::from(blake3::derive_key("skein x25519 v1", &seed));
        Identity { sk: SigningKey::from_bytes(&seed), xs }
    }
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        Self::from_seed(seed)
    }
    /// Load the seed from `path`, or create it. The file is the node's private key: keep it secret.
    pub fn load_or_create(path: &Path) -> std::io::Result<Self> {
        if let Ok(b) = std::fs::read(path) {
            if let Ok(seed) = <[u8; 32]>::try_from(b.as_slice()) {
                return Ok(Self::from_seed(seed));
            }
        }
        let id = Self::generate();
        std::fs::write(path, id.sk.to_bytes())?;
        Ok(id)
    }
    pub fn vk(&self) -> [u8; 32] {
        self.sk.verifying_key().to_bytes()
    }
    pub fn xpub(&self) -> [u8; 32] {
        XPublic::from(&self.xs).to_bytes()
    }
    pub fn addr(&self) -> Addr {
        addr_of(&self.vk())
    }
    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.sk.sign(msg).to_bytes().to_vec()
    }
    pub fn xsecret(&self) -> &StaticSecret {
        &self.xs
    }
}
