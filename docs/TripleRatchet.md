# Experimental cryptographic profile

Sigil implements published designs independently; no Signal source or wire interoperability is implied. Implementation status belongs in [Status.md](Status.md).

## Pinned references

| Reference | Revision | PDF SHA-256 |
| --- | --- | --- |
| [PQXDH](https://signal.org/docs/specifications/pqxdh/pqxdh.pdf) | 3, 2024-01-23 | `9fd0e02a5e13075b64adc7aa6dc9baade4f65af70b5571a332991756d98fe896` |
| [Double/Triple Ratchet](https://signal.org/docs/specifications/doubleratchet/doubleratchet.pdf) | 4, 2025-11-04 | `1d9b4dc3c6440b0777d747ff42707fccba3a45d209a2bdc33d1ea816aa05990c` |
| [ML-KEM Braid](https://signal.org/docs/specifications/mlkembraid/mlkembraid.pdf) | 1, 2025-09-26 | `c38a3ab844c7c583e7be15ff714b07792220cb662b5d1e9590e46aa5909a3ee6` |
| [XEdDSA](https://signal.org/docs/specifications/xeddsa/) | 1, 2016-10-20 | `a65684d87d747934e5b05698ed20bec72cd3c30e3f7ff2f6376555d65a5104b7` |

## Identity and handshake

XEdDSA uses curve25519-dalek primitives, canonical reduced scalars in both orientations, branchless orientation selection and 64 fresh OS-random nonce bytes. Verification rejects noncanonical scalars and weak points. Owned secret buffers are zeroized; compiler/library copies and persistent remnants have no complete erasure guarantee.

PQXDH combines X25519 with one-time ML-KEM-1024 and optional one-time EC prekeys. Its HKDF-SHA-256 input is 32 bytes FF, DH1–DH3, optional DH4, then the KEM secret; salt is 32 zero bytes. Info is `Sigil/experimental/pqxdh/v0_CURVE25519_SHA-256_ML-KEM-1024`. Verify identity-bound prekey signatures before use. Successful decapsulation alone is not authentication.

The public bundle uses `SGPQ 00 01 01 00`. Keys use tag 1 + EC[32] or tag 2 + KEM[1568]. Bundle fields: identity, signed EC prekey/signature[64], KEM prekey/signature[64], optional-EC flag and key. Exact sizes: 1,772 or 1,805 bytes. Bundle IDs exclude randomized signatures.

Native initial messages require `SGPQ 00 02 02 00`, sender identity, ephemeral EC, bundle ID[32], KEM ciphertext[1568], u32 ciphertext length and ciphertext. Profile 2 derives the initial key with `Sigil/experimental/pqxdh/initial/v1_TripleRatchet`, also bound into associated data. No classical fallback.

## Specification interpretations

- Braid's ML-KEM header is `seed || SHA3-256(vector || seed)`, preserving the standard FIPS 203 public-key hash order.
- Delayed fragments report `receiving_epoch = message.epoch - 1`; they do not advance Braid. A future epoch advances only one step from `Ct2Sampled`.
- SPQR caches counters strictly before the target, then derives the target key; previous-chain sealing includes its final counter.
- The section 7.2 chain KDF binds the current chain key and counter explicitly. These choices require protocol review; round trips alone cannot settle specification ambiguity.

## KDF and message profile

Integers are big endian. Epochs are u64; counters are u32, starting at one in SPQR and zero in the classical ratchet. Overflow rejects the candidate.

- `B = Sigil/experimental/braid/v0_MLKEM1024_SHA-256_RaptorQ64`
- `P = Sigil/experimental/spqr/v0_MLKEM1024_SHA-256_RaptorQ64`
- `T = Sigil/experimental/triple-ratchet/v0_X25519_MLKEM1024_SHA-256_AES256GCMSIV_RaptorQ64`

HKDF/HMAC use SHA-256; Z denotes 32 zero bytes.

| HKDF operation | Salt | Input | Info | Output |
| --- | --- | --- | --- | --- |
| Split handshake | Z | SK | T + `:Initialize` | 64: EC/SPQR seeds |
| Braid output | Z | raw KEM secret | B + `:SCKA Key` + epoch | 32 |
| Authenticator update | prior root | output key | B + `:Authenticator Update` + epoch | 64: root/MAC |
| SPQR initialize | Z | SPQR seed | P + `Chain Start` | 96: root/A→B/B→A |
| SPQR epoch | prior root | Braid output | P + `Chain Add Epoch` | 96: root/A→B/B→A |
| SPQR step | Z | chain key | P + `Chain Step` + counter | 64: chain/message key |
| Hybrid key | SPQR message key | EC message key | T | 32 |

Initialize Braid authentication by updating zero root at epoch 1 with the SPQR seed. Header MACs bind B + `:ekheader` + epoch + header; ciphertext MACs bind B + `:ciphertext` + epoch + ct1 + ct2.

RaptorQ uses fixed objects of 96/1536/1408/192 bytes, one source block/sub-block, alignment one and 64-byte symbols. Fragments contain a four-byte RFC 6330 payload ID plus symbol. Send systematic then fresh repair symbols; 24-bit IDs never wrap. Decoders accept at most 64 distinct symbols, reject other blocks/conflicting duplicates, and never take parameters from the network. No Reed–Solomon-equivalent vulnerable-message-set bound is claimed.

Each ratchet retains at most 128 skipped keys and derives at most 128 per packet. Unrecoverable larger gaps can stall a session; capacity errors do not authorize automatic reset. SPQR retains at most three nonempty chains and skipped keys from `max(sending_epoch, receiving_epoch).saturating_sub(2)` onward. Successful local sends can prune receive keys; incoming eviction commits only after authentication. Evicted late messages fail.

Triple headers: `SGTR 00 01 00 00`, EC DH[32], previous/current u32 counts, SPQR length[1], and SPQR header[17 or 85]. Braid contributes epoch[8], kind[1], optional fragment[68]; SPQR adds two u32 counts. Header size: 66/134 bytes. Plaintext maximum: 65,536 bytes.

AES-256-GCM-SIV uses each hybrid message key once with zero nonce. AAD is `T || context[32] || complete_header`. Stage both ratchets; discard both on any authentication/error. Persist state and exact ciphertext/incoming content atomically before release. Standalone Braid decoding is not authenticated transport.

## Native framing and persistence

`SGHI 00 02 00 00` contains u32 initial length, a fixed 1,694-byte profile-2 handshake encrypting empty content, and the Triple packet carrying application content. Context hashes the entire handshake. Authenticate both before consuming prekeys or accepting messages.

Repeat the exact initial header until authenticated peer traffic confirms the session. Server receipts cannot confirm it. The receiver derives a stable session from handshake and peer context. Unconfirmed sends cannot extend the first delivery's frozen expiry. Initial plaintext is capped at 65,374 bytes within the 67,230-byte mailbox limit.

Version-1/raw native initials cannot be newly accepted or retransmitted. Legacy classical sessions remain history-only. Migrations never reconstruct absent handshake state or rewrite retry ciphertext. `SGTS` version-2 checkpoints seal both ratchets and bounded Braid state, capped at 32,804 sealed bytes; restore validates state relationships and exact framing. Older checkpoint insertion order provides initial skipped-key eviction order.

Signed recovery requests use 176-byte `SGRR 00 01 00 00`: original message ID, requester/target fingerprints, absolute expiry and XEdDSA signature over the first 112 bytes. They are visible metadata, not encrypted content. IDs bind the endpoints/original message; retries never renew expiry. Fresh-session responses preserve logical bytes and require retained signed-request authorization, at most three hops. See [session rules](SessionsDevices.md).

Committed replacement clears superseded checkpoints from live SQLite files; migration vacuums older free pages. Recovery must never import live state. Filesystem snapshots, flash remnants and external copies require platform protection; see [erasure boundaries](Security.md#erasure).

## Independent fixtures

`crypto/tests/reference.c` uses OpenSSL/libsodium/json-c; `private-credentials.c` uses libsodium. Their fixtures test implementation agreement, not independent security certification.

```sh
cc -std=c11 -Wall -Wextra -Werror -O2 crypto/tests/reference.c -o /tmp/sigil-reference $(pkg-config --cflags --libs openssl libsodium json-c)
/tmp/sigil-reference crypto/tests/vectors/pqxdh.json
```

The ignored `generate_reference_inputs` helper writes candidates to `/tmp/sigil-pqxdh-vectors.json`. Check them independently before replacing committed fixtures. NIST vector provenance/notices remain in `licenses/NIST-Vectors.txt`.
