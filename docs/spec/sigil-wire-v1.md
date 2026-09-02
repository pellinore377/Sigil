# Sigil Wire Protocol v1

**Status: draft, frozen for implementation.** This document specifies what
travels inside a bag (the operations a client asks of a server), the frames
on the Envoy control channel, blind tokens, how MLS is plugged in, and the
semantics a server MUST implement for each operation. It builds on
[`sigil-protocol-v1.md`](sigil-protocol-v1.md), which defines every
derivation and the bag, envelope and encoding formats used here.

Reference implementation: `wire`, `token` and `requests` in the
`sigil-protocol` crate. Vectors: the `wire`, `token` and
`requests_envelope` sections of `protocol/vectors/v1.json`.

---

## 1. Transport

A client never connects to a server. It connects to an **Envoy** over TLS
1.3 (HTTPS or a WebSocket, or QUIC) and exchanges **frames** (section 6).
Every server operation is a **bag** (protocol spec section 8) carried in a
`Bag` frame; the Envoy forwards it to the named server over its own TLS
connection and returns the sealed response in a `BagResponse` frame.

A server accepts bags only from Envoys. It keeps one long-lived stream to
each Envoy that has ever subscribed a handle with it, on which it sends
`Deliver` frames.

A server that also acts as its own Envoy (`--role both`) still separates the
two internally: the bag is sealed by the client and opened by the home role,
so the code path and the stored state are the same as in the split
deployment.

## 2. Bag contents

Inside every request bag is one **request**: an operation code byte followed
by that operation's SPE fields (section 3). Inside every response bag is one
**response**: a status byte, then the operation's result fields if the
status is `0`.

Status codes:

| Code | Name | Meaning |
|---|---|---|
| 0 | ok | |
| 1 | malformed | could not decode, wrong lengths, trailing bytes |
| 2 | unauthorized | a signature, proof or capability failed |
| 3 | not_found | no such slot, blob, name, label or shelf |
| 4 | conflict | write key does not match the pinned key |
| 5 | token_invalid | bad signature or unknown key id |
| 6 | token_spent | nonce already seen for this key id |
| 7 | rate_limited | per-name or per-server backoff in force |
| 8 | too_large | chunk or envelope over its limit |
| 9 | unavailable | feature not offered (no TPM, no OIDC, …) |
| 10 | name_taken | registration of a claimed localpart |

An error response is the status byte alone. A server MUST NOT include
detail text: the client has the code and the operation, which is enough.

## 3. Operations

Field types are SPE (protocol spec section 3). `token` is a blind token
(section 5), 321 bytes, or empty where the table says optional. All
`address`, `read_cap`, `write_pub`, `label`, `shelf`, `id`, `wake_handle`
fields are 32 bytes; `sig` and `proof` are 64.

### 3.1 Slots

| Op | Request fields | Response | Server MUST |
|---|---|---|---|
| 1 `slot.put` | `address, write_pub, sig, envelope bytes, token` | `seq u64` | verify the token and mark it spent; if the address is new, pin `write_pub`; else require equality (`conflict`); verify `sig` per protocol spec 7 (`unauthorized`); require an envelope of 1024, 4096 or 16384 bytes (`too_large`); append, assign the next `seq` starting at 1; deliver to every subscribed handle |
| 2 `slot.get` | `read_cap, write_pub, after_seq u64, limit u16` | `count u16, (seq u64, envelope bytes)×count, more u8` | check `KDF("sigil v1 slot address", read_cap ‖ write_pub)` names an existing slot (`not_found`; a wrong capability on an existing slot is `not_found` too, never `unauthorized`, so the two are indistinguishable); return up to `limit` (max 64) envelopes with `seq > after_seq` in order |
| 3 `slot.ack` | `read_cap, write_pub, seq u64` | empty | same check; record that this reader has everything up to `seq`, for retention |
| 4 `slot.subscribe` | `address, wake_handle, proof bytes, token` | empty | verify and spend the token; for a conversation slot `proof` MUST be empty; for a requests slot `proof` MUST be a valid requests-read proof for a nonce the server issued to this Envoy stream within the last 60 s (section 3.7); record `address → wake_handle` and `wake_handle → this Envoy stream`; a slot may be subscribed before it has any writes |
| 5 `slot.unsubscribe` | `address, wake_handle` | empty | remove the pair; idempotent |
| 22 `requests.put` | `address, envelope bytes, token` | empty | verify and spend the token; require an envelope of 4096 or 16384 bytes (section 4); the address MUST be one some subscriber has proven ownership of (`not_found`); enforce per-address and per-issuing-server quotas (`rate_limited`); deliver like `slot.put` |

Sequence numbers are per address and never reused. A server MUST NOT store
a wall-clock time with a slot or an envelope.

**Retention.** A slot expires when every subscribed handle has acked its
last `seq`, or 30 days after its last write, in day granularity. Expiry is
the only per-slot temporal state and is stored as a day number.

**Ordering rule for the group layer.** When two members commit in the same
epoch, the commit with the lower `seq` is the one every member applies. A
member whose commit lands second discards it and re-proposes against the
new epoch. Servers do nothing; the rule is entirely in clients.

### 3.2 Shelves

| Op | Request | Response | Server MUST |
|---|---|---|---|
| 6 `shelf.put` | `shelf, sealed bytes, identity_pub, sig, token` | empty | require `shelf == KDF("sigil v1 shelf address", identity_pub)` and `sig` = Ed25519(identity, `"sigil v1 shelf put" ‖ shelf ‖ sealed`); replace the shelf's contents |
| 7 `shelf.take` | `shelf` | `sealed bytes` | pop one package (`not_found` when empty); servers MUST NOT hand the same package out twice |

`sealed` is an SPE list `count u16, (package bytes)×count`, each package
sealed under `shelf_key` (protocol spec 5). The server stores the list and
pops from it without being able to read a package.

### 3.3 Blobs

| Op | Request | Response | Server MUST |
|---|---|---|---|
| 8 `blob.put` | `chunk bytes, token` | `id` | require `len(chunk) == 262144` unless the client marks it final (the last chunk may be shorter, but MUST be padded by the client to a multiple of 4096); `id = H(chunk)`; store; idempotent on repeat |
| 9 `blob.get` | `id` | `chunk bytes` | return or `not_found`; extend expiry to 30 days from now, day granularity |

Chunks are encrypted by the client before upload with a per-file key that
travels inside the message (kind 9). The server sees only `id` and size.

### 3.4 Names

| Op | Request | Response | Server MUST |
|---|---|---|---|
| 10 `name.register` | `card bytes, gate bytes, token` | empty | verify the card (protocol spec 4) and that its `server` is this server; apply the registration policy to `gate` (invite code, OIDC ID token, proof-of-work, or nothing); claim the localpart (`name_taken`); store the card |
| 11 `name.lookup` | `localpart string` | `card bytes` | return the current signed card or `not_found` |
| 12 `name.update` | `card bytes` | empty | require the same `identity_pub` as the stored card and a newer card (cards carry no counter, so "newer" is: the request arrived later; last write wins) |

A server MUST NOT log which localpart was looked up.

### 3.5 Backup and recovery

| Op | Request | Response | Server MUST |
|---|---|---|---|
| 13 `backup.put` | `label, index u32, chunk bytes, token` | empty | store chunk `index` under `label`; overwrite allowed |
| 14 `backup.get` | `label, index u32` | `chunk bytes` | return or `not_found` |
| 15 `wrap.put` | `username string, salt[16], wrap bytes, sig` | empty | require `sig` = Ed25519(identity of `username`'s card, `"sigil v1 wrap put" ‖ salt ‖ wrap`); store salt and wrap by name |
| 16 `wrap.get` | `username string` | `salt[16], wrap bytes` | return or `not_found`; apply per-name backoff (`rate_limited`): 1 s after the first request in an hour, doubling per request, capped at 1 h |
| 17 `tpm.info` | | `ek_pub bytes, cert_chain bytes` | return the endorsement key and its certificate chain, or `unavailable` |
| 18 `tpm.relay` | `username string, command bytes` | `response bytes` | apply the same per-name backoff as `wrap.get`; if OIDC is configured, require a fresh ID token for this name's `sub` in the bag's `gate` position (`unauthorized`); pass `command` to `/dev/tpmrm0` and return the raw response |

The backup label never appears in the same request as a username. A
server that wants to correlate them has only timing, which `backup.put`
clients spread on a fixed hourly schedule.

### 3.6 Tokens and server info

| Op | Request | Response | Server MUST |
|---|---|---|---|
| 19 `token.credential` | `identity_pub, sig, gate bytes, blinded bytes` | `blind_sig bytes` | require a registered card for `identity_pub` on this server and `sig` = Ed25519(identity, `"sigil v1 credential" ‖ blinded`); blind-sign `blinded` with the credential key; at most once per name per key rotation |
| 20 `token.issue` | `credential bytes, count u16, (blinded bytes)×count` | `count u16, (blind_sig bytes)×count` | verify `credential` (a token under the credential key); enforce the daily quota for it (`rate_limited`); blind-sign each `blinded` (max 64 per request) with the token key |
| 21 `server.info` | | `server_card bytes` | return the signed server card |

**Server card** (SPE): `version u8 = 1, hostname string, kem_pub[1216],
token_key bytes (SPKI DER), flags u8, signing_pub[32]`, followed by a
64-byte Ed25519 signature by `signing_pub` over `"sigil v1 server card" ‖`
the preceding bytes. Flags: bit 0 TPM recovery offered, bit 1 OIDC gate on
registration, bit 2 open registration. The card also carries, as trailing
SPE fields after the signature in v1.1, the credential key and an Envoy
list; v1 clients ignore trailing bytes of a server card only.

### 3.7 Requests-read nonces

A server hands each Envoy stream a fresh 32-byte nonce in every
`Deliver`-stream keepalive (section 6.3) and accepts a requests-read proof
against any nonce it issued to that stream in the last 60 s. Nonces are
never stored beyond that window.

## 4. Requests envelopes

The requests slot has no epoch secret. A sender seals to the recipient's
identity KEM key:

```
(ct, shared) = SigilKEM.Encapsulate(recipient.kem_pub, eseed)
key          = KDF("sigil v1 requests envelope", shared)
envelope     = ct ‖ nonce ‖ XChaCha20-Poly1305(key, nonce,
                   ad = "sigil v1 requests" ‖ requests_address, pad(plain))
```

padded so the whole envelope is 4096 or 16384 bytes (plaintext capacity
2935 or 15223). `plain` is an event (protocol spec 7) of kind 11 whose body
is the group layer's Welcome plus the sender's signed contact card, so the
recipient can show who is asking before joining anything.

Vector (`requests_envelope`): recipient seed `12`×32, eseed `13`×32,
nonce `14`×24, 4096-byte envelope in the file.

## 5. Blind tokens

Scheme: RFC 9474 **RSABSSA-SHA384-PSSZERO-Deterministic**, RSA-2048.

```
key_id    = KDF("sigil v1 token key id", spki_der)
message   = "sigil v1 token" ‖ key_id ‖ nonce            // nonce: 32 random bytes
blinded   = Blind(pk, message)                            // client, RFC 9474 §4.1
blind_sig = BlindSign(sk, blinded)                        // server
signature = Finalize(pk, blind_sig, blinding_inverse, message)   // client
token     = 0x01 ‖ key_id ‖ nonce ‖ signature             // 321 bytes
spend_id  = H(nonce)
```

A server verifies `signature` over `message` under the key named by
`key_id`, then checks `spend_id` against its spent set for that key id and
records it. Keys rotate weekly; a server accepts the current and previous
key id, and drops a key's spent set when it stops accepting it.

Two keys: the **credential key** signs credentials (one per name per
rotation, obtained by proving the name); the **token key** signs daily
tokens (obtained by presenting a credential). Both are the scheme above.
Because both issuances are blind, the server cannot link a credential to a
name, a token to a credential, or a token to another token.

Envoys running the clocked tier obtain tokens the same way from a
credential the server issues to Envoys it accepts, so cover writes are
indistinguishable from real ones.

Vector (`token`): issuer key in the file as PKCS#8 DER; nonce `15`×32;
`key_id = 0801318c1c6d49ea14fe04f18fed54fd210cff4c1a79fb27b2a00db414b68fac`. The blinding inverse is recorded so
`Finalize` can be checked without reproducing the client's RNG.

## 6. The Envoy control channel

Frames between client and Envoy, SPE, first byte is the type. The same
`Deliver` layout is used on the server-to-Envoy stream.

| Type | Direction | Fields | Semantics |
|---|---|---|---|
| 1 `Bag` | client → Envoy | `id u32, server string, has_bind u8, [bind_handle[32]], bag bytes` | forward `bag` to `server`; if `has_bind`, first record `bind_handle → this connection` |
| 2 `BagResponse` | Envoy → client | `id u32, response bytes` | the server's sealed reply to `Bag id` |
| 3 `Deliver` | server → Envoy → client | `wake_handle[32], queue_seq u64, envelope bytes` | on the server stream `queue_seq` is 0; the Envoy assigns a per-handle `queue_seq` starting at 1 and stores the envelope until acked |
| 4 `Ack` | client → Envoy | `wake_handle[32], queue_seq u64` | the Envoy drops everything up to `queue_seq` for that handle |
| 5 `Push` | client → Envoy | `kind u8, token bytes` | how to wake this device: 0 none, 1 APNs, 2 FCM, 3 UnifiedPush endpoint URL |
| 6 `Release` | client → Envoy | `wake_handle[32]` | forget the handle and its queue |

### 6.1 Envoy state

Per connected device: its push registration and the set of handles bound
on this connection. Per handle: the queue of undelivered envelopes with
their `queue_seq`. Nothing else. An Envoy MUST NOT store the server name
next to a handle beyond the lifetime of the `Bag` frame that bound it, and
MUST NOT log bag contents, sizes, or server names.

### 6.2 Queues

A handle's queue holds at most 1,000 envelopes or 16 MiB, then the oldest
are dropped and the client backfills with `slot.get` on reconnect (its
last known `seq` per address tells it where to start). Queues live 30 days
after the last delivery.

### 6.3 Server stream

An Envoy opens one TLS connection to each server it has forwarded a
subscribe for, identified by its own SigilKEM public key in the handshake,
and keeps it open. The server sends `Deliver` frames on it and, every
30 s, a `Keepalive` (type 7: `nonce[32]`) whose nonce is the requests-read
nonce of section 3.7. If the stream drops, the server holds deliveries for
that Envoy for 24 h, then expires them; the Envoy's clients backfill.

### 6.4 Push

On `Deliver` for a handle whose device is not connected, the Envoy sends a
push containing nothing but the Envoy's hostname. In the clocked tier the
push is sent at the next scheduled tick rather than immediately.

## 7. The group layer: MLS

Sigil v1 uses MLS (RFC 9420) for every conversation, with these bindings:

- **Cipher suite**: `MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519`
  (0x0003) at launch; the X25519+ML-KEM-768 hybrid suite when its code
  point is assigned, behind a client feature flag. Both may be present on a
  shelf; a client takes the strongest package it supports.
- **Credential**: `BasicCredential` whose identity is SPE
  `identity_pub[32], device_pub[32], sig[64]` with
  `sig = Ed25519(identity, "sigil v1 device" ‖ device_pub)`. The leaf
  signature key is `device_pub`. Receivers MUST check `sig` and that
  `identity_pub` matches the contact card they hold for the sender.
- **Epoch secret**: `MLS-Exporter("sigil v1 epoch", "", 32)`, from which
  protocol spec section 6 derives everything.
- **Group id**: 32 random bytes chosen by the creator. Never sent to a
  server: the outer envelope hides it.
- **Application messages** carry an event (protocol spec 7). Commits and
  proposals are MLS messages wrapped the same way, with event kinds 12
  and 13 so a client can prioritise them on read.
- **Policy** (event kind 8) is a signed document `admins[], name, avatar
  blob, slot_server, join rule` applied by clients when validating a
  commit; a commit that violates policy is treated as invalid by every
  member, exactly like a bad signature.
- **Self-update**: every device commits a self-update at least every 24 h,
  which rotates the address (protocol spec 6).
- **Welcome** delivery is section 4.

## 8. Open rooms

An open room is a slot the host server can read: its `read_cap`,
`write_pub` and `write_secret` are published in a **room card** the server
serves at `name.lookup` of the room's localpart (rooms share the namespace,
prefixed `#`). Members write ordinary envelopes sealed under a
`envelope_key` that is also in the card; the server, holding it, indexes
events for search and serves history to newcomers with `slot.get`. Members
sign events with a per-room pseudonymous Ed25519 key by default and MAY
include their contact card in a kind-7 event to attach a username. Bans are
the server refusing `slot.put` from a listed pseudonymous key, on the
instruction of an admin listed in the card. Everything else in this document
applies unchanged; the only difference is who holds the keys.

## 9. Not yet specified

- OIDC token validation details beyond "signature, issuer, audience,
  expiry, `sub` bound to the name".
- The UnifiedPush server side the Envoy exposes (it is the public
  UnifiedPush spec; nothing Sigil-specific).
- Media chunk manifests inside kind-9 events.
- The clocked tier's exact scheduling constants.

---

## Appendix A. Vectors in this document

From `protocol/vectors/v1.json`, section `wire`:

- `requests.*`: 16 encoded requests, one per operation family, including
  a `slot.subscribe` with a requests-read proof.
- `responses.*`: success and error responses; `error.not_found` is the single
  byte `03`.
- `frames.*`: every frame type; `bag` binds a handle, `bag.nobind` does not.
- `server_card`: an unsigned card body with flags `0b011`.

Section `token`: a full issuance with the blinding inverse. Section
`requests_envelope`: a sealed Welcome.

## Appendix B. Additional domain strings

KDF contexts introduced here: `sigil v1 requests envelope`,
`sigil v1 token key id`, and `sigil v1 test rng` (vectors only).
Signature and AEAD domains: `sigil v1 requests`, `sigil v1 token`,
`sigil v1 shelf put`, `sigil v1 wrap put`, `sigil v1 credential`,
`sigil v1 device`, `sigil v1 server card`.
