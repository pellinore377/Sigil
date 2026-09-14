# Device linking, profile 1

## Trust and user interaction

Linking needs one physical scan and approval on the existing device:

1. The joining installation displays a QR containing its offer, canonical server, relay ID, phone capability and a fresh 32-byte secret. The existing device scans it directly.
2. The devices exchange their existing encrypted proposal and signed response through HTTPS relay slots, additionally authenticated with the QR secret. The joining device displays one emoji; the existing device displays six choices.
3. The user selects the matching emoji on the existing device. It submits the complete signed proof; the joining installation retrieves and verifies it, installs its connection, and signs in automatically. The computer needs no camera.

The QR uses `sigil:link:v1:relay:` followed by bounded JSON. The secret never reaches the relay server. Forwarded screenshots and unsolicited codes must not replace scanning the intended device. The internal offer/proposal/response encodings remain unchanged.

The displayed emoji is the first symbol from `emoji_confirmation`; the five distinct alternatives and their order are transcript-derived. This is supplementary user confirmation, not a six-choice cryptographic authentication mechanism. Trust requires the QR secret, authenticated full transcript and signed proof. Joining consent is generated only after verifying the QR-secret-authenticated proposal; sponsor consent requires selecting the matching emoji.

Existing accepted contacts bootstrap through same-account encrypted conversation synchronization. A dedicated private scope carries bounded, hashed contact snapshots in existing `UiSetting` fragments, preserving compatibility with older clients. Only complete snapshots import signed trust anchors; independent-verification status is preserved, not inferred. Existing contact decisions and conflicting/blocked peer records are not overwritten. Catalog state is excluded from recovery archives and forwarded only by its authoring device; imported contacts can publish fresh snapshots from their own live trust store.

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


## Relay

`POST /client/v0/link-relay/create` reserves two role-separated capabilities; only their SHA-256 hashes are stored. `POST /client/v0/link-relay/exchange` writes the caller’s immutable slot and reads the opposite slot. Exact retries are idempotent; replacement fails. Either capability may cancel the relay. Creation uses enrollment throttling; all requests use global limits and browser-origin checks. At most 256 reservations live for ten minutes, with two packets of at most 10,000 hexadecimal characters each. Expiry and restore clear reservations.

Packets use AES-256-GCM-SIV under the QR secret, with associated data `Sigil/device-link-relay/v1/<server>/<id>/<proposal|response>`. The relay can delay or drop setup but cannot substitute a binding without the QR secret. Client storage encrypts relay state and freezes packet ciphertext before transmission. Completion removes the client relay secret. Relay cancellation alone does not retract signed consent; durable cancellation below handles that case.

## Durable authorization

Freeze offers, challenges, encrypted frames and signatures before exposure. An attempt cannot change fields or extend expiry. The joining installation pins one proposal. Confirmation requires the full independently checked digest.

The sponsor persists its proof before HTTPS; receipt and verified peer commit atomically. Joining connection/binding/trust/completion also share a transaction. Exact retries can recover already committed authorization after expiry but cannot create a late grant or revive revoked credentials.

`POST /client/v0/device-links` requires a live sponsor, matching published binding, both consents, account scope, distinct target keys/device and one-use challenges. `GET /client/v0/device-link` retrieves the joining proof. `DELETE /client/v0/device-links/{challenge}` durably cancels the sponsor-scoped challenge and revokes an already created device. Cancellation wins either ordering against authorization.

Link/cancellation proofs remain within storage quota; 256 active devices per account is not a lifetime link limit. Restore retains proofs while revoking credentials. Sponsor cancellation blocks local work before retrying remote revocation; joining cancellation cannot retract a signature already sent.

A contact may accept an endorsement only from its already verified exact sponsor. Server inventory never transfers trust. Completion/cancellation removes provisioning secrets from live storage; [physical erasure limits](Security.md#erasure) still apply.
