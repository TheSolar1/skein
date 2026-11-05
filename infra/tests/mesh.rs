use skein::{identity::*, naming::*, node::{FetchError, Node}, transport::{Transport, Udp}};
use std::{sync::Arc, time::Duration};

async fn node(peers: Vec<String>) -> (Arc<Node>, String) {
    let udp = Udp::bind("127.0.0.1:0".parse().unwrap(), peers).await.unwrap();
    let port = udp.local_addr().to_string();
    let n = Node::new(Identity::generate(), vec![Arc::new(udp) as Arc<dyn Transport>]);
    n.start();
    (n, port)
}

async fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..100 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn addresses_roundtrip_and_are_bound_to_keys() {
    let id = Identity::generate();
    let s = fmt_addr(&id.addr());
    assert!(s.starts_with("5c1e:"));
    assert_eq!(parse_addr(&s), Some(id.addr()));
    assert_eq!(parse_addr("fe80:0000:0000:0000:0000:0000:0000:0001"), None);
}

#[test]
fn names_first_come_first_served_and_signed() {
    let (a, b) = (Identity::generate(), Identity::generate());
    let mut names = Names::default();
    let rec = |id: &Identity, n: &str, seq| NameRecord::new(n, id.vk(), seq, |m| id.sign(m));
    assert!(names.insert(rec(&a, "hello", 1)));
    assert!(!names.insert(rec(&b, "hello", 99)), "another key must not take a name");
    assert!(names.insert(rec(&a, "hello", 2)), "the owner can update");
    assert!(!names.insert(rec(&a, "hello", 2)), "same sequence is not newer");
    let mut forged = rec(&b, "mine", 1);
    forged.owner = a.vk();
    assert!(!names.insert(forged), "forged signature");
    assert!(!valid_name("Bad_Name") && !valid_name("-x") && valid_name("my-site-2"));
    assert_eq!(names.resolve("hello"), Some(a.addr()));
}

#[test]
fn sealed_boxes_only_open_for_the_recipient_and_the_header() {
    let (me, other) = (Identity::generate(), Identity::generate());
    let ct = skein::crypto::seal(&me.xpub(), b"hdr", b"secret");
    assert_eq!(skein::crypto::open(me.xsecret(), b"hdr", &ct).as_deref(), Some(&b"secret"[..]));
    assert!(skein::crypto::open(other.xsecret(), b"hdr", &ct).is_none());
    assert!(skein::crypto::open(me.xsecret(), b"other header", &ct).is_none());
}

/// A -- B -- C over real UDP sockets: C publishes a site with a 700 KB file, A downloads it through B.
#[tokio::test(flavor = "multi_thread")]
async fn bulk_transfer_across_a_relay() {
    let dir = std::env::temp_dir().join(format!("skein-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let big: Vec<u8> = (0..700_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    std::fs::write(dir.join("big.bin"), &big).unwrap();
    std::fs::write(dir.join("index.html"), "<h1>hi</h1>").unwrap();

    let (b, bport) = node(vec![]).await;
    let (c, _) = node(vec![bport.clone()]).await;
    let (a, _) = node(vec![bport]).await;
    c.publish(dir.clone(), "demo").await.unwrap();
    let caddr = c.id.addr();

    wait("A to learn C's route and name", || a.route_count() >= 2 && a.resolve("demo") == Some(caddr)).await;
    assert!(b.resolve("demo").is_some());

    assert_eq!(a.fetch(caddr, "/").await.unwrap(), b"<h1>hi</h1>");
    assert_eq!(a.fetch(caddr, "/big.bin").await.unwrap(), big);
    assert_eq!(a.fetch(caddr, "/missing").await, Err(FetchError::NotFound));
    assert_eq!(a.fetch(caddr, "/../secret").await, Err(FetchError::NotFound), "no escaping the shared folder");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn session_keys_match_on_both_sides_and_reject_tampering() {
    use skein::crypto::{session_key, session_open, session_seal};
    let (a, b, c) = (Identity::generate(), Identity::generate(), Identity::generate());
    let ka = session_key(a.xsecret(), &b.xpub(), &a.vk(), &b.vk());
    let kb = session_key(b.xsecret(), &a.xpub(), &b.vk(), &a.vk());
    assert_eq!(ka, kb);
    assert_ne!(ka, session_key(c.xsecret(), &a.xpub(), &c.vk(), &a.vk()), "a third node must not share the key");
    let ct = session_seal(&ka, 0, 7, b"hdr", b"payload");
    assert_eq!(session_open(&kb, 0, 7, b"hdr", &ct).as_deref(), Some(&b"payload"[..]));
    assert!(session_open(&kb, 1, 7, b"hdr", &ct).is_none(), "wrong direction");
    assert!(session_open(&kb, 0, 8, b"hdr", &ct).is_none(), "wrong counter");
    assert!(session_open(&kb, 0, 7, b"other", &ct).is_none(), "wrong header");
}
