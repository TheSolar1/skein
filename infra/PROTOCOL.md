# Skein protocol (v1 draft)

All integers are encoded with bincode varints. A **packet** is `{kind, ttl, id, src, dst, body}`; the header is in clear so relays can route. Maximum 16 hops.

## Identity and addresses
- A node has an Ed25519 key. Its X25519 key is derived from the same 32-byte seed.
- Address = `5c1e` + first 14 bytes of `blake3(ed25519_public_key)`. An address therefore proves nothing without its key, and a key cannot claim someone else's address.

## Kinds
1. **Announce** `{vk, xpub, seq, hops, sig}`. `sig` covers `vk, xpub, seq` (not `hops`). Flooded to neighbours; a receiver keeps it when `seq` is newer, or equal with fewer hops, then forwards it with `hops + 1`. A new neighbour gets a full table dump (its own announce, all routes, all names).
2. **Name** `{name, owner, seq, sig}`. Valid when `name` matches `[a-z0-9-]{1,63}` and the signature verifies under `owner`. The first owner keeps the name; only the same owner can publish a higher `seq`. A name resolves to the address of its owner's key.
3. **Data**. `body` = sealed box (ephemeral X25519 → recipient key, ChaCha20-Poly1305, associated data = `src‖dst‖id`) containing `{from_vk, sig, msg}`. The recipient checks that `addr(from_vk) == src` and that `sig` covers `src‖dst‖id‖msg`. Messages: `Get`, `Chunk`, `Resend`, `NotFound`.

4. **Session** (radio links, MTU below 1000). `body` = `counter(8 bytes, big endian) ‖ ChaCha20-Poly1305` under a key derived from X25519(static, static) and both public keys; nonce = direction byte + counter; associated data = `src‖dst`. Authenticates the pair without a signature; a 64-wide sliding window rejects replays. The counter is seeded from the clock so it keeps increasing across restarts.

## Meshtastic
Packets are carried as port 256 (`PRIVATE_APP`) payloads (max 233 bytes) sent with `hop_limit` 0: Skein does the routing. Floods are one broadcast; unicast uses the neighbour's radio node number as the next hop.

## Bulk transfer
`Get{req, path}` is answered with `Chunk{req, idx, total, data}` messages sized from the link MTU. The requester re-asks missing indexes with `Resend` after 1.5 s without progress (up to 150 per request).

## What an observer sees
Source and destination addresses and packet sizes. Not the content, the path requested, or the file names.

## Known limits
- Name squatting is possible (first come first served, no cost). Owners can be pinned by key outside the protocol.
- No forward secrecy for the recipient's static key if it leaks later (the sender side is ephemeral).
- No replay protection beyond random packet ids; a replayed request is simply answered again.
- Announcements are not rate-limited.
