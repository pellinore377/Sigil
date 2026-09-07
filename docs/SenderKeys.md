# Experimental Sender Keys profile — implementation contract

Status: local crypto, key distribution and same-server text delivery are implemented and tested. Private-group coordination, distribution recovery and independent composition review remain open. Group authorization and ordering are in [Groups.md](Groups.md). This is a Sigil application profile using the Sender Keys design as a reference, not Signal wire interoperability.

## Referenced construction and limitations

The reference construction uses a symmetric chain and a separate sender signature key distributed through authenticated pairwise channels. Our primitive choices and framing below are explicit Sigil choices; the reference paper is not a proof of this adaptation. Compromising a chain reveals derivable future message keys. Ordinary chain advancement does not heal that exposure; new entropy must be distributed through channels with the required security. Static epoch signing keys do not provide forward-secure signatures. [Sender Keys analysis, version 2](https://arxiv.org/abs/2301.07045v2).

## Primitive and context selection

- Reuse the existing HMAC-SHA-256 chain step: message key is HMAC(chain, `0x01`); next chain is HMAC(chain, `0x02`).
- Reuse existing AES-256-GCM-SIV message encryption with a zero nonce under a distinct per-message key. No message-key reuse is permitted; the durable client must commit counter advancement and ciphertext together and must never restore live sender state from a history backup.
- Generate a separate fresh XEdDSA signing identity per sender epoch. It does not replace the sender's independently authenticated device identity. Sign a domain-separated SHA-256 digest of the exact header and ciphertext. Verify the signature before doing skipped-key work or decrypting.
- Context binds group ID, committed state digest, epoch, sender-device fingerprint and a fresh random sender-chain ID. Packet headers additionally bind counter and logical message ID. Version tags and big-endian fixed-width integers are canonical. Unknown versions, trailing data and length mismatches fail.
- Generate the initial chain, signing key and chain ID from the existing operating-system-backed cryptographic generators. Sender distributions contain the initial chain and signing public key, never its private signing key.

The distribution is sensitive plaintext and must travel inside authenticated pairwise encryption. A recipient must match the authenticated origin to the named authorized sender and pin the exact distribution to the committed group state. A distribution cannot overwrite an advanced receiver or revive a retired epoch. A transport receipt does not prove a recipient installed a key.

## Lifetime and failure behavior

Every committed group-state transition defines a new key epoch, including policy changes. Binding distributions to a state digest must not leave policy-only transitions with an ambiguous reusable epoch. This conservative rule avoids a separate mutable mapping between policy revisions and key epochs. The local membership prototype's earlier policy-only epoch exception has been removed.

A sender exports its initial distribution before its first message; callers durably prepare encrypted fan-out before advancing the sender chain. Later retries use those frozen encrypted distribution packets, not a regenerated or exported historical chain. Every member/device removal invalidates old outgoing epochs atomically with the local membership cutover. Fresh keys go only to the new authorized set. Frozen/closed/removed states cannot prepare new group sends.

Use the existing bounded skipped-key policy: at most 128 newly skipped positions per receive operation and at most 128 retained skipped message keys, evicting the oldest only on successful authenticated receipt. Replay or evicted messages fail explicitly. Invalid signatures, ciphertext, oversized counter jumps and failed persistence leave the original receiver unchanged. This does not promise unlimited offline reordering.

Reject new application delivery under an old committed state after local cutover; already authenticated retained history remains readable. An isolated sender can still use stale knowledge, and a malicious ordering service can withhold newer state. This profile does not claim instantaneous global removal. A stale sender refreshes state and prepares a new logical send according to the eventual conversation retry rules; existing ciphertext is never silently re-encrypted under its old transport identity.

## Required evidence

The schema 45 same-server delivery increment freezes one group ciphertext and unique per-recipient transport IDs with the sender counter in one transaction. The mailbox remains opaque and uses its existing bounded queue and authenticated sender routing. Group packets are a distinct worker event; their acknowledgement journal must not collide with direct-message or recovery journals. A distribution must have a stored server-acceptance receipt before sending its dependent group packet. This orders local submission without claiming remote key installation.

Keep at most 16 pending logical messages per group (reusable pending capacity, not a lifetime message limit). Submit through a bounded, durable cursor while preserving pending order per recipient. Local membership cutover cancels pending old-state fan-out and erases its retry ciphertext with the live keys. It cannot recall network work already started. Store accepted history separately from live keys; every accepted retry must match the original packet and retained logical content. Reject newly arriving old-state messages after cutover, while allowing exact acknowledgement retries for already committed history.

These packet headers expose routing context, state digests and sender-chain identifiers to the transport observer. This profile does not hide traffic-derived group membership or claim anonymous delivery. The private-membership service gate is separate and still closed.

Independent chain/signature/AEAD vectors must remain enabled. Add context/ciphertext/signature substitution tests, receiver failure atomicity, reorder/replay/skipped-key boundaries, sender/receiver encrypted checkpoint binding, counter exhaustion without wraparound, and fresh-epoch tests showing precisely which old key material cannot decrypt the new epoch. Then test client commits, distribution fan-out, removal/freeze cutover and failed/restarted delivery before group messaging is enabled. Do not mark item 2 complete from the in-memory engine.

## Local engine evidence

`sigil_crypto::sender_keys` now provides fresh sender/distribution creation, signed packets, bounded atomic receiver evolution and context-bound encrypted sender/receiver checkpoints. Eight focused tests pass. The tests cover every byte of a small packet, valid signatures with invalid AEAD, receiver attempts to forge a sender, reorder/eviction behavior, overflow refusal, maximum plaintext framing, restored skipped keys and independent fresh-epoch entropy. No raw live checkpoint export was added.

`crypto/tests/reference.c --sender-keys` independently generates three packets with OpenSSL HMAC-SHA-256/AES-256-GCM-SIV/SHA-256 and libsodium signatures. The Rust test compares the complete deterministic packet header/ciphertext and final chain, and decrypts/verifies the C packets. The C signatures use Ed25519 with a positive-orientation public key converted to X25519, exercising the XEdDSA verification profile; they do not purport to reproduce randomized XEdDSA signing bytes. Existing `--xeddsa` fixtures separately test that signer in both orientations.

The C generator passes AddressSanitizer/UndefinedBehaviorSanitizer. Regenerating the old `--xeddsa` and `--ratchet` fixtures produces byte-identical existing files. New fixture: `crypto/tests/vectors/sender-keys.json`, SHA-256 `35d6ee8ab2ede0925124c67d57cb8fe421ed7815f64f4bd25425d3e575504be4`.

```sh
cc -std=c11 -O1 -g -Wall -Wextra -Werror -fsanitize=address,undefined \
  -I/usr/include/json-c crypto/tests/reference.c -lcrypto -lsodium -ljson-c \
  -o /tmp/sigil-reference
/tmp/sigil-reference --sender-keys
cargo test --locked -p sigil-crypto sender_keys::
```

These checks establish implementation agreement with the selected local profile; they are not an independent security audit or a composition proof. The schema 44 client increment now tests secret-control history exclusion, authenticated group membership at distribution, durable receiver installation and atomic key cutover; see [Groups.md](Groups.md). Schema 45 additionally tests durable application fan-out and fresh-epoch cutover. Distribution recovery, earlier-history sharing and group-scoped trust are still pending.
