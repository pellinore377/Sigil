# Device linking, profile 1

## Trust and user interaction

Linking uses three direct, physical QR scans between the intended devices:

1. The joining installation creates a pending offer with its independent identity, fresh device ID, challenge, ephemeral provisioning key and commitment to its own random transport credential. The sponsor scans this offer directly from that installation.
2. The sponsor displays an encrypted proposal containing its signed account/device binding and the complete proposed transcript. The joining device scans this return QR directly from the sponsor. It pins the exact proposal and displays the account context and confirmation value for approval.
3. After explicit confirmation, the joining device displays an encrypted response containing its own signed binding and role-specific consent. The sponsor scans it, checks the confirmation value and explicitly approves. Both signatures bind the complete transcript.
4. The live sponsor submits the complete proof over HTTPS. The server atomically creates the one-use authorization. The joining installation authenticates with the credential it generated locally, retrieves the proof, verifies it against its saved exchange, and atomically installs its connection and verified sponsor relationship. Both devices retain independent identity and ratchet keys.

QR payloads use `sigil:link:v1:offer:`, `sigil:link:v1:proposal:` and `sigil:link:v1:response:` followed by bounded, lowercase hexadecimal bytes. The direct scans are the authentication channel: unsolicited codes, forwarded screenshots and network-delivered proposals cannot substitute for that trust assumption. This flow requires scans in both directions; it is not a single-scan rendezvous service.

`emoji_confirmation` maps the first 48 bits of the transcript confirmation digest to eight symbols from the fixed 64-entry alphabet in `client/src/link_exchange.rs`, six bits per symbol, most significant first. Emojis are an additional visual consistency check. **The short string alone must never establish trust.** The full, directly scanned exchange and exact confirmation digest are mandatory. UI work must display the proposed account and device context and obtain the approval consumed by the Rust confirmation methods.

## Encodings and cryptography

The public offer is exactly 184 bytes: `SGLO 00 01 00 00`, device ID, installation public identity, joining challenge, provisioning public key and credential commitment (32 bytes each), then creation and expiry (u64 big endian). It contains no credential or private key.

The transcript is exactly 216 bytes:

| Bytes | Field |
| --- | --- |
| 0–7 | `SGLT 00 01 00 00` |
| 8–39 | Sponsor binding fingerprint |
| 40–71 | Joining binding fingerprint |
| 72–103 | Sponsor challenge |
| 104–135 | Joining challenge |
| 136–167 | Joining provisioning public key |
| 168–199 | SHA-256 commitment to the joining transport credential |
| 200–207 | Creation, u64 big endian |
| 208–215 | Expiry, u64 big endian |

Fingerprints hash canonical unsigned bindings, including server, username, account, device and encryption identity. Validation requires matching account/address context, distinct devices and identities, distinct nonzero challenges, nonzero transcript fields and a lifetime of at most ten minutes. The provisioning key must differ from both identity keys. New approval rejects times before creation and at/after expiry.

Confirmation is SHA-256 of `Sigil/device-link-confirmation/v0` followed by the transcript. Each consent signs `Sigil/device-link-consent/v0`, one role byte (1 sponsor, 0 joining), and the transcript using XEdDSA. Shared `sigil_crypto::link` validation checks both binding signatures and both consents against the independently trusted sponsor fingerprint.

Provisioning uses separate ephemeral X25519 keys, HKDF-SHA-256 with the confirmation digest as salt and `Sigil/link-provisioning/v0` as info, and the existing AES-256-GCM-SIV storage codec. AEAD associated data binds `Sigil/link-frame/v0`, the confirmation digest and frame role (0 proposal, 1 response). A frame contains `SGLF 00 01 00 00`, sender provisioning public key, digest and ciphertext. Maximum plaintext is 2,048 bytes; maximum frame is 2,156 bytes. Invalid/low-order public keys, wrong keys, role/context changes and damaged ciphertext fail closed.

This provisioning channel is classical. It carries public bindings and consent signatures, **not identity private keys, live ratchets, history keys or transport credentials**. Linked devices establish fresh PQXDH/Triple Ratchet messaging sessions after authorization. History transfer and general multi-device event synchronization remain separate milestones.

A complete proof has an exact bounded encoding: `SGLP 00 01 00 00`, transcript, two u16-length-prefixed signed bindings and two 64-byte signatures, at most 1,380 bytes. Parsing rejects truncation, malformed fields and trailing bytes.


## Durable authorization

Freeze offers, challenges, encrypted frames and signatures before exposure. An attempt cannot change fields or extend expiry. The joining installation pins one proposal. Confirmation requires the full independently checked digest.

The sponsor persists its proof before HTTPS; receipt and verified peer commit atomically. Joining connection/binding/trust/completion also share a transaction. Exact retries can recover already committed authorization after expiry but cannot create a late grant or revive revoked credentials.

`POST /client/v0/device-links` requires a live sponsor, matching published binding, both consents, account scope, distinct target keys/device and one-use challenges. `GET /client/v0/device-link` retrieves the joining proof. `DELETE /client/v0/device-links/{challenge}` durably cancels the sponsor-scoped challenge and revokes an already created device. Cancellation wins either ordering against authorization.

Link/cancellation proofs remain within storage quota; 256 active devices per account is not a lifetime link limit. Restore retains proofs while revoking credentials. Sponsor cancellation blocks local work before retrying remote revocation; joining cancellation cannot retract a signature already sent.

A contact may accept an endorsement only from its already verified exact sponsor. Server inventory never transfers trust. Completion/cancellation removes provisioning secrets from live storage; [physical erasure limits](Security.md#erasure) still apply.
