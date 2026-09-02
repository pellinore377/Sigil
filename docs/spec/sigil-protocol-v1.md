# Sigil Protocol v1

**Status: draft, frozen for implementation.** This is the normative
specification of every derivation and wire format that must be bit-exact
between Sigil clients and servers. The design it implements is described in
[`docs/blind-backend.md`](../blind-backend.md); this document does not
repeat the reasoning, only the rules.

The reference implementation is the `sigil-protocol` crate in
[`protocol/`](../../protocol/). The test vectors in
[`protocol/vectors/v1.json`](../../protocol/vectors/v1.json) are generated
by it and are normative: an implementation that disagrees with a vector is
wrong. Selected values are quoted below; the file has all of them.

Words in capitals (MUST, MUST NOT, SHOULD) are used as in RFC 2119.

---

## 1. Primitives

The protocol is Sigil's own. The primitives are not, deliberately.

| Role | Primitive | Sizes |
|---|---|---|
| hash `H` | BLAKE3-256 | out 32 |
| KDF | BLAKE3 derive-key mode | out 32, or `n` via XOF |
| AEAD | XChaCha20-Poly1305 | key 32, nonce 24, tag 16 |
| signature | Ed25519 | secret 32, public 32, signature 64 |
| KEM | SigilKEM = X25519 + ML-KEM-768 (section 2) | public 1216, ciphertext 1120, secret 32 |
| password hash | Argon2id v1.3 | m = 65536 KiB, t = 3, p = 1, out 32 |

Definitions:

- `H(x)` = BLAKE3(x), 32 bytes.
- `KDF(ctx, ikm)` = BLAKE3 in derive-key mode with context string `ctx`
  and key material `ikm`, 32 bytes.
- `KDF_n(ctx, ikm, n)` = the same, extended to `n` bytes with BLAKE3's XOF.
  The first 32 bytes equal `KDF(ctx, ikm)`.
- Every context string begins `sigil v1 ` and is one of the strings listed
  in Appendix A. Implementations MUST NOT invent new ones within v1.
- `‖` is concatenation. Where a derivation concatenates variable-length
  values, the lengths are fixed by the spec so no separator is needed.

Vectors (`kdf`):

```
H("")                          = af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262
H("sigil")                     = bf03715896f808253fa08d984681313aa9d50d76992482a8bd6bc2af4a830f4e
KDF("sigil v1 test", "abc")    = c3e04e5ffa4341138b4be73280ec980908b49fb7ed542dac34376244545c89b4
```

## 2. SigilKEM

A hybrid key encapsulation mechanism. Both components are standard; the
combiner is Sigil's. Both shared secrets, the whole ciphertext and the
whole public key enter the combiner, which is the generic construction that
needs only IND-CCA security from each component.

**Keys.** From a 32-byte `seed`:

```
x_secret       = KDF("sigil v1 kem x25519 seed", seed)          // clamped by X25519
(d ‖ z)        = KDF_n("sigil v1 kem mlkem seed", seed, 64)
(ml_dk, ml_ek) = ML-KEM-768.KeyGen_internal(d, z)               // FIPS 203 deterministic form
public         = X25519(x_secret, 9) ‖ ml_ek                     // 32 + 1184 = 1216 bytes
```

**Encapsulate(public, eseed)** with a 32-byte ephemeral seed:

```
e_secret   = KDF("sigil v1 kem x25519 eph", eseed)
m          = KDF("sigil v1 kem mlkem eph", eseed)
ss_x       = X25519(e_secret, public[0..32])
(ct_m, ss_m) = ML-KEM-768.Encaps_internal(public[32..], m)
ciphertext = X25519(e_secret, 9) ‖ ct_m                          // 32 + 1088 = 1120 bytes
shared     = KDF("sigil v1 kem combine", ss_x ‖ ss_m ‖ ciphertext ‖ public)
```

**Decapsulate** recomputes `ss_x` with the static X25519 secret and `ss_m`
with ML-KEM decapsulation, then the same combiner. Implementations MUST
NOT reject an all-zero `ss_x`; the combiner and ML-KEM's implicit rejection
make the output unpredictable regardless.

Vector (`kem`): seed `01`×32, eseed `02`×32 →
`shared = 945b1ea9a3660396eadd558d355bc337b62c754e4ebc38dd12e75e0cc44ca54b`.

## 3. SPE, the packed encoding

Every structure in this document is encoded as the concatenation of its
fields in the order listed, with:

- integers little-endian at fixed width: `u8`, `u16`, `u32`, `u64`;
- fixed-size byte arrays raw, no length;
- variable-length bytes and UTF-8 strings as `u32` little-endian length,
  then the bytes.

Nothing is self-describing. A decoder MUST reject trailing bytes and MUST
reject a `version` field other than `1`.

## 4. Identity

From a 32-byte `identity_seed`:

```
signing_secret = KDF("sigil v1 identity signing", identity_seed)   // Ed25519 secret
identity_pub   = Ed25519 public key of signing_secret
kem_secret     = SigilKEM.KeyGen(KDF("sigil v1 identity kem", identity_seed))
fingerprint    = KDF("sigil v1 fingerprint", identity_pub)
```

**Fingerprint display**: the first 20 bytes of `fingerprint` as lowercase
hex in ten groups of four separated by single spaces. Users compare this
out of band.

**Username**: `@localpart:server`. `localpart` is 1 to 32 characters of
`[a-z0-9._-]`, not beginning or ending with `.`. `server` is 1 to 253
characters of `[a-z0-9.-]`, containing at least one `.`, not beginning or
ending with `.`. Both are lowercase; clients lowercase user input before
use and servers reject anything else.

**Contact card**, SPE:

```
version u8 = 1
username string
identity_pub [32]
kem_pub [1216]
slot_server string          // server hosting this user's conversation slots
flags u8                    // bit 0: server offers TPM recovery; other bits MUST be 0
signature [64]              // Ed25519(signing_secret, "sigil v1 card" ‖ all preceding bytes)
```

Vector (`identity`): seed `03`×32 →
`identity_pub = 86f7c7c29fff3c9267f12f68867f330c896ef0773d14ff4fd9405db5a1cecd73`,
display `5de2 12e0 10ea 7e07 df1e 1747 f4eb 98eb 7b7b 4f21`.

## 5. Name-bound objects

The only two server objects tied to a person.

```
shelf_address           = KDF("sigil v1 shelf address", identity_pub)
shelf_key               = KDF("sigil v1 shelf key", identity_pub)
requests_address(period) = KDF("sigil v1 requests address", identity_pub ‖ u32le(period))
period                  = floor(unix_seconds / 2 592 000)          // 30-day periods
```

The shelf's contents are sealed with `shelf_key` under XChaCha20-Poly1305
with a random 24-byte nonce and associated data `"sigil v1 shelf" ‖ shelf_address`.

Reading, acknowledging or subscribing to a requests slot requires a proof
of ownership. The server supplies a fresh 32-byte `nonce`; the client
returns

```
proof = Ed25519(signing_secret, "sigil v1 requests read" ‖ requests_address ‖ nonce)
```

Writing to a requests slot requires only the address and a token. Clients
SHOULD subscribe to the current period's address and the next one.

Vector (`names`): period 689 →
`requests_address = 7cc28bbd9a92fc668e8ab80c53c6a39a1a29d2cbc3ef2edaa7bd8da43a42e79b`.

## 6. Epoch material

Every conversation has, per epoch, one 32-byte `epoch_secret` shared by all
members. In v1 it is produced by the group key schedule as
`MLS-Exporter("sigil v1 epoch", "", 32)`. Nothing below depends on that
choice.

```
slot_seed    = KDF("sigil v1 slot seed", epoch_secret)
read_cap     = KDF("sigil v1 slot read", slot_seed)
write_secret = KDF("sigil v1 slot write", slot_seed)                 // Ed25519 secret
write_pub    = Ed25519 public key of write_secret
address      = KDF("sigil v1 slot address", read_cap ‖ write_pub)
envelope_key = KDF("sigil v1 envelope key", epoch_secret)
call_room    = KDF("sigil v1 call room", epoch_secret)
```

A new epoch yields a new address. Members MUST keep the previous epoch's
material until they have received every envelope written to it.

Vector (`epoch`): epoch_secret `05`×32 →
`address = 4cff9736228b8a6558f841fb129a651e4dcea304f8af47091071b735041e19ae`.

## 7. Events and envelopes

**Event**, SPE. Every kind of thing a client sends has this shape.

```
version u8 = 1
kind u16
ts_ms u64            // sender's clock, ms since Unix epoch; display only
reference bytes      // what this refers to, or empty
body bytes           // defined per kind by the application layer
```

Kinds: 1 text, 2 reaction, 3 edit, 4 redact, 5 receipt, 6 typing,
7 membership, 8 policy, 9 media, 10 call, 11 welcome, 12 commit,
13 proposal, 14 link, 15 poll, 16 vote, 17 poll end, 18 location (wire
spec 16). Unknown kinds MUST be ignored, not rejected.

Ordering of events in a slot is by the server-assigned sequence number,
never by `ts_ms`.

**Padding.** `pad(p)` appends one byte `0x80` then zero bytes to the
smallest of the plaintext sizes 984, 4056, 16344 that fits. `unpad` strips
trailing zeros, then requires and removes one `0x80`. A plaintext over
16343 bytes MUST be sent as a media blob, not an envelope.

**Envelope.** With `plain` the group layer's message bytes (in v1, an MLS
message; envelopes exist because MLS messages carry the group id in the
clear):

```
nonce    = 24 random bytes
envelope = nonce ‖ XChaCha20-Poly1305(envelope_key, nonce,
                     ad = "sigil v1 envelope" ‖ address, pad(plain))
```

An envelope is exactly 1024, 4096 or 16384 bytes. A server MUST reject any
other length.

**Put signature.** A `slot.put` carries

```
sig = Ed25519(write_secret, "sigil v1 slot put" ‖ address ‖ envelope)
```

The first put to an address pins `write_pub`; the server MUST verify every
later put against it. A read presents `(read_cap, write_pub)` and the server
MUST check `KDF("sigil v1 slot address", read_cap ‖ write_pub) == address`.

Vector (`envelope`): event {kind 1, ts 1756684800000, body "hello, world"}
sealed with nonce `06`×24 under the section 6 material is the 1024-byte
value in the file; `put_signature = e86343d4e088ffb6aa597993f4a5ae6e…`.

## 8. Bags

A bag is one request from a client to a server, sealed to the server's
SigilKEM public key so that the Envoy relaying it sees only ciphertext.

**Request.**

```
(ct, shared) = SigilKEM.Encapsulate(server_pub, eseed)      // eseed random
request_key  = KDF("sigil v1 bag request", shared)
response_key = KDF("sigil v1 bag response", shared)
nonce        = 24 random bytes
bag          = 0x01 ‖ ct ‖ nonce ‖ XChaCha20-Poly1305(request_key, nonce,
                    ad = "sigil v1 bag" ‖ ct, pad_req(request))
```

`pad_req` pads (as in section 7) so that the whole bag is exactly 2048,
8192, 32768 or 266240 bytes; the plaintext capacities are 887, 7031, 31607
and 265079. The largest bucket exists for one thing: a 256 KiB media or
backup chunk with its token.

**Response.**

```
nonce    = 24 random bytes
response = nonce ‖ XChaCha20-Poly1305(response_key, nonce,
               ad = "sigil v1 bag response", pad_resp(plain))
```

padded so that the response is exactly 1024, 4096, 16384, 65536 or 266240
bytes; the largest carries a chunk.

A server MUST answer every bag with a response of a valid size, including
errors, so that the Envoy learns nothing from the reply length beyond its
bucket. The body of `request` and `response` (the operation and its
fields) is defined in the wire protocol document, not here.

Vector (`bag`): server seed `07`×32, eseed `08`×32, request `"ping"` →
`request_key = 4121f32ec541d633d06c9945244e0c1e51e97b56ff99921abd784e5070a4043c`.

## 9. Recovery and backup

```
salt         = 16 random bytes, stored by the server against the username
pw_key       = Argon2id(password_nfc_utf8, salt; m = 65536 KiB, t = 3, p = 1; 32)
recovery_key = 32 random bytes, generated on the first device
backup_key   = KDF("sigil v1 backup key", pw_key ‖ recovery_key)
data_key     = 32 random bytes; encrypts the backup itself
backup_label = KDF("sigil v1 backup label", data_key)
wrap         = nonce ‖ XChaCha20-Poly1305(backup_key, nonce,
                   ad = "sigil v1 data key wrap", data_key)      // 72 bytes
tpm_auth     = KDF("sigil v1 tpm auth", pw_key)
```

Passwords are NFC-normalised before hashing. The server stores `salt` and
`wrap` by username, and the backup chunks by `backup_label`. It never
holds `pw_key`, `recovery_key`, `backup_key` or `data_key`.

**Recovery code.** `recovery_key ‖ check`, where `check` is the first two
bytes of `KDF("sigil v1 recovery code", recovery_key)`, encoded as base32
with the RFC 4648 alphabet in lowercase and no padding: 34 bytes become 55
characters, displayed in groups of five separated by `-`. Parsers MUST
accept any case and ignore `-` and whitespace, and MUST reject a code
whose check bytes do not match.

Vector (`recovery`): password `correct horse battery staple`, salt `0b`×16,
recovery_key `0c`×32 →
`backup_key = 46bdb478ff0ee5e925867381b85a6fce5796b3b7702019602e87a85968ac44e6`,
code `bqgay-dambq-gayda-mbqga-ydamb-qgayd-ambqg-aydam-bqgay-dambq-gcf5q`.

## 10. Device linking

**Offer** (shown as a QR code by the new device), SPE:

```
version u8 = 1
kem_pub [1216]       // the new device's SigilKEM public key
nonce [16]
```

```
offer_slot = EpochMaterial(KDF("sigil v1 link offer", offer_bytes))     // section 6 derivations
```

The existing device scans the offer, computes
`(ct, shared) = SigilKEM.Encapsulate(kem_pub, eseed)`, and writes `ct` as
an ordinary envelope (section 7, kind 14) into the offer slot. The new
device has no tokens yet, so it only reads, with the offer slot's
`read_cap`; the existing device pays for every write in the exchange.
Both sides then derive:

```
link_secret = KDF("sigil v1 link secret", shared ‖ nonce)
link_slot   = EpochMaterial(KDF("sigil v1 link rendezvous", link_secret))
sas         = KDF_n("sigil v1 link sas", link_secret, 7), each byte & 0x3f
```

Both devices display the seven emoji `TABLE[sas[i]]` from Appendix B in
order. The user MUST confirm the match on the *existing* device before it
writes anything to the link slot. The transfer is a sequence of envelopes
in the link slot carrying events of kind 14, whose bodies are defined in
the wire specification (section 10 there).

Vector (`linking`): new-device seed `0f`×32, nonce `10`×16, eseed `11`×32 →
sas indices `[60, 6, 28, 45, 17, 37, 51]`, displayed `📌 🐸 ☕ 🚂 🍌 🎯 ☁️`.

## 11. Not in this document

The operations inside a bag, the Envoy control channel, blind tokens, the
requests-slot envelope and the MLS bindings are in
[`sigil-wire-v1.md`](sigil-wire-v1.md). Nothing there changes a derivation
defined here.

---

## Appendix A. Context strings

All KDF context strings used in v1. An implementation MUST use exactly these.

```
sigil v1 kem x25519 seed        sigil v1 kem mlkem seed
sigil v1 kem x25519 eph         sigil v1 kem mlkem eph
sigil v1 kem combine
sigil v1 identity signing       sigil v1 identity kem
sigil v1 fingerprint
sigil v1 shelf address          sigil v1 shelf key
sigil v1 requests address
sigil v1 slot seed              sigil v1 slot read
sigil v1 slot write             sigil v1 slot address
sigil v1 envelope key           sigil v1 call room
sigil v1 bag request            sigil v1 bag response
sigil v1 backup key             sigil v1 backup label
sigil v1 tpm auth               sigil v1 recovery code
sigil v1 link secret            sigil v1 link rendezvous
sigil v1 link sas
sigil v1 link offer
sigil v1 requests envelope      sigil v1 token key id
sigil v1 test                   sigil v1 test rng        (vectors only)
```

MLS exporter label (not a KDF context): `sigil v1 epoch`.

Signature and AEAD domain strings (not KDF contexts):
`sigil v1 card`, `sigil v1 requests read`, `sigil v1 slot put`,
`sigil v1 envelope`, `sigil v1 shelf`, `sigil v1 bag`,
`sigil v1 bag response`, `sigil v1 data key wrap`, and those listed in the
wire specification's Appendix B.

## Appendix B. Emoji table

64 entries, index 0 to 63, row-major:

     0 🐶   1 🐱   2 🦁   3 🐴   4 🦄   5 🐷   6 🐸   7 🐙
     8 🐢   9 🦋  10 🐝  11 🐧  12 🦉  13 🐟  14 🐘  15 🐼
    16 🍎  17 🍌  18 🍇  19 🍓  20 🍒  21 🍍  22 🥕  23 🌽
    24 🍕  25 🍔  26 🍩  27 🎂  28 ☕  29 🧀  30 🥑  31 🍄
    32 ⚽  33 🏀  34 🎸  35 🎺  36 🎲  37 🎯  38 🎈  39 🎁
    40 🚗  41 🚀  42 ✈️  43 ⛵  44 🚲  45 🚂  46 🏠  47 ⛺
    48 ☀️  49 🌙  50 ⭐  51 ☁️  52 🌈  53 ❄️  54 🔥  55 🌊
    56 ❤️  57 🔑  58 🔔  59 ⏰  60 📌  61 ✂️  62 🔒  63 🧲

## Appendix C. Regenerating the vectors

```
cd protocol
cargo test                                   # verifies vectors/v1.json
cargo run --bin gen-vectors > vectors/v1.json   # only when the spec changes
```

A change to any derivation is a change to this document, a new vector
file, and a version bump of every affected context string.
