# Device linking, profile 1

The backend and shared Rust client implement an experimental, durable linking flow. This is a Sigil protocol, not Signal interoperability or an independently audited protocol. The native capability is `device_link: [1]`; production camera, QR rendering and confirmation screens remain part of the paused UI work.

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

## Persistence and retries

Native schema 42 retains the sealed link journal within the configurable database-page budget; there is no 256-record lifetime limit. Records remain bound to the installation and record purpose, including cancelled attempts and consent tombstones. Each sealed record is bounded to 32,768 bytes. Exhausted storage rejects new state without deleting replay evidence; raising the budget does not reset identities. Valid old client database snapshots remain outside universal rollback detection guarantees. See [SessionsDevices.md](SessionsDevices.md) for storage and turnover acceptance.

Offers, prospective bindings, challenges, exact encrypted QR frames and randomized signatures commit before exposure. Reusing an attempt with changed fields fails. Expiry cannot be renewed by retry. The joining installation pins one proposal per attempt. Confirmation methods require the exact independently checked digest.

The sponsor saves its proof before the network request, then commits the server receipt and verified joining peer atomically. The joining installation commits its connection, own signed binding, verified sponsor and completion state atomically. Lost local commits after server success recover using the same proof and credential, including after the original approval deadline: this recovers an already committed authorization and does not create a late grant. Completed cached receipts report past acceptance, not current authorization.

Provisioning secrets are logically removed after completion or cancellation. SQLite/WAL/filesystem snapshots can retain old ciphertext; this does not establish physical key erasure.

## Server authorization and cancellation

Server schema 12 implements:

- `POST /client/v0/device-links`: requires a live, enabled-account sponsor, its published matching signed binding and both valid consents. Checks account scope, new target identity/device, credential uniqueness, one-use sponsor/joining challenges and the 256-active-device account bound. Revoked/expired devices use storage rather than active slots. Device authorization, identity, binding, proof ledger and storage reservation commit together.
- `GET /client/v0/device-link`: returns the authenticated joining device's proof.
- `DELETE /client/v0/device-links/{challenge}`: durably cancels a sponsor-scoped challenge and revokes any device already created by it. Transactional ordering makes cancellation win either side of a race with authorization. Cancelled challenges remain retained within the account storage quota, without a 256-attempt lifetime limit.

Exact authorization retries return the existing live target without renewing expiry, changing its credential or reviving revocation. Restore retains proof/cancellation ledgers while revoking credentials, so replay cannot revive a pre-restore grant. Routes retain native-only Origin policy, request bounds and existing authentication/rate limits.

`cancel_sponsored_link_online` commits local cancellation, removes its provisioning secret and blocks any known child peer before attempting remote cancellation. An ambiguous network failure requires retry; local intent survives restart. Joining-side cancellation stops local completion and erases its pending secret, but cannot retract a signature already transmitted; remote grant cancellation belongs to the authenticated sponsor.

`accept_linked_peer` permits a contact to accept a live, complete endorsement only from an already verified exact sponsor binding. An expired endorsement first encountered by an offline contact requires fresh endorsement or independent fingerprint verification. Server inventory alone never supplies encryption trust. Distributed device-roster synchronization and general fan-out remain separate work.

## Acceptance

Regression tests cover independent keys, three-step QR exchange, restart after every step, exact bytes, changed/malformed/expired proposals, incorrect confirmations, cross-attempt substitution, AEAD tampering, transaction rollback, ambiguous server success, fresh encrypted messages in both directions, forged proofs, wrong accounts, exact retry after expiry, revocation, cancellation/authorization races and server restore. These are implementation acceptance tests, not an independent security audit.
