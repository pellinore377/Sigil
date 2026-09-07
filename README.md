# Sigil

Self-hostable messaging under development. Rust owns the server, protocol and native client logic; Compose remains the provisional UI framework. The shared sessions/devices milestone is complete; the rest of the backend and production UI remain unfinished. Use synthetic data only: independent security review and full production acceptance remain pending.

Current backend inventory: **one complete, nine partial, two absent**. [Implemented behavior, fixed remaining scope and evidence](docs/BackendProgress.md#current-inventory--2026-09-07).

## Build and run

Linux server; Rust 1.98. Dependencies are pinned in Cargo.lock. The server builds independently of Compose and uses the shared crypto crate to verify device-link proofs.

```sh
cargo test --locked --workspace
cargo test --locked --release -p sigil-client --lib
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTFLAGS="--remap-path-prefix=$HOME=/build" cargo build --locked --release -p sigil-server
export SIGIL_DATA_DIR=/path/outside/repository/sigil-data
target/release/sigil-server serve
```

The server creates a private 0700 directory and 0600 database/credential files. Existing non-private directories are refused. It binds `127.0.0.1:8080` by default; `SIGIL_LISTEN` overrides the address. Non-loopback HTTP requires a trusted TLS reverse proxy. TLS termination and a browser Admin UI are not implemented in the server.

`admin.token` contains a random installation credential. Supply it as `Authorization: Bearer <token>` on Admin requests. Keep credentials out of URLs, logs, shell history and the repository. Admin authorization does not confer a client encryption identity or recovery key. Browser Origin requests are currently refused.

## Implemented foundations

| Component | Current behavior |
| --- | --- |
| Admin | Revision-checked configuration, immutable homeserver name, invitations, account reauthorization, offline backup/restore and credential rotation. |
| Device authorization | Hashed expiring credentials, rotation/revocation, exact retries and transactional replacement devices without inherited encryption identity or contact grants. |
| Peer verification | Signed device bindings, explicit fingerprint confirmation, key-change quarantine and blocking enforced by peer-bound session checkpoints. |
| Sessions and devices | Automatic authenticated selection/convergence, durable lost-session retry, seven-day inactivity retirement policy, direct QR/emoji linking with one-use authorization, and explicit fingerprint-approved replacement. See [milestone acceptance](docs/SessionsDevices.md). |
| Delivery | Opaque durable mailbox, one-use public prekeys, recipient-controlled admission, contact invitations, bounded queues, quotas, retries and background expiry. |
| Cryptography | Experimental PQXDH profile 2 and Triple Ratchet using X25519, ML-KEM-1024 Braid, sparse post-quantum chains and AES-256-GCM-SIV. Complete headers are authenticated; native sessions reject classical fallback. |
| Native persistence | Encrypted identity/prekey slots, durable publication/claim retries, bounded private-key retirement, atomic message transactions and durable receipts. Explicit session retirement frees active peer slots while retaining history and blocking ID reuse. |
| Native HTTPS | Certificate-verified transport, durable enrollment/credential rotation, exact outbox retries and incremental recovery transfer. Redirects, proxies and unbounded DNS workers are disabled. |
| Online work | Bounded sync covers receive/acknowledgement, prekey supply, recovery, outbound packets and guarded key maintenance. A scheduled entry point persists retry deadlines and backoff; platform wakeups and lifecycle integration remain pending. |
| Text/file events | Versioned encrypted events bind logical message IDs, conversations and both device fingerprints. Bounded incoming routing, durable acknowledgements and cyclic mailbox scans survive interrupted work. |
| Groups | Membership policy, encrypted ordering/fork journals, Sender Keys, same-server text/file delivery and an optional private authority with durable native submission/sync. Group-scoped channels, invitation/admin relay, distribution recovery, earlier-history sharing and federation integration remain open. |
| Attachment transfers | Immutable encrypted chunks, resumable upload/download, separate access capabilities, server quotas/cleanup, private native cache, durable worker scheduling, authenticated event/recovery handoffs and explicit restore blocking. Complete retention, formats and previews remain open. |
| Android key protection | Hardware-backed Android Keystore wraps an independent Rust database key in private, non-backed-up storage. Missing or altered keys fail without resetting the database. |
| History recovery | Configured ordinary text and file descriptors are archived atomically with message state. Independently keyed records, pages and chained manifests support durable upload snapshots, verified staged import, monotonic revisions and deletion markers. Live sessions are never imported. |

`GET /healthz` reports liveness. `/readyz` reports configured control-plane availability, not messaging readiness. `/versions` advertises implemented experimental transport capabilities separately; `encrypted_event`, `federation` and end-user `recovery` remain empty. `recovery_storage: [0]` and `attachment_storage: [0]` advertise only their opaque storage APIs.

Admin configuration uses `GET`/`PUT /admin/v0/configuration`. A first request can use:

```json
{"expected_revision":0,"settings":{"server_name":"chat.example","default_quota_bytes":10737418240,"max_attachment_bytes":1073741824}}
```

Stale revisions return 409. Account quota covers queued mailbox payloads, recovery ciphertext, reserved/stored attachment chunks and retained protocol-record storage reservations. No automatic history eviction occurs when quota is exhausted. Full API behavior and historical implementation decisions are in [the plan](docs/plan.md); recovery routes and limits are in [Recovery.md](docs/Recovery.md), and file-transfer behavior is in [Attachments.md](docs/Attachments.md).

## Storage and operational limits

Server schema 19 and native client schema 55 migrate earlier supported schemas transactionally. Session/device ledgers use storage budgets instead of fixed lifetime event counters; the native default is a 1 GiB database-page budget, adjustable through `open_with_storage_limit`. See [the storage and acceptance contract](docs/SessionsDevices.md). Legacy classical sessions remain readable as history but require fresh handshakes before sending. Earlier raw and version-1 initial packets are preserved but cannot be retransmitted under the current native envelope. Android now supplies hardware-backed wrapping; other platforms remain pending. Metadata and sizes remain visible. SQLite deletion is not guaranteed physical erasure, and valid old database snapshots cannot be detected universally: **never restore a live client database as a history backup**.

Requests have route-specific body bounds, a five-second handler deadline, 32 active-handler slots and a separate bounded database worker. Client requests share a process-wide rate budget; authenticated devices also have per-device read/write budgets. Rate rejection supplies `Retry-After`. Limits require workload testing and trusted proxy protection before public exposure. No forwarding headers are treated as authenticated source identity.

Stop the server before offline maintenance; an exclusive directory lock prevents concurrent maintenance:

```sh
target/release/sigil-server backup /path/outside/repository/server.db
target/release/sigil-server rotate-admin-token
SIGIL_DATA_DIR=/path/outside/repository/new-private-directory \
  target/release/sigil-server restore /path/outside/repository/server.db
```

Operator backups include configuration, account/routing metadata, credential hashes and stored ciphertext. They exclude the Admin credential and client-held keys. Restore validates and stages a private database, revokes device credentials/invitations, clears prekeys and queued delivery, and generates a new Admin credential. Encrypted recovery objects survive, but their head is explicitly flagged as a restored checkpoint. The native importer refuses that potentially stale snapshot. A surviving archive can repair only an exact match to its trusted checkpoint, locally prepared direct successor, or an ancestor proven by its bounded authenticated manifest ledger, with an atomic history-only handoff into a freshly reauthorized client and a new snapshot preserving local edits/tombstones. Interrupted direct-successor uploads retain their exact ciphertext and re-upload all objects after repair authorization. Unproven or conflicting checkpoints and pending imports remain blocked; see [Recovery.md](docs/Recovery.md). Restore refuses existing destinations and unsupported schemas. Startup, backup and restore reject triggers and views that could alter these operations; native client startup rejects them too. Both stores enable SQLite defensive mode.

## Container

```sh
docker compose up --build -d
```

The Linux amd64 image uses pinned Rust/Debian digests. Compose binds loopback, runs as non-root, drops capabilities and uses a read-only root filesystem with persistent private storage. Health checks call the Rust binary. Do not publish or expose this as a finished messaging service.

See [Deployment.md](docs/Deployment.md) for the reproducible container acceptance check, offline backup, fresh-volume restore, upgrade procedure and remaining deployment gates. Backups are standalone files that can be restored from read-only storage; existing database files with shared permissions are rejected.

## Verification and remaining gates

The optimized workspace passes 568 tests, with five skipped. This includes 271 native client tests (214 library and 57 integration) and 97 crypto tests. Clippy with warnings denied and formatting checks pass. Coverage includes restart/concurrent writers, injected transaction failures, ratchet delivery schedules, real HTTPS group/attachment transfers, private authority issuance/ordering, queued epoch cutover and fresh-client recovery. The preceding debug workspace passed 564 tests, with nine skipped; optimized acceptance additionally exercises four bulk client cases. The earlier explicit 1 GiB cache acceptance remains unchanged. Earlier Android arm64 checks passed 74 crypto and 84 native client tests plus five on-device keystore tests; the new group/media/push/SigilText work has not repeated that device acceptance. The last Docker image passed persistent-volume SIGKILL/restart, exact retry, backup locking, restore revocation, attachment access/repair/deletion and push capability reset; it predates the new credentials and authority service. Independent C/OpenSSL/libsodium checks validate handshake, ratchet, Sender Keys, attachment and private authentication credential fixtures; these are not upstream Signal interoperability vectors or a full independent security audit. See [the implementation ledger](docs/BackendProgress.md) for exact runs and limitations.

Backend item 1 (sessions and devices) has passed its fixed server/shared-Rust implementation acceptance. Item 2 remains open despite the optional same-server authority and native adapter: invitation/admin relay, group-scoped trust/channels, distribution recovery, earlier-history sharing and federation/privacy composition remain unfinished. See [group progress](docs/Groups.md), including the administrator-only opaque update policy and metadata limits. Platform lifecycle integration remains separate. See [review findings](docs/SessionsDevicesReview.md) and [acceptance status](docs/SessionsDevices.md). Recovery backfill/onboarding, durable key erasure, other platform key adapters, complete retention/media handling, group/federation design, independent protocol/security review and product acceptance remain open. Old WAL checkpoints can remain decryptable after compromise of the current client storage key; logical retirement does not establish forensic erasure. The blocking HTTPS adapter requires a dedicated worker. A fresh device with only its recovery secret cannot detect a malicious server replaying an older valid archive without an independent trusted checkpoint.

Text-event binding: [Events.md](docs/Events.md). Group/federation analysis: [GroupsFederation.md](docs/GroupsFederation.md). Exact suite choices and specification interpretations: [TripleRatchet.md](docs/TripleRatchet.md). Product scope and gates: [plan.md](docs/plan.md). Visual direction: [Design.md](docs/Design.md). Dependency notices: [server](licenses/Server-ThirdParty.txt), [cryptography](licenses/Crypto-ThirdParty.txt), [native client](licenses/Client-ThirdParty.txt), [SigilText](licenses/Text-ThirdParty.txt), [NIST vectors](licenses/NIST-Vectors.txt). Licensing and branding remain unselected. The native server dependency path permits MIT-compatible distribution with required notices; that does not determine the complete client's eventual license.
