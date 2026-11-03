# Skein

An encrypted overlay network for places where the usual internet is missing or not trusted, with two parts:

| Folder | What it is |
|---|---|
| [`site/`](site) | **Coverage map** of France: every mobile radio mast (ANFR open data, ~51,000 supports), how far its signal reaches **with terrain**, and the antenna you would need at any point you click. |
| [`infra/`](infra) | **The Skein software** (Rust): own addresses, own routing protocol, `.skein` names usable in a normal browser, end-to-end encryption, and large file transfers. |

## Status

Early skeleton. What works today is tested; what does not exist yet is listed honestly below.

**Works**
- Addresses derived from public keys (`5c1e:…`, 128-bit, like IPv6).
- Signed route announcements, distance-vector routing with sequence numbers, multi-hop relaying.
- Names: `name.skein`, first come first served, owned by a key, signed and gossiped (no central registry, no DNS server).
- Everything between nodes is end-to-end encrypted (X25519 + ChaCha20-Poly1305) and signed (Ed25519). Relays only see the routing header.
- Bulk transfer: chunked, with lost chunks re-requested. Tested: 700 KB through a relay over real UDP sockets.
- Browser access with no setup: open `http://NAME.skein.localhost:8080/` (browsers resolve `*.localhost` to your own machine).
- Runs as a normal user: no admin rights, no drivers, no special licence (UDP sockets only).

**Not done yet**
- **Meshtastic (LoRa) and BitChat (Bluetooth) transports.** The `Transport` trait is the plug-in point (see `infra/src/transport.rs`); only UDP exists. Chunk sizes already adapt to each link's MTU, but over LoRa (~230 bytes per packet) the per-packet overhead (~200 bytes today: signature, sealed box, header) would leave almost no room. A compact radio mode with session keys is the planned fix.
- Route expiry / link-failure detection, congestion control, a hop-count-only metric.
- The map does not yet show Skein nodes, only existing masts.

## Coverage map (`site/`)

```
cd site && python -m http.server 8140     # then open http://127.0.0.1:8140
```

Click a red dot: the orange area is where a receiver of your choice (gain, height, sensitivity) can hear it, computed along 72 radials over real elevation data. The dashed grey line is the same range without relief. Then click anywhere: you get the path loss, the signal level and the antenna (Yagi length or dish diameter) needed there.

Model: free-space loss + single knife-edge diffraction over the terrain profile (ITU-R P.526 approximation), earth radius k = 4/3. It is an estimate for planning, not a survey. ANFR's public dataset has no transmit power, so EIRP is a setting (default 50 dBm, a typical macro cell). Data: ANFR "Données sur les réseaux mobiles" (Licence Ouverte 2.0), regenerate with `site/data/build_emitters.py`; elevation: Mapterhorn tiles.

The propagation maths are tested: `node site/test/propagation.test.js`.

## The software (`infra/`)

```
cd infra
cargo test                                   # unit + 3-node relay test over UDP
cargo run --release -- run --site ./mysite --name hello --peer some.host:7777
```

Then anyone connected to the same network opens `http://hello.skein.localhost:8080/`. `skein addr` prints your address. Your private key is `skein.key`: back it up, never share it.

Packet layout and trust model: [`infra/PROTOCOL.md`](infra/PROTOCOL.md).

## Radio rules (France)

No licence is needed for the licence-free ISM bands (433 MHz, 868 MHz, 2.4 GHz) within the legal power and duty-cycle limits (868 MHz: 25 mW ERP, 1% duty cycle on most sub-bands). Skein never transmits on its own: radio use is the job of the transport you plug in. Check the current ARCEP/ANFR rules before building a radio link.

## Licence

MIT.
