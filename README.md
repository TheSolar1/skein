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

**Live: https://thesolar1.github.io/skein/** (served from the `gh-pages` branch; refresh it with `git subtree split --prefix site -b gh-pages && git push -f origin gh-pages`).

The map shows the nodes that transmit **our network** (Skein, green) and **the other mesh networks** (Meshtastic, blue; a France snapshot of the public Meshtastic map), plus mobile masts as an optional reference layer.

- Click a node, or press **Zones for nodes in view**: the area it reaches is painted in colours that say **which antenna you need** (stock dipole, 5 dBi omni, small/medium/long Yagi, dish), computed over real terrain, with the antenna size for the chosen band in the legend and the area (km²) of each colour.
- Click anywhere on the zones: the antenna needed at that exact spot (Yagi length or dish diameter).
- **Place a node** to see what your own Skein node would cover; **Export my nodes** gives JSON to send as a pull request to `site/data/skein-nodes.json` (it is empty today: no Skein node exists yet).
- Bands: 868 MHz and 433 MHz LoRa, 2.4 GHz Wi-Fi/BLE (all licence-free). Power, sensitivity and antenna heights are adjustable.
- Share a view: `?view=lat,lon,zoom&zones=1`.

Model: free-space loss + single knife-edge diffraction over the terrain profile (ITU-R P.526 approximation), earth radius k = 4/3, 300 m grid. An estimate for planning, not a survey. Meshtastic positions are what each owner chose to publish (many are blurred), and only position, role and hardware are kept. Data: Meshtastic map (`site/data/build_mesh.py`), ANFR masts (`build_emitters.py`, Licence Ouverte 2.0), elevation by Mapterhorn.

Tests: `node site/test/propagation.test.js`.

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
