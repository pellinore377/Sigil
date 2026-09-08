# Sender Keys profile

This is Sigil framing based on the [Sender Keys analysis, version 2](https://arxiv.org/abs/2301.07045v2), PDF SHA-256 `23b7285aa1dcbec272764b865dee8fa0b0dde8db2755dd1daabadd197a0bcc81`. The paper is not a proof of this adaptation. See [group authorization](Groups.md).

A sender epoch contains a random symmetric chain, random chain ID and separate XEdDSA signing key. Message key = HMAC-SHA-256(chain, 0x01); next chain = HMAC-SHA-256(chain, 0x02). AES-256-GCM-SIV encrypts once per message key with zero nonce. Sign the domain-separated digest of exact header/ciphertext.

Context binds group, committed state digest, epoch, sender fingerprint and chain ID. Packet headers additionally bind counter, logical message ID and length. Integers are big endian; strict version/length checks reject trailing bytes. Codecs are in `crypto/src/sender_keys.rs` and `client/src/group_control.rs`.

The sensitive 208-byte initial distribution contains the chain and signing public key, never signing secret. Freeze encrypted per-recipient fan-out before the first send; erase the retained seed once fan-out is frozen. Later retries use exact ciphertext, not re-exported historical chains. Authenticate distribution origin, current membership and exact state; never overwrite an advanced receiver or revive a retired epoch.

Missed or expired distribution recovers through fresh group-authorized PQXDH. `SGKD` v2 adds an eight-byte current counter to the initial encoding. The receiver may advance that exact chain/signing identity, retaining existing skipped keys; it cannot rewind. A missing receiver key triggers a request after 60 seconds, with retries at most every 300 seconds. An explicit recovery request can bypass the initial wait. Distribution expiry alone never triggers replacement. Replies to authenticated requests freeze current-position ciphertext and cancel recipient deliveries preceding that position atomically. Those older messages require authorized history sharing. Per-device request/reply state is replaced, and membership cutover erases it.

Verify signatures before skipped-key work/decryption. Derive at most 128 skipped positions and retain at most 128 skipped keys; evict oldest only on authenticated commit. Replays, oversized jumps, exhausted counters, invalid signatures/ciphertext and failed persistence leave original state unchanged.

Sender advancement, logical history and per-recipient delivery identities commit atomically. There are at most 16 pending logical sends per group. A stored distribution transport receipt gates dependent sending but does not prove remote key installation. Removal/freeze/cutover cancels old pending ciphertext and live keys; accepted history remains.

Compromise of a chain reveals derivable future keys. Symmetric advancement does not heal; fresh entropy must travel through appropriately authenticated/healed channels. Static signing keys are not forward-secure signatures. Fresh distribution to one sender does not heal every sender. Packet wrapping hides explicit headers but not routing/timing or knowledge retained by former members.

Independent fixtures are in `crypto/tests/vectors/sender-keys.json`; `crypto/tests/reference.c --sender-keys` generates them using OpenSSL/libsodium. Its final chain also checks the current-position distribution encoding.
