//! Two Skein nodes talk through a fake Meshtastic radio network (a hub that speaks the stream API over TCP):
//! checks the protobuf framing, the 233-byte payload limit, compact session packets and bulk transfer over radio.
use skein::{identity::*, meshtastic::*, node::Node, transport::Transport};
use std::{sync::{atomic::{AtomicUsize, Ordering}, Arc}, time::Duration};
use tokio::{io::AsyncWriteExt, net::TcpListener, sync::broadcast};

/// Start `n` fake radios that relay every packet to the others; returns their addresses and the largest payload seen.
async fn fake_radios(n: usize) -> (Vec<String>, Arc<AtomicUsize>) {
    let (bus, _) = broadcast::channel::<(usize, u32, u32, Vec<u8>)>(4096);
    let (addrs, biggest) = (std::cell::RefCell::new(vec![]), Arc::new(AtomicUsize::new(0)));
    for i in 0..n {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        addrs.borrow_mut().push(l.local_addr().unwrap().to_string());
        let (bus, biggest) = (bus.clone(), biggest.clone());
        tokio::spawn(async move {
            let (s, _) = l.accept().await.unwrap();
            let (mut rd, mut wr) = s.into_split();
            let mut sub = bus.subscribe();
            let me = 0x1000 + i as u32;
            tokio::spawn(async move {
                while let Ok((from, to, id, payload)) = sub.recv().await {
                    if from != i && (to == BROADCAST || to == me) {
                        let f = frame(&from_radio_packet(0x1000 + from as u32, to, id, &payload));
                        if wr.write_all(&f).await.is_err() {
                            return;
                        }
                    }
                }
            });
            while let Ok(buf) = read_frame(&mut rd).await {
                if let Some((to, id, payload)) = decode_to_radio_packet(&buf) {
                    biggest.fetch_max(payload.len(), Ordering::Relaxed);
                    let _ = bus.send((i, to, id, payload));
                }
            }
        });
    }
    (addrs.into_inner(), biggest)
}

#[test]
fn protobuf_roundtrip() {
    let payload = vec![7u8; 233];
    let from_radio = from_radio_packet(0xdead_beef, 0x1234, 9, &payload);
    assert_eq!(decode_from_radio(&from_radio), FromRadio::Packet { from: 0xdead_beef, to: 0x1234, payload: payload.clone() });
    assert_eq!(decode_to_radio_packet(&to_radio_packet(BROADCAST, 5, &payload)), Some((BROADCAST, 5, payload)));
    assert_eq!(decode_from_radio(&[0x08, 0x01]), FromRadio::Other);
}

#[tokio::test(flavor = "multi_thread")]
async fn bulk_transfer_over_meshtastic() {
    let dir = std::env::temp_dir().join(format!("skein-radio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data: Vec<u8> = (0..30_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    std::fs::write(dir.join("page.bin"), &data).unwrap();

    let (addrs, biggest) = fake_radios(2).await;
    let mk = |addr: String| async move {
        let radio = Meshtastic::connect(&addr).await.unwrap();
        let n = Node::new(Identity::generate(), vec![Arc::new(radio) as Arc<dyn Transport>]);
        n.set_radio_pace(Duration::ZERO);
        n.start();
        n
    };
    let (a, b) = (mk(addrs[0].clone()).await, mk(addrs[1].clone()).await);
    b.publish(dir.clone(), "radio").await.unwrap();
    let baddr = b.id.addr();
    for _ in 0..100 {
        if a.resolve("radio") == Some(baddr) && a.route_count() >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(a.resolve("radio"), Some(baddr), "name did not arrive over the radio");
    assert_eq!(a.fetch(baddr, "/page.bin").await.unwrap(), data);
    let max = biggest.load(Ordering::Relaxed);
    assert!(max <= MAX_PAYLOAD, "a packet of {max} bytes does not fit a Meshtastic payload");
    println!("largest radio payload: {max} bytes");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn silent_routes_expire() {
    use skein::{identity::Identity, packet::Announce, routing::{Link, Routes, ROUTE_TTL}};
    let (me, other) = (Identity::generate(), Identity::generate());
    let (vk, xpub, seq) = (other.vk(), other.xpub(), 1);
    let a = Announce { vk, xpub, seq, hops: 0, sig: other.sign(&Announce::signed_bytes(&vk, &xpub, seq)) };
    let mut r = Routes::default();
    r.update(&a, &Link { transport: 0, peer: "x".into() }, &me.addr());
    assert_eq!(r.expire(), 0);
    for v in r.0.values_mut() {
        v.heard -= ROUTE_TTL + std::time::Duration::from_secs(1);
    }
    assert_eq!(r.expire(), 1);
    assert!(r.0.is_empty());
}

#[tokio::test]
async fn beacon_adds_a_peer_without_internet() {
    use skein::transport::{Transport, Udp};
    use std::sync::Arc;
    let a = Arc::new(Udp::bind("127.0.0.1:0".parse().unwrap(), vec![]).await.unwrap());
    let b = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut beacon = b"SKEINHI".to_vec();
    beacon.extend_from_slice(&[9; 8]);
    beacon.extend_from_slice(&4242u16.to_be_bytes());
    b.send_to(&beacon, a.local_addr()).await.unwrap();
    b.send_to(b"real packet", a.local_addr()).await.unwrap();
    let (_, data) = a.recv().await.unwrap(); // the beacon is swallowed, the real packet comes through
    assert_eq!(data, b"real packet");
    assert!(a.peers().contains(&"127.0.0.1:4242".to_string()));
}
