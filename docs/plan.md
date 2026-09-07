# Sigil: unified implementation plan

Current backend status (2026-09-06): **item 1, sessions/devices, has passed its fixed implementation acceptance**. R1/R2/L1–L4 and V1 are closed in [SessionsDevices.md](SessionsDevices.md). Backend items 2–12 remain; production UI stays paused and the independent audit follows backend implementation completion. Historical implementation entries below retain their original results.

## 1. Outcome and project rules

Build a self-hostable E2EE messenger with a polished client, rich SigilText content, reliable recovery, integrated calling, and a first-class Admin management experience. Complete the agreed feature set on Android first, then implement Web → iOS → Linux → Windows → macOS.

The everyday experience is one account on one homeserver, with automatic communication across independent servers. Operators manage infrastructure redundancy; users do not manage multiple hosting identities.

Self-hosting must be UI-driven by default. A new operator should be able to install Sigil, open Admin, complete guided setup, configure the server, manage users/invitations/integrations, review health, and perform upgrades/backups/restores without hand-editing configuration files for normal operation.

### Development constraints

- Use Rust for the server, shared client logic, cryptographic protocols, SigilText, and suitable rendering/processing components.
- Use Compose Multiplatform as the leading provisional shared UI candidate, with Kotlin for UI and narrow platform adapters. Rust owns shared domain logic. Web accessibility and Apple validation remain open.
- Prefer established C/C++ components when suitable Rust implementations do not meet requirements. No Python application services.
- Use idiomatic Rust, default rustfmt, Clippy, explicit error handling, and bounded resource usage. Justify and document necessary unsafe.
- Comments must be extremely terse and necessary. Maintain one concise README; add only required license notices and existing specification material.
- Start in one private monorepo with independently buildable server, shared-core, client, and platform packages.
- Project licensing remains unselected. As clarified on 2026-09-07, an MIT-compatible server is no longer mandatory. Reserve official branding separately.
- Review exact dependency versions, enabled features, native libraries, assets, and transitive licenses. Do not import, vendor, copy, or port Signal implementation code, including through wrappers or forks. Implement published specifications independently using established non-Signal primitives. Relaxed licensing does not authorize Signal code or automatically approve other strong-copyleft dependencies.

### Personal-data prohibition

Real conversations, usernames, domains, credentials, device identifiers, screenshots, and device-derived logs must never enter the project, fixtures, commits, documentation, or build artifacts. Use synthetic fixtures and reserved example domains. Keep necessary test-device configuration outside the project; collect only deliberately selected, non-identifying measurements.

## 2. Architecture and security boundaries

### Accounts, discovery, and devices

- One homeserver account, addressed as @user:server; usernames are unique within that server.
- Separate account addresses, display names, stable internal identity references, and client-controlled encryption keys.
- Support invitation-based enrollment and optional standard OIDC authentication, including Pocket ID. Neither OIDC nor server administrators can decrypt history or silently inherit an existing verified encryption identity.
- Allow exact-address discovery by default, with a user opt-out. No public browsable directory or partial-name search.
- Resolve ordinary mentions from known contacts and current conversation participants. Duplicate display names require disambiguation; copied names/images never transfer verification.
- Link devices using an authenticated QR exchange and transcript-bound emoji confirmation. Each device has independent credentials and session state.
- Provide device inspection, revocation, contact verification, key-change warnings, blocking, invitation revocation, and contact-request controls.
- Defer account migration. Preserve identity/address separation and versioned authenticated updates so migration can be added without rewriting conversation references.

### Messaging, groups, and federation

- Implement our own Rust protocol code following pinned Signal Triple Ratchet and associated session-establishment specifications, using established cryptographic primitives. Do not invent replacement cryptography.
- Use Signal’s private-group and Sender Keys designs as references for group messaging, administration, and device distribution.
- Treat their adaptation to federation as a security-design milestone. Do not claim pairwise Triple Ratchet guarantees automatically apply to groups.
- Establish authenticated group membership transitions, device changes, key rotation, and concurrent administrative-update handling before implementing shared group state.
- Federate authenticated, bounded encrypted deliveries. Users never need accounts on their contacts’ servers.
- Servers persist encrypted delivery/synchronization data and encrypted attachments; devices maintain encrypted local history and search indexes.
- Keep conversation semantics—names, replies, threads, group settings, receipts, and structured actions—inside encrypted content wherever the protocol permits.
- Version client-server, federation, encrypted-event, and backup interfaces independently. Define compatibility and downgrade rejection before freezing version 1.
- Implement retry-safe delivery, duplicate suppression, replay protection, offline queues, ordered local presentation, and reconciliation after interruption.

### Recovery and retained history

- Offer automatic encrypted history recovery during onboarding, protected by a separately generated recovery key.
- A fresh device can restore using account access plus recovery material, without the original phone.
- Restore conversation structure and text first; fetch media progressively. Show recovery progress, last successful checkpoint, and unprotected pending uploads.
- Never blindly restore and reuse old live ratchet state. Establish fresh device/session credentials.
- Retain local history until deletion or expiration. Retain remote encrypted recovery history and media according to disclosed account retention settings.
- Default account storage quota: 10 GiB, configurable by the operator. Warn before exhaustion and reject new storage cleanly; do not silently delete old history.
- Exclude view-once and disappearing content from recovery archives initially. Track deletions to prevent routine restoration from resurrecting removed content.
- Explain that recovery-key compromise exposes retained backup history, and recipient copies cannot be forcibly erased.

### Metadata objective

Long-term objective: eliminate or substantially reduce observable communication relationships, not merely encrypt message text.

Version 1 must document remaining account, discovery, routing, traffic, push, attachment-access, and call metadata. No “100% blind” claim.

Maintain a research milestone for PIR, traffic shaping, padding, and other metadata-protection techniques. Evaluate them against malicious servers with observation enabled. Adopt stronger mechanisms only when their complete guarantees, costs, and mobile behavior are demonstrated.

## 3. Client behavior and feature scope

### Conversations and organization

- Distinct direct conversations, named groups, and Note to Self.
- Groups support members/admins, invitations, removal, permission controls, and administrator changes.
- Replies, threads, reactions, edits, deletion, disappearing messages, voice messages, and delivery/read/failure states.
- Timeline timestamps, encryption/trust details, configurable typing indicators and contact-limited status/presence.
- Conversation/global search performed locally; conversation settings, shared message pins, and private conversation pins.
- Collections are off by default, enabled in settings. Their names and membership synchronize encrypted across linked devices.
- Long-press actions: pin/unpin, mark read/unread, snooze, and leave group; direct chats provide delete and block actions.
- Drafts synchronize encrypted. Preserve concurrent conflicting drafts rather than silently overwriting them.
- Snooze presets: one hour, eight hours, and until the next local morning. Incoming-call controls are separate.
- Read receipts and typing indicators start enabled and are independently configurable. No public last-seen directory.

### Configurable group history

Default new members to history from joining. Admins can enable sharing retained earlier history.

Implement earlier-history access as an authenticated, encrypted transfer from an authorized existing device, with pending/unavailable states when no supplying device is available. Do not distribute old live ratchet state. Exclude expired, view-once, and deleted content; disclosure cannot subsequently be revoked.

### SigilText

Implement the complete current SigilText specification through Android milestones:

- Markdown, modifiers, colors, gradients, reveal/redaction effects, and parameterized animations.
- Checklists, recurring lists, tasks, polls, reminders, notes, timers, and countdowns.
- Charts, diagrams, tables, recipes, mathematics, calculations, conversions, randomizers, display helpers, ASCII art, QR codes, and service-backed cards.
- Live formatted composition, context formatting, completion, graphical builders, round-trip editing, and integrated help.
- One canonical semantic model shared by typed syntax and graphical creation.
- NFC normalization and grapheme-safe effect ranges; redaction occurs before any transmitted representation, search text, or fallback is produced.
- Compact inline cards, expanded viewers, accessibility, theme safety, reduced motion, and deterministic animation replay.
- Notes remain a per-conversation view of notes, checklists, and reminders, separate from pinned messages and Note to Self.

Independent checklist-item edits merge; same-item conflicts resolve deterministically with attribution. Recurrence settings require authorized edits. Poll ballots are per identity and editable until closure.

Closed-result polls and task undo deadlines are initially application behavior, not guarantees against malicious modified clients. Do not claim hidden results are cryptographically inaccessible or client timestamps prove a deadline.

### Attachments, maps, and services

- Attachment panel includes camera, gallery, files, stickers, GIFs, emoji, polls, formatting, location, and categorized SigilText tools.
- Support arbitrary file transfer up to a configurable 1 GiB default, with encrypted resumable chunks, progress, cancellation, and retry.
- Every attachment gets a useful bubble; supported formats receive thumbnails and expanded in-app viewing.
- Cover the requested text, data, PDF, spreadsheet, document, presentation, STL/3MF, vCard, image, audio, video, and GIF families.
- Implement a format capability registry. Distinguish full rendering, simplified previews, and download-only fallback; never silently describe incomplete support as full fidelity.
- Prefer PDFium through Rust bindings for initial PDF evaluation; compare hayro where useful. Evaluate calamine for spreadsheet data and Symphonia for audio.
- DOCX/PPTX and uncommon codecs require explicit implementation feasibility checks. A parser alone does not satisfy the viewer requirement.
- Authenticate attachment data before processing. Use isolated preview workers without key access or arbitrary networking, bounded memory/time/decompression, and no macros or document scripts.
- Add local malware scanning only where operationally practical. Never upload files or hashes automatically for scanning; never label a file guaranteed safe.
- Use PMTiles with an evaluation of embedded martin-core. Serve styles, fonts, and sprites locally. No Postgres requirement for file-backed maps.
- Support one-time location, dropped pins, and live sharing for 15 minutes, one hour, or eight hours, with explicit stop controls.
- External-service features require explicit provider configuration and clear query disclosure. Received cards render from encrypted snapshots without repeating recipient-side queries.
- Support self-hosted endpoints. Do not require a GIPHY integration incompatible with the desired delivery/privacy model.

### Calls and notifications

- Distinct audio/video entry buttons backed by one call session capable of adding video.
- Add people to a call without adding them to a conversation.
- Include group calls, screen sharing, device/audio routing, reconnects, and background incoming-call handling.
- Compare str0m and webrtc-rs for the Rust forwarding service; use established client media processing and hardware codecs where needed.
- Encrypt media end to end through the forwarding service. Transport encryption alone is insufficient.
- Send FCM only generic wake-up data without message, sender, or conversation content. Construct notifications locally after fetching and decrypting events.
- Offer UnifiedPush support; account for distributor setup and platform delivery limitations.

### Admin management

The Admin UI is the integrated operator-facing management surface for Sigil. It is part of the product, not an afterthought.

- First-run setup wizard: server name/domain, administrator account, storage paths/quotas, HTTPS/reverse-proxy checks, push options, OIDC, external/self-hosted providers, and optional maps/media settings.
- Prefer safe defaults and automatic validation. Normal setup must not require editing TOML/YAML/JSON by hand.
- Dashboard: service health, version, federation reachability, storage usage, backup status, queue/failure summaries, push/provider status, and upgrade availability.
- User/admin controls: invitations, account status, role/permission management, device/session visibility, blocking/abuse controls, and configurable registration policy.
- Federation controls: allow/deny policy where supported, peer diagnostics, retry/error visibility, and clear distinction between local faults and remote-server faults.
- Storage/recovery controls: quotas, retention policy, backup destination/status, restore workflow, media usage, and warnings before exhaustion.
- Integration controls: OIDC, FCM, UnifiedPush guidance, service-backed card providers, maps, and other optional integrations from one UI.
- Maintenance: guided upgrades, migration checks, backup-before-upgrade, rollback guidance where feasible, logs/diagnostics export with secrets and personal data redacted by default.
- Security-sensitive changes require explicit confirmation and explain their effect before applying.
- Expose advanced/raw configuration only behind an expert view. UI changes and file/API configuration must round-trip to one canonical server configuration model.
- The Admin UI must work well on desktop and mobile web so a small self-hoster can administer Sigil without SSH for routine tasks.

## 4. Implementation milestones and decision gates

| Milestone | Deliverable and completion condition |
|---|---|
| A. UI feasibility and concepts | Run the agreed two-hour first-pass Slint/Compose comparison, counting setup. Use identical synthetic content and release builds on Android. Check styled editing, emoji/RTL/IME behavior, interactive cards, gestures, accessibility, and a 1,000-message timeline. Also prototype the Admin setup flow as a web surface. Record surface-integration checks that cannot fit as unresolved. |
| B. Visual selection | Produce three Android concepts: Material 3 Expressive, a restrained minimal direction, and a distinct content-focused direction. Each shows inbox, rich conversation, attachment creation, and calling; demonstrate optional collections. User selects the direction before production UI styling. |
| C. Dependency and protocol foundations | Pin feasible, license-compatible components; define wire/event/recovery contracts and the security model. Complete group/federation analysis, cryptographic vectors, and core misuse tests. No real-message pilot before this gate. |
| D. Reliable Android messaging | Enrollment/login, discovery, device linking, DMs, groups, Note to Self, cross-server delivery, offline synchronization, notifications, encrypted local storage, and recovery from a lost-phone scenario. |
| E. Conversation and SigilText experience | Complete conversation controls and all SigilText constructs, builders, renderers, Notes, search, organization, shared-state behavior, and accessibility. |
| F. Files, maps, and service cards | Implement the requested attachment matrix, secure expanded viewers, resumable media, PMTiles maps, location sharing, and configured external/self-hosted providers. Unsupported required formats remain blockers requiring a scope decision. |
| G. Calling | Complete audio/video transitions, independent participant invitations, group calls, screen sharing, reconnects, encrypted forwarding, and network-condition testing. |
| H. Android acceptance and deployment | Full feature acceptance, recovery and fault testing, resource/performance checks, dependency review, independent security assessment, and a single-container installation/upgrade/restore test. Admin must complete fresh setup, routine configuration, user/integration management, health review, upgrade, backup, and restore without hand-editing configuration for the normal path. |
| I. Remaining clients | Implement Web, iOS, Linux, Windows, and macOS in that order, reusing shared logic and UI where validated. Each platform passes its own input, accessibility, security, and media checks. |

### Selection rules

- Compose Multiplatform is the selected provisional candidate after evaluation. An unresolved critical check is not a pass; keep web accessibility and Mac/iPhone validation open.
- Prefer str0m for the forwarding server if it passes the shared interoperability and reliability tests; select webrtc-rs if it resolves demonstrated str0m integration deficiencies. If both fail, stop at the media gate.
- Prefer embedded Martin functionality when bounded serving and integration tests pass. Evaluate packaging an internal tile-serving process in the same container only if embedding fails.
- Do not start independent cryptography, Office-rendering, codec, or mapping-engine rewrites to bypass a failed dependency evaluation without a separate decision.

## 5. Validation, deployment, and future work

### Initial performance benchmark

Use synthetic workloads on a reference 4-core/8-GiB server, with 50 enrolled users and 20 connected devices. Measure both same-server and cross-server delivery; record network conditions and test-device characteristics without storing identifying details.

Targets under a healthy, controlled network:

- Local send feedback within 100 ms.
- Online recipient-visible text delivery below 500 ms p95, with approximately 50-ms client-to-host RTTs and 50-ms inter-host RTT.
- Call media established within two seconds p95 after acceptance.
- Validate groups of 256 members and calls of eight participants.
- Responsive search and virtualized scrolling with 100,000 synthetic messages.
- Text delivery retains priority during bulk transfers. Report idle/busy battery, memory, and bandwidth measurements; do not invent battery guarantees before device measurements.

### Security and failure tests

- Cryptographic vectors, malformed inputs, replay/reordering, tampering, crash/rollback recovery, revoked devices, username reassignment, and attempted identity substitution.
- Parser/protocol fuzzing, oversized structured cards, decompression bombs, hostile documents, worker crashes, and unsafe external URLs.
- Concurrent group changes, checklist edits, poll closure, offline history sharing, expiration, and multi-device conflicts.
- Fresh-device restoration without the original phone, incorrect recovery material, incomplete backups, deleted-content handling, and storage exhaustion.
- Interrupted uploads, process/container restarts, federation outages, packet loss, changing networks, denied permissions, and delayed pushes.
- Accessibility, large text, RTL, keyboard/gesture conflicts, reduced motion, and screenshot comparisons using synthetic content only.
- Metadata inventory based on observable traffic and server behavior, not absence of logs.
- Independent review before presenting Sigil as production-grade secure messaging.

### Deployment

Ship one Docker image and Compose example suitable for Dockge, with persistent data storage, optional read-only maps, health checks, migration handling, and operator backup/restore instructions. OIDC and external providers remain optional integrations. Document required HTTPS/media connectivity and support reverse-proxy/VPS deployments without embedding personal configuration.

Normal deployment and administration is driven through Admin after the container starts. The documented happy path is: start container → open setup URL → complete guided configuration → operate from the admin UI. Hand-editing configuration remains an expert/debug path, not the expected installation experience.

### Beyond 1.0

Preserve versioned content and authenticated-action interfaces for turn-based games, collaborative content, and optional on-device reply suggestions/transcription. Do not ship a downloaded executable-plugin system initially.

Keep metadata-resistant transport and authenticated account migration as explicit future work. Neither is required for the initial account experience, and neither is represented as already solved.

## 6. Backend foundation decisions — implementation in progress

This section records the current implementation boundaries. Milestone C is **not complete**. UI implementation is paused until the backend/protocol foundations are ready.

### Implemented control plane

The Rust workspace separates `sigil-server`, shared `sigil-protocol` types, and the existing `sigil-core`. The server has no dependency on client UI or editor code. Admin v0 implements installation configuration with immutable homeserver naming, revision-checked updates, private local credentials, SQLite migrations, and offline configuration backup/restore. Readiness reports the control-plane scope explicitly. Message/federation/recovery version sets remain empty; unsupported route versions are rejected, never silently downgraded.

The local installation credential grants operator configuration access. It confers no client encryption identity. It is deliberately not used as a placeholder for user authentication. Configuration and account/device authorization metadata are stored; there are no plaintext message endpoints or fake encrypted envelopes.

### Trust boundaries for the next backend work

| Boundary | Required treatment before messaging is implemented |
| --- | --- |
| Device ↔ homeserver | Account/session authorization controls routing and storage. Client-held encryption identity is separate and cannot be inherited through administrator or OIDC access. |
| Homeserver ↔ peer | Authenticate origin and intended destination; bound deliveries and retries; account for malicious peers, replay, request smuggling, and outbound discovery/SSRF. TLS alone does not authenticate encrypted-event authorship. |
| Device ↔ device | Verify identity-bound session establishment and authenticated events; detect substitution, malformed keys, unsupported suites, and replay. An address lookup is not contact verification. |
| Device ↔ recovery store | Encrypt retained history with independent recovery material; authenticate manifests and deletion state; restore into fresh device/session credentials. Never roll back live ratchet state. |
| Operator ↔ Admin | Administrative access must not expose content keys. Future browser sessions require an explicit origin/CSRF policy and credential lifecycle; the current local control-plane credential is not that design. |

Treat homeservers and relays as potentially malicious for message confidentiality and integrity. They can withhold, reorder, or replay delivery and observe routing/traffic metadata. Endpoint compromise and recipient copying are separate threats; encryption cannot promise availability or erase another participant's retained copy.

### Pinned upstream specification references

These are research inputs, not implemented cryptographic suites. PDF hashes identify the exact reviewed references; do not silently follow a changed upstream document.

| Reference | Revision | PDF SHA-256 |
| --- | --- | --- |
| [PQXDH](https://signal.org/docs/specifications/pqxdh/pqxdh.pdf) | Revision 3, last updated 2024-01-23 | `9fd0e02a5e13075b64adc7aa6dc9baade4f65af70b5571a332991756d98fe896` |
| [Double/Triple Ratchet](https://signal.org/docs/specifications/doubleratchet/doubleratchet.pdf) | Revision 4, 2025-11-04 | `1d9b4dc3c6440b0777d747ff42707fccba3a45d209a2bdc33d1ea816aa05990c` |
| [ML-KEM Braid](https://signal.org/docs/specifications/mlkembraid/mlkembraid.pdf) | Revision 1, last updated 2025-09-26 | `c38a3ab844c7c583e7be15ff714b07792220cb662b5d1e9590e46aa5909a3ee6` |

PQXDH requires explicit curve, signature, KEM, KDF, AEAD, and encoding choices; its XEdDSA signatures are not interchangeable with an arbitrary Ed25519 API. Its authentication properties must not be confused with post-quantum forward secrecy. Triple Ratchet composes classical and sparse post-quantum ratchets; the ML-KEM Braid specification supplies the referenced sparse key-agreement design. These observations are reasons to finish suite selection and test vectors before writing session code.

### Contracts to settle before version 1

- Account address, stable account reference, device reference, encryption identity, and transport session credential remain separate types. Define canonical encoding and reassignment/revocation semantics before exposing enrollment.
- Independently version Admin, client-server, federation, encrypted-event, and recovery formats. Bind negotiated cryptographic versions and algorithms into authenticated transcripts; never use a transport capability list as proof of authenticated negotiation.
- Outer delivery metadata must contain only required routing, bounded lengths, and retry/deduplication identifiers. Conversation titles, message bodies, receipts, and structured actions belong inside authenticated encryption. Exact wire encoding and envelope limits are not frozen.
- Define a durable acceptance point and retry identity: retries cannot create additional logical deliveries or bypass quotas. Transport deduplication is distinct from cryptographic replay protection. Expiration, acknowledgement, deletion, and interrupted synchronization need explicit retention rules.
- Recovery manifests need independent versioning, authenticated ordering/tombstones, and quota accounting. The current SQLite configuration backup is an operator tool with none of these message-recovery guarantees.
- Group membership transitions, device fan-out, key rotation, simultaneous admin changes, prior-history grants, and federation membership authorization require their own reviewed design. Pairwise security is not a proof for groups.

Next completion gates are exact cryptographic primitive/version/license selection, independent known-answer vectors and malformed-key checks, session persistence/crash semantics, the group/federation analysis, and authenticated enrollment/device authorization. No dependency or protocol acceptance is inferred from the functioning Admin server.

### Account/device authorization increment

Implemented invitation-bound enrollment and independent random account/device references under client v0. Capability reporting advertises enrollment separately and still exposes no encrypted-event, federation, or recovery version. The first device supplies a fresh random transport credential before enrollment so an interrupted response can be recovered through authenticated session inspection. This is not an encryption identity or verified key association. Invitations and credentials are stored as hashes; invitations are single-use and bounded, credentials expire and can be rotated/revoked, and disabled usernames remain reserved.

Schema 2 stores account/device authorization metadata. Server-backup restoration clears invitations and revokes every device credential, including credentials that were valid in the old snapshot. This deliberately fails closed; account re-authorization is now implemented below, while end-user history recovery remains open. The operator can prepare home-server hardware, but a real-account deployment handoff waits for these gates and the remaining security/messaging work.

### Cryptographic primitive evaluation increment

`sigil-crypto` is a separate client-side crypto evaluation crate, not a server dependency or an enabled wire suite. It wraps X25519, ML-KEM-1024, HKDF/HMAC-SHA-256 root/chain derivation, and AES-256-GCM-SIV message-key operations, with the experimental handshake described below. These selections are provisional pending the complete protocol and platform performance review. No Triple Ratchet/ML-KEM Braid state machine, persisted ratchet state, or authenticated identity/device binding is implemented yet.

The crate uses x25519-dalek 2.0.1, ml-kem 0.3.2 with zeroize, aes-gcm-siv 0.11.1, AES 0.8.4 with zeroize, HKDF 0.12.4, HMAC 0.12.1, SHA-256 0.10.9, getrandom 0.4.3, and zeroize 1.9.0. Dependencies are pinned; upstream notices are retained separately from server notices. No new hand-written curve, lattice, signature, or block-cipher arithmetic was introduced.

Validation sources:

- [RFC 7748 §6.1](https://www.rfc-editor.org/rfc/rfc7748#section-6.1): X25519 public keys and shared secret; additional tests reject noncontributory and incorrectly sized keys.
- [RFC 5869 Appendix A.1](https://www.rfc-editor.org/rfc/rfc5869#appendix-A.1): HKDF-SHA-256 known-answer test.
- [RFC 8452 Appendix C.2](https://www.rfc-editor.org/rfc/rfc8452#appendix-C.2): AES-256-GCM-SIV known-answer tests; additional tests reject modified ciphertext/AAD and bound allocations.
- NIST ACVP [ML-KEM key generation](https://github.com/usnistgov/ACVP-Server/blob/master/gen-val/json-files/ML-KEM-keyGen-FIPS203/internalProjection.json) and [encapsulation](https://github.com/usnistgov/ACVP-Server/blob/master/gen-val/json-files/ML-KEM-encapDecap-FIPS203/internalProjection.json), ML-KEM-1024 test case 51 from each source. Only the fields needed by the tests are retained in `crypto/tests/vectors/mlkem1024.json`, with the NIST notice in `licenses/NIST-Vectors.txt`.
- Original NIST JSON SHA-256 hashes: key generation `d7a62a2c3476957f56dd8d24f9004ea6776ccfe995ffe71a65bb9506dc9c7b1b`; encapsulation `a556952ce869bb89c3a3196a701dad89647c193a34c86eafb61a9d710d5b810f`.
- Root/chain outputs were independently calculated with OpenSSL HKDF and HMAC, using synthetic fixed inputs. The experimental root context is `Sigil/experimental/root/v0`; chain constants follow the referenced Signal recommendation. These tests validate the selected primitive composition, not a full ratchet.

Important integration boundaries:

- The message-key handle is consumed by encryption, but callers can still re-derive a key from old chain state. Durable send-state commits and anti-rollback handling are mandatory before use. A fixed AEAD nonce is allowed only under the one-message-key/one-encryption rule; this crate does not enforce that rule across restarts or duplicated chain inputs.
- Failed authenticated decryption does not mutate key state. The future receiving ratchet must commit counters/skipped-key removals only after authentication, bound skipped-key storage, and reject replays.
- ML-KEM performs implicit rejection: corrupted ciphertext can produce a different shared secret rather than an explicit error. The authenticated handshake must detect this; successful decapsulation alone is not peer authentication.
- Secret-owning wrappers are not Debug/Clone/Serialize and erase owned buffers on drop. This is not proof that every compiler temporary, library key schedule, allocation, or crash dump is cleared; a memory-handling review remains required.
- Current primitive limits are 64 KiB plaintext and 4 KiB associated data. They are evaluation limits, not a frozen message/attachment protocol. The crate compiles for Android arm64; it has not passed real-device performance, Wasm, or Apple acceptance.

The full workspace now has 144 passing tests, an opt-in fixture-generation helper and subprocess helpers invoked by their parent tests. This is neither NIST certification nor an independent cryptographic audit. Messaging capability remains disabled.

Server restore now stages the restored database, invalidates credentials, closes its journal, and only then publishes the final filename without overwriting existing data. A failure to invalidate credentials must leave no bootable destination database. Interrupted-restore staging files are not treated as live server storage.

### Experimental authenticated handshake increment

`crypto/src/handshake.rs` evaluates the pinned PQXDH DH/KEM composition in memory. Every receiver slot has a signed curve prekey and a signed **one-time** ML-KEM-1024 prekey; an additional one-time curve prekey is optional. Last-resort PQ prekeys and client rotation are not implemented. Experimental authenticated publication/claim distribution is described below. Experimental wire parsing is described below. ML-KEM-1024 is Sigil's provisional selection, not a claim of compatibility with CRYSTALS-Kyber or Signal's wire format.

XEdDSA signing uses Apache-2.0 `xeddsa` 1.1.0. Verification converts the Montgomery identity to an Edwards key with sign zero and uses `ed25519-dalek` 2.2.0 strict verification. Protocol keys reject noncanonical field encodings, small-order keys, and failed curve conversion. The raw X25519 primitive retains RFC 7748 decoding. Signing uses `rand` 0.10.2 StdRng seeded explicitly from OS entropy, so entropy acquisition failure is returned before signing. Upstream signing temporaries and RNG state are not guaranteed to be zeroized; this remains a deployment gate. No signature arithmetic was written in Sigil.

Experimental encoding uses tag 1 plus 32-byte EC public keys and tag 2 plus 1568-byte KEM public keys. The bundle identifier hashes the suite context, identities/prekeys and optional-EC presence; randomized signatures are excluded. This identifies a whole prekey slot, rather than defining final per-key server identifiers. Initial associated data binds both identities, the complete KEM public key, suite context, bundle identifier, ephemeral key and KEM ciphertext. Callers must independently establish the expected peer identity: passing an untrusted downloaded identity does not authenticate a person or device.

The PQXDH HKDF input is 32 bytes of FF followed by DH1, DH2, DH3, optional DH4, then KEM secret; salt is 32 zero bytes and info is `Sigil/experimental/pqxdh/v0_CURVE25519_SHA-256_ML-KEM-1024`. A separate HKDF context `Sigil/experimental/pqxdh/initial/v0` derives the initial AEAD key. These are experimental domain choices, not frozen negotiation rules. There is no configurable downgrade path.

Authentication failure returns neither plaintext nor a session secret and leaves the one-time slot unchanged. Success removes its KEM/optional-EC private keys and subsequent acceptance fails. This is only in-memory replay prevention: publication races, crashes, rollback, cross-process transactions and malicious-server withholding still need their own handling. A post-handshake ratchet must mix fresh receiver randomness before replies; no send/reply protocol is exposed here.

New validation includes both DH variants, signature/ciphertext/identity/ephemeral/bundle substitution, truncated KEM ciphertext, successful retry after failed authentication, and replay rejection after acceptance. [RFC 8032 §7.1 test 1](https://www.rfc-editor.org/rfc/rfc8032#section-7.1), whose Edwards key has sign zero, independently checks signature verification after Montgomery conversion; it does **not** validate XEdDSA's randomized signing procedure. OpenSSL independently checks the PQXDH KDF with each DH/KEM block filled with bytes 01 through 05. Full-handshake cross-implementation checks are described below. Signing nonce/memory review, broader vector coverage, cryptographic review, durable state and the ratchets remain acceptance gates. Android arm64 compilation and dependency auditing pass; these do not establish platform performance or production readiness.

### Experimental handshake byte encoding

`crypto/src/handshake/wire.rs` provides public-data-only `to_bytes`/`from_bytes` methods. It does not serialize private keys or establish a persistence format. The fixed eight-byte header is ASCII `SGPQ`, version 0, suite 1, object kind (1 bundle, 2 initial message), and reserved byte 0. Every byte must match; unknown versions, suites, kinds and reserved bits fail without fallback. Suite 1 maps only to the existing authenticated experimental PQXDH context. These values are provisional and are not advertised by the server.

- Bundle order: header, encoded identity EC key, encoded signed EC key, 64-byte EC signature, encoded ML-KEM public key, 64-byte KEM signature, optional-EC presence byte (0 or 1), then the encoded optional EC key when present. Total length must be exactly 1,772 or 1,805 bytes. Decoding checks canonical curve and KEM encodings, both signatures and the caller's expected identity.
- Initial-message order: header, encoded sender identity EC key, encoded ephemeral EC key, 32-byte bundle identifier, 1,568-byte KEM ciphertext, big-endian u32 AEAD ciphertext length, then AEAD ciphertext. The fixed prefix is 1,678 bytes; ciphertext must be 16 through 65,552 bytes. Total size and the declared length are checked before public-key work or allocation. Parsing validates structure and public keys, but only `Receiver::accept` authenticates the ciphertext and consumes keys.

Neither format permits trailing bytes, alternate key tags or noncanonical optional-key flags. KEM public-key decoding is shared with encapsulation, replacing the earlier length-only handshake check. Tests exercise both optional-key branches across encoded exchange, every truncation point, header/tag corruption, trailing bytes, invalid length claims, malformed public keys, altered signatures, wrong expected identities, maximum payloads and failed-authentication retry. No dependencies were added. Durable acceptance, expanded vector coverage, account/device key authorization, ratchets and real-message deployment remain open.

### Independent handshake receiver checks

`crypto/tests/reference.c` checks both fixtures in `crypto/tests/vectors/pqxdh.json` using OpenSSL 3.6.3, libsodium 1.0.22 and json-c 0.19. It uses no Sigil Rust code or Rust crypto dependencies. OpenSSL independently reconstructs X25519 public keys/DH outputs and the ML-KEM key from its seed, decapsulates the KEM ciphertext, hashes the bundle identifier, derives both HKDF outputs, and authenticates/decrypts the initial message. Libsodium independently validates both prekey signatures after deriving the sign-zero Edwards identity. The checker also validates the fixed framing and compares the expected shared secret and plaintext.

The fixtures were generated by Sigil, then checked by this independent receiver; they are not upstream Signal interoperability vectors or an independent security audit. Inputs are synthetic: Alice's private key is byte 01 repeated 32 times, Bob's 02, signed EC prekey 03, ML-KEM seed 04 repeated 64 times, and optional EC prekey 05 repeated 32 times. Ephemeral keys and signing randomness were generated once for the saved transcripts. Fixture SHA-256 is `44a9ec47adc6edb791cd17cc65ccc9ded510615d23f04174f59a892721b5a4b3`.

The regular Rust regression test checks both stored transcripts, signatures, secret/plaintext agreement, rejection after ciphertext modification, retry after failure and replay rejection after success. The C checker passed with AddressSanitizer and UndefinedBehaviorSanitizer; separate modified-secret, modified-signature and modified-ciphertext inputs each failed as expected. This validates the tested composition and signing outputs, not the distribution or erasure of signing randomness, every adversarial key class, or durable acceptance semantics.

Run the independent checker from the repository root with the system development packages installed:

```sh
cc -std=c11 -Wall -Wextra -Werror -O2 crypto/tests/reference.c \
  -o /tmp/sigil-crypto-reference $(pkg-config --cflags --libs openssl libsodium json-c)
/tmp/sigil-crypto-reference crypto/tests/vectors/pqxdh.json
```

For fixture maintenance, `cargo test -p sigil-crypto generate_reference_inputs -- --ignored` writes fresh synthetic candidates to `/tmp/sigil-pqxdh-vectors.json`. Run the C checker on that file before adopting it and updating the recorded hash. The ignored helper generates data; no acceptance test is skipped. The C checker is test tooling only and is not linked into the server or clients. No Cargo dependencies changed.

### Classical ratchet and session handoff

`crypto/src/ratchet.rs` implements the classical Double Ratchet component from pinned revision 4, section 3. It uses the existing root/chain KDFs and message AEAD. This is not the sparse post-quantum ratchet or Triple Ratchet composition, and it is not enabled for real-message delivery.

`handshake::initiate_session` and `Receiver::accept_session` bind the ratchet context to SHA-256 of the exact initial PQXDH message encoding. The signed EC prekey supplies the initial responder DH key. The responder waits for the first authenticated ratchet packet before replying; that receive step introduces fresh responder DH randomness. Direct ratchet constructors are lower-level evaluation interfaces whose caller must supply authenticated handshake material and context. No plaintext/key persistence is performed by these helpers.

Experimental packet format: eight-byte prefix (ASCII `SGDR`, version 0, suite 1, two reserved zero bytes), 32-byte canonical DH public key, big-endian u32 previous-chain length, big-endian u32 message number, then 16 through 65,552 bytes of ciphertext. Total header length is 48 bytes. The entire header and 32-byte session context are AEAD associated data under `Sigil/experimental/double-ratchet/v0`. Unknown headers and invalid key encodings fail before ratchet use. Headers remain visible; encrypted headers are not implemented.

Receiving stages changes in a private candidate containing root/chain keys, counters and DH state. Failed authentication discards it. Existing skipped keys are borrowed for authentication and removed only on success; they are not copied with every candidate. At most 128 skipped keys are retained across all DH chains, with a bounded gap and no silent eviction. Exhaustion rejects the packet; receiving an earlier retained packet frees a slot for retry. Counter overflow fails before advancing state. Key-owning public types remain non-Clone/non-Debug/non-Serialize; private candidate copies are transient owned secrets, subject to the existing memory-review limits.

Tests cover alternating turns, encoded packet exchange, delayed messages across DH rotations, replay rejection, authentication failure on fresh/current/skipped-key paths, preservation of an existing send chain after a failed DH turn, the global skipped-key budget across rotations, overflow, context/header tampering, payload limits, and PQXDH handoff. These are state-machine regression tests; independent complete ratchet vectors remain required.

The durable adapter still needs these atomic boundaries before transmission or plaintext delivery:

| Operation | Required durable transaction |
| --- | --- |
| Send | Compare the session revision, save advanced ratchet state, and insert the exact ciphertext into an outbox. Retry the saved ciphertext after uncertain delivery; never re-encrypt from old state. |
| Receive | Compare the session revision, save authenticated candidate state/skipped-key removals, and store the accepted message plus deduplication identity before exposing plaintext. Authentication failure writes nothing. |
| Initial handshake | Consume the one-time prekey and create the session/accepted initial message in one transaction. Concurrent or restarted consumers must not create a second session from the same slot. |

In-memory success currently returns a packet/plaintext immediately; it is not a substitute for these transactions. The native adapter below now implements encrypted classical-session storage, concurrent-writer checks, abrupt-exit testing and packet outbox retries. Device key wrapping and durable handshake acceptance remain unimplemented. SQLite transactions alone cannot detect replacement of the entire database with an old snapshot; recovery must establish fresh sessions, with an explicit design for rollback detection where required. Independent ratchet review and ML-KEM Braid/Triple Ratchet integration remain deployment gates.

### Server public-prekey distribution increment

Schema 3 adds a device encryption-identity pin and durable public-prekey assignments. Publication and claims run inside immediate SQLite transactions, including authorization checks, so another connection cannot revoke the device between authorization and the write. Publication pins the first identity and rejects subsequent identity changes; signatures and canonical cryptographic-key checks remain the recipient's responsibility. Enrollment ownership is not proof of key possession or a verified person/device association.

The publication ID is SHA-256 of the ML-KEM public key, distinct from the client's full-bundle transcript identifier. Its global uniqueness prevents republishing the same one-time KEM key under another device or another bundle signature. Exact upload retries return success without refreshing expiry or resetting claims. A retained payload hash distinguishes exact retries after payload expiry. Claim IDs are scoped to the authenticated claimant device and cannot be reused for another target. Claimed ciphertext-free public bundles are retained for retries until expiry; identifier/assignment tombstones are retained to prevent reissue. Backup restoration clears all public bundle payloads as well as invalidating credentials, preventing restored inventory from becoming available for new claims.

The existing 8 KiB request limit and bounded database worker apply. One bundle per upload, 64 unclaimed live bundles per device, one-hour to seven-day expiry, and a hard lifetime maximum of 4,096 identifiers bound initial storage growth. Expired payloads are now cleared by the bounded background maintenance described below; live inventory counts exclude expired bundles even before cleanup. Reaching the lifetime maximum stops further uploads. A reviewed key-epoch/renewal policy is required before ongoing deployment; tombstones must not simply be discarded and reused. Enrollment remains invitation-only, but this does not replace per-peer claim abuse controls. No production crypto dependencies were added: `sigil-crypto` is only a server dev-dependency for the end-to-end integration test.

Validation covers HTTP authentication/origin policy, claim-to-PQXDH/ratchet handoff, upload/claim retries across database reopen, concurrent claims through separate SQLite connections, identity pinning and cross-device key reuse, claim expiry with replacement inventory, target revocation, restoration payload clearing, schema 2 migration, live-inventory and lifetime limits. The existing schema 1 migration test now checks schema 3. Docker publication and idempotent claims pass with non-root/read-only settings; the test container and temporary credentials were removed. Messaging, federation and recovery capabilities remain disabled.

### Server durable mailbox increment

Schema 4 introduces a local device-to-device opaque mailbox. Submission checks sender authorization, recipient activity, configuration/quota, retry identity and queue bounds inside an immediate SQLite transaction. Payload insertion and its receipt commit together. Separate connections submitting the same sender/message ID obtain the same sequence; altered content/recipient/expiry conflicts. The payload hash, recipient and absolute expiry remain attached to the retry identity after acknowledgement or expiry. No server-side decryption or encryption-authenticity claim is made.

Recipient polling is non-consuming, ordered and limited to 16 messages. Acknowledgement removes only the authenticated recipient's payload and is repeatable. Senders cannot acknowledge another device's delivery. Expired payloads are now cleared by bounded background maintenance; tombstones are retained. Polling and quota queries exclude expired payloads independently of cleanup. The expiry query uses an index over non-null payloads. Revocation stops future authorization/targeting; already-accepted sender messages are not automatically retracted. Database replacement from backup clears all mailbox payloads along with prekeys/credentials, deliberately sacrificing snapshot queues to prevent server restoration from replaying them.

Development limits are 256 pending messages per recipient, 4,096 lifetime delivery IDs per sender, 67,230 binary payload bytes represented as hex, and a 140 KiB HTTP submission body. Recipient-account quota charges encoded payload bytes across its devices. These limits do not constitute fair scheduling or peer authorization: an invited abusive sender could exhaust a recipient's queue. Admission/abuse policy, tombstone renewal without retry reuse, longer-term storage policy and multi-device/group semantics remain required before deployment. The server's receipt is acceptance into storage, not proof of recipient delivery, ratchet acceptance or read status.

Validation includes delivery/decryption of a real classical-ratchet packet, database reopen before polling, repeat polling and acknowledgement, retry after acknowledgement, altered retry content, cross-device access rejection, two-connection duplicate submission, expiry, revocation, batch/queue/lifetime/quota bounds, schema 3 migration and backup payload clearing. Existing migration tests cover older schemas through schema 4. HTTP accepts maximum-size encrypted payloads while preserving existing Admin limits and native-device authentication/origin policy. Docker prekey plus mailbox submission/retry/poll/acknowledgement checks pass in a non-root read-only container; synthetic test credentials/container were removed. No new dependencies were added.

A native persistence adapter for classical sessions is now implemented below; complete client handshake and transport integration remains open. The mailbox therefore remains an experimental transport and does not close the durable client acceptance/outbox gates or the post-quantum/Triple Ratchet gate. `delivery: [0]` is reported separately; encrypted-event, federation and recovery capabilities remain empty.

### Recipient admission and queue fairness increment

Schema 5 adds explicit recipient-device → sender-device grants with a 256-entry bound. Cross-account mailbox submission and prekey claim/retry require a grant; same-account active devices remain implicitly allowed. Grant changes and both authorization checks run inside the existing immediate transactions, serializing them with concurrent submission. Removing a grant also clears that pair's pending payloads atomically, preserving tombstones so a future grant plus old upload retry cannot redeliver cleared content. Already-downloaded keys or recipient-held messages cannot be revoked by the server.

The per-pair pending limit is now 64, within the existing 256-message recipient limit and account byte quota. This limits one approved device's queue share; it is not a request-rate limiter and does not prevent coordinated abuse by multiple approved devices. Schema 5 migration clears cross-account payloads accepted under the earlier permissive transport instead of delivering them without consent; the new grant table starts empty. Backup restoration still revokes all devices and clears all public-prekey/mailbox payloads. Restored grants refer only to those revoked device IDs and are not inherited by new devices.

Validation covers wrong-direction grants, default denial for prekeys and messages, repeated grants/removal, persistence, prekey retry denial after removal, queued-payload clearing, no resurrection after regrant, same-account behavior, schema 4 migration, HTTP authentication/origin policy and submission racing permission removal through separate database connections. Existing migration fixtures now reach schema 5. Docker tests use two synthetic accounts to check denial, grant, delivery and removal with queue clearing; the non-root read-only test container and credentials were removed. Workspace tests and Clippy pass; no dependencies changed.

This is an explicit developer API, not the final contact-request UX. Device discovery, authenticated identity verification, contact invitation/bootstrap policy, abuse/rate handling and post-quantum/client-persistence gates still need implementation. The new admission capability does not enable an encrypted-event version.

### Contact invitation bootstrap increment

Schema 6 adds hashed, device-owned contact invitations. Clients persist a fresh random secret and absolute expiry before creation, allowing an uncertain creation response to be retried without a new invitation. The invitation ID is the hex SHA-256 digest of the secret's canonical hex text, not the secret itself. It is safe for management routing but does not redeem the invitation. Exact creation retries never refresh expiry, clear revocation or reset the claimant.

Redemption checks both device authorization and invitation ownership/liveness in an immediate SQLite transaction. The first claimant is recorded alongside reciprocal admission grants. Both grants use the same bounded helper as manual permission changes; if either grant fails, the entire transaction rolls back, including the first grant and claimant assignment. Two concurrent claimant devices cannot both succeed. A recorded claimant can retry only while both permissions remain effective; retry does not silently undo a later permission removal. Self-redemption is rejected. Cancellation disables redemption/retry, while established permissions are managed separately.

Limits are 16 pending invitations and 4,096 lifetime IDs per device, with expiry no more than seven days away. Only secret hashes are retained, and secret-bearing request types do not implement Debug. Invite secrets are bearer authorization: first possession plus an authenticated local device is enough to redeem. This is consent bootstrapping, not a signature, safety-number check, verified encryption identity or person authentication. The inviter must share through an appropriate trusted channel. Existing outstanding invitations are separate authorization capabilities; removing a sender grant does not automatically cancel every other invitation.

Validation covers creation/redeem retries across reopen, reciprocal message delivery, non-restoration of removed grants, management-ID non-redemption, cancellation, expiry, self-redemption, pending capacity, backup invalidation, competing claimant connections, rollback when the second permission list is full, and HTTP device-auth/origin policy. Docker verifies creation, redemption/retry and reciprocal grants between two synthetic accounts in the non-root/read-only container; temporary credentials and containers were removed. Existing migration fixtures reach schema 6. Clippy and all workspace tests pass; dependencies are unchanged.

This completes a server API for contact bootstrap. It does not implement the client sharing UI, remote contact discovery, identity verification, request-rate controls, durable client sessions or the post-quantum ratchet. Those remain deployment gates.

### Bounded request-rate increment

`server/src/rate.rs` adds process-local token buckets without new dependencies or schema changes. Client v0 traffic has a shared pre-authentication budget (100 burst, 50/second refill); enrollment additionally has a 5-burst, one-per-5-seconds budget. Authenticated requests resolve stable device IDs through the bounded database worker before allocating map entries. Each device receives an overall 60-burst, 2/second budget plus a 20-burst, one-per-2-seconds submission budget. GET/HEAD/OPTIONS/DELETE bypass only the submission budget, allowing queue acknowledgement and permission removal after submission exhaustion. Handlers retain transaction-local authorization, so the preliminary lookup cannot bypass a concurrent revocation.

The map has at most 4,096 entries. Idle entries are reclaimed only after 60 seconds, longer than either bucket's full refill interval. Active entries are never evicted merely to admit new devices; full capacity returns retryable 503. Credits use monotonic elapsed time with nanosecond precision and saturate at the configured burst. A backwards test instant cannot manufacture credit. Rejected requests do not hold a mutex across an await or body read. The existing five-second timeout covers the limiter lookup as well as handler work; cancellation does not release the bounded blocking-database permit prematurely.

Tests cover fractional refill, burst caps, monotonic time handling, independent device budgets, read access during submission exhaustion, bounded map reclamation, Retry-After rounding, authenticated malformed requests, credential rotation preserving limits, enrollment throttling and health availability. Docker verifies 429/Retry-After and readable session state after submission exhaustion in the non-root/read-only container. Test credentials/container were removed; all workspace tests and Clippy pass.

These are development limits, not a complete anti-abuse design. The global pre-authentication budget can be exhausted by an attacker and affect other users; no source-address identity is inferred from untrusted forwarding headers. Process restart resets the limiter. Trusted reverse-proxy/edge controls, workload measurement, multi-process coordination if introduced, and tuning remain open. No encryption/persistence gate is closed by rate limiting.

### Bounded background expiration increment

Schema 7 adds expiration indexes for prekeys and enrollment invitations. The server attempts one immediate transaction per second, clearing at most 64 expired mailbox payloads, 64 expired prekey payloads and 64 expired enrollment reservations. Indexed ordering bounds each batch without scanning the entire retained history. Mailbox/prekey retry identities, hashes and assignments remain; contact-invitation tombstones are not removed. A failure rolls back the entire batch.

Maintenance shares the existing bounded database worker and skips attempts while it is busy, without queuing more work. Consecutive storage failures emit one generic log message until recovery. The shipped server runs an explicit maintenance future alongside HTTP serving and drops it after graceful HTTP shutdown; the router-only library helper does not start hidden tasks. Sustained database contention can delay cleanup and requires workload validation.

The superseded global mailbox cleanup during submission and per-device prekey/poll cleanup are removed. Live inventory, queue bounds, account quotas and polling exclude expired payloads directly, so delayed cleanup does not make expired content available or consume live capacity. Clearing payload columns makes database pages reusable; it does not guarantee immediate file shrinkage or erase copies in SQLite pages, WAL files or backups. Retained identifier lifetime limits and renewal policy remain unchanged.

Validation covers exact expiry boundaries, multiple bounded batches, retained live payloads and tombstones, rollback after an injected storage failure, and schema 6 migration. A real server process clears 130 expired payloads without client traffic and shuts down gracefully. All 96 workspace tests pass (one opt-in fixture generator remains ignored); Clippy with warnings denied, the Docker build and the non-root/read-only transport/admission/contact/rate smoke test pass. No dependencies were added. Client persistence, post-quantum ratcheting and the other security/deployment gates remain open.

### Device revocation authorization increment

Device revocation now checks the caller and updates the target in one immediate transaction using the existing authorization helper. Previously a separate session read could authorize a write after another connection revoked the caller. Competing same-account devices revoking each other now serialize: only one succeeds, and the other fails authorization. Target ownership and repeat revocation by a still-authorized caller remain enforced. Session lookup also rejects timestamps outside SQLite's signed-integer range instead of wrapping them into a value that could accept expired credentials. These timestamps are an internal API edge case, not client-controlled HTTP input.

Three regression tests cover timestamp boundaries without mutation, authorized versus revoked retry behavior, and 32 simultaneous revocation races through separate connections. Synthetic fixtures represent a second device; this does not implement authenticated device linking. All 99 workspace tests and Clippy with warnings denied pass. The Docker image builds and passes non-root/read-only prekey and transport/admission/contact/rate checks; disposable containers and credentials were removed. No dependencies or schema changes were needed; the previous non-transactional revocation implementation was removed. Account re-authorization, durable client sessions and the remaining security/deployment gates are still open.

### Account re-authorization increment

Admin can now issue a short-lived, single-use invitation for an existing enabled account through `POST /admin/v0/accounts/{id}/reauthorization-invitations`. The client explicitly redeems at `/client/v0/reauthorize` using a freshly generated, durably retained transport credential. The existing enrollment implementation is shared rather than duplicated. An immediate transaction checks the account, invitation and credential uniqueness, revokes every old device, creates an independent replacement device and consumes the invitation. Failed insertion rolls back revocations and invitation consumption. Issuance alone leaves current access intact. A lost successful response is recoverable through session lookup with the replacement credential.

The existing invitation table is reused without a migration: usernames are immutable and never reassigned, and a recovery invitation is bound to the existing account under that username. Ordinary invitation creation still rejects existing usernames; ordinary enrollment rejects their invitations, and recovery redemption rejects invitations for uncreated accounts. Disabled accounts cannot be recovered. The shared expiry, cancellation and global pending cap apply; backup restore deletes these invitations too, requiring fresh Admin authorization afterward. Redemption shares the enrollment rate bucket and native-only origin policy. Retained device history is capped at 256 records per account; a reviewed renewal policy is still needed.

Recovery preserves the account ID and address but never copies the old device's encryption identity, prekeys, mailbox assignments or sender permissions. Old device records remain revoked; their retained payloads follow existing expiration rules. This is account access authorization, not authenticated device linking, person verification or encrypted-history recovery. Admin access cannot supply missing client keys. Clients must start fresh sessions, surface identity changes and re-establish trust; these client flows remain unfinished. Capability reporting adds `reauthorization: [0]` and leaves encrypted-history `recovery` empty.

Seven new regression tests cover fresh device/permission state, multi-device revocation, reopen/lost-response lookup, wrong endpoint use, expiry, cancellation, disabled accounts, credential collision, rollback on injected storage failure, backup restore, concurrent redemption, retained-device limits, HTTP authorization/origin policy and the shared rate budget. All 106 workspace tests pass, along with Clippy with warnings denied. The Docker build and non-root/read-only recovery smoke test pass, including old credential rejection, fresh permissions and replay rejection; temporary credentials and the test container were removed. Dependencies and schema are unchanged. Durable client sessions, post-quantum ratcheting, recovery UI/history and the remaining deployment gates stay open.

### Durable classical-session client increment

`sigil-client` is a native Rust adapter over the existing ratchet engine and existing SQLite dependency. It reloads an encrypted session checkpoint inside each immediate transaction, authenticates or encrypts a message, then saves the next revision with the corresponding inbox/outbox record. No packet or accepted plaintext is returned before commit. Separate connections serialize writes and revision checks prevent stale updates. Failed authentication and failed message insertion leave durable state unchanged. Existing in-memory ratchet operations remain the implementation used by this adapter; they are not a second protocol implementation.

Send IDs bind a keyed plaintext commitment to the stored packet. Exact retries return the committed bytes without advancing the chain; altered input conflicts. Acknowledgement clears the packet but retains the ID/commitment, preventing re-encryption under that ID. Receive IDs bind a keyed packet commitment to encrypted local content. Exact retries recover committed plaintext without consuming a second key; packet reuse under another ID is rejected by the ratchet. A separate read method retrieves committed plaintext offline. Pending outbox reads return at most 16 packets in insertion order.

`sigil-crypto` adds encrypted checkpoints and a local-storage envelope. AES-256-GCM-SIV uses fresh random 96-bit nonces; HKDF separates encryption and HMAC commitment keys from the caller-supplied wrapping key. Authenticated data binds purpose, session ID, and message ID or state revision. Checkpoints retain DH/root/chain state, counters, context and at most 128 skipped keys; framing rejects malformed versions, invalid optional fields, oversized counts, duplicate skipped keys and trailing bytes. Secret checkpoint buffers are zeroizing and preallocated to avoid reallocating secret-bearing data. This does not prove erasure of every compiler/library temporary.

SQLite uses WAL and FULL synchronization. The file adapter currently requires a private Unix parent directory and a regular private database file and is tested on Linux. A sealed verifier rejects the wrong wrapping key; unrelated/future database formats are refused. Session checkpoints, outbox packets and inbox plaintext are encrypted, while record identifiers, relationships, sizes and revisions remain metadata. The wrapping key is never stored in SQLite. Platform secure-key wrapping and lifecycle adapters remain to be implemented.

Limits are 1,024 sessions per database, 256 pending sends and 4,096 lifetime send IDs per session, plus 4,096 inbox entries per session. There is no implemented retention/deletion or key-rotation policy. Whole-store rollback, or replacement with valid earlier records and matching metadata, is not detected. These live ratchet checkpoints must never be restored as a history archive; recovery must establish fresh sessions. The packet outbox still needs persisted peer/routing/absolute-expiry metadata for complete HTTP retries. Atomic initial-handshake acceptance and one-time-prekey consumption are not yet part of the database transaction, and the Android/UI adapters are not connected.

Validation adds nine client tests and four crypto tests: reopen/identical retry, acknowledgement tombstones, offline reads, skipped-key restoration, concurrent sends and receives, injection of inbox/outbox storage failures, authenticated-record tampering, wrong keys, maximum message size, queue/counter limits, private-path checks and malformed checkpoints. An explicit child process exits without destructors after a committed send and an unfinished transaction; reopening retains only committed work. A full 128-skipped-key checkpoint remains usable across DH turns. All 119 workspace tests and Clippy with warnings denied pass; the helper marked ignored is explicitly invoked by its parent test. The server Docker image still builds with the new workspace member. No new third-party dependency versions were introduced, and the server's normal dependency graph remains separate from client cryptography.

This implements persistence for already-established classical sessions. Durable handshake/prekey creation, platform key protection, transport integration, independent review, post-quantum/Triple Ratchet persistence and encrypted history recovery remain open. The encrypted-event capability is still disabled.

### Durable incoming PQXDH acceptance increment

Client schema 2 adds an encrypted local identity and encrypted private prekey slots. The client commits identity and slot creation before returning public bundle bytes. A live slot retry reconstructs and returns the original signed bundle rather than generating replacement keys; changing optional EC-key presence conflicts, and consumed slot IDs remain tombstones. Limits are 64 live slots and 4,096 retained slot IDs. Version 1 migration preserves session state and pending packets and runs after wrapping-key verification. Server schema and wire capabilities are unchanged.

Encrypted identity checkpoints have strict version/length checks. Private-slot checkpoints retain the signed curve key, ML-KEM seed, optional curve one-time key and canonical public bundle, with preallocated zeroizing secret buffers. Loading verifies bundle signatures against the persisted identity and checks that every restored private key reproduces its public key. Consumed receivers cannot produce a live checkpoint. No raw secret serialization API is exposed.

`accept_initial` parses the bounded initial packet and checks the independently selected sender identity inside the existing immediate transaction. It reconstructs a private receiver, authenticates the handshake, inserts the initial classical-session checkpoint and encrypted initial plaintext, and clears the private slot before commit. The private candidate is discarded on failure. Retry commitments bind the initial transcript hash, local and sender identities, local prekey slot, session ID and message ID. Identical delivery retries recover committed plaintext without consuming a second key; different sessions cannot share a slot. Existing session creation uses the same bounded insertion helper, removing the earlier duplicated insertion path.

Seven client tests and two crypto tests cover both optional-key branches, encrypted identity/slot reopen, malformed checkpoints and key mismatch, exact and altered retries, usable ratchet handoff after restart, maximum initial plaintext, identity/authentication failures, rollback after message insertion or slot consumption failure, simultaneous identical/different handshakes, slot bounds and schema migration with an existing outbox. A disposable subprocess commits an initial handshake and exits without destructors; reopening recovers the message and consumed-key tombstone. All 128 workspace tests and Clippy with warnings denied pass. The server Docker build and non-root/read-only transport, rate-limit and re-authorization smoke checks pass; disposable containers and credentials were removed. SHA-256 is now a direct client dependency at the version already pinned in the workspace; no third-party versions changed.

This closes the incoming handshake transaction boundary for the experimental native adapter. Outgoing initial-message/session persistence, full HTTP request and prekey-ID mapping, private-slot expiry/retirement, platform secure key storage and application integration remain open. Expected identity selection is still the caller's responsibility. SQLite logical deletion does not guarantee secret erasure from pages/WAL or earlier snapshots, and old live-state databases must never be restored. Post-quantum ratcheting, encrypted-history recovery and independent security review remain separate deployment gates.

### Durable outgoing PQXDH increment

`start_initial` verifies the expected recipient and signed bundle, derives an initial packet and classical session, then commits both with a retained recipient KEM identifier in one immediate transaction. No packet escapes before commit. Retry commitments bind the local/recipient identities, exact public bundle, session/message IDs and plaintext. Exact retries recover the saved initial packet without recreating the handshake or resetting an advanced session. Reusing that ID with a ratchet send or changed initial inputs conflicts; acknowledged IDs remain tombstones.

Client schema 3 adds a unique recipient one-time KEM identifier mapped to each outgoing session. The identifier matches the server publication ID and excludes randomized signatures, preventing another local session from reusing the same KEM prekey. It remains reserved after packet acknowledgement. Schema 1/2 migration preserves existing identity, private prekeys, sessions and outbox records. The existing 1,024-session limit also bounds initiation records. Initial and ratchet packets share encrypted outbox insertion, retry and acknowledgement helpers; the duplicated retry/insertion code was removed. Local record bounds now accommodate the maximum 67,230-byte initial packet without changing plaintext message limits or the envelope format.

Five added client tests cover restart retries, both optional-key branches, maximum initial packets, ratchet continuation without session reset, changed inputs and cross-method ID conflicts, acknowledged/prekey-use tombstones, rollback of failed outbox or initiation insertion, concurrent identical retries and competing sessions, schema 2 migration, and abrupt process exit after a committed initial send. All 133 workspace tests and Clippy with warnings denied pass. The subprocess helper is invoked explicitly by its parent test. No dependencies changed.

Both incoming and outgoing handshake transactions now persist in the experimental native adapter. Complete HTTP routing/absolute-expiry metadata and retry integration, platform key protection, private-prekey retirement, identity verification UI, post-quantum/Triple Ratchet and encrypted-history recovery remain open. Live database rollback can still revive old key state and is not a supported recovery mechanism. This does not enable a production encrypted-event suite.

### Frozen mailbox request increment

Client schema 4 adds encrypted delivery metadata referencing an existing outbox record. `prepare_delivery` atomically freezes recipient-device ID and absolute expiry before returning the mailbox `Submit` body. Packet creation remains its own earlier durable transaction: interruption between the two leaves an explicit unprepared packet, never an inferred or partially saved network request. Metadata uses the existing local encryption key and binds record purpose, session and message IDs. IDs are unique across the delivery table, preventing different local sessions from sharing one server message ID. Metadata remains after acknowledgement alongside the existing outbox tombstone.

`pending_deliveries` reconstructs at most 16 complete request bodies in outbox insertion order. It rejects malformed/authentication-failing metadata and stops on unprepared or expired packets rather than skipping them or silently extending expiry. Retrying preparation with different recipient or expiry conflicts. New expiry is future and at most seven days from the supplied current time; frozen expiry remains unchanged on retry. Expired packets remain available locally for explicit future failure/recovery handling. These methods do not perform HTTP, choose a server origin, authenticate a device or establish that a routing device belongs to the verified encryption identity.

Schemas 1–3 migrate transactionally. Existing outbox packets remain intact and unprepared until their original transport fields are supplied; migration does not invent them. The raw packet queue remains available to the crypto adapter/tests, while network integration must use frozen requests. The schema's foreign key keeps delivery metadata attached to its outbox identity. Row counts inherit the existing outbox/session limits. No new cryptographic format or third-party version was introduced; the client now uses the shared protocol crate and the server as a test-only dependency.

Six tests cover restart reconstruction through actual server mailbox storage, identical server receipts without duplicate delivery, concurrent conflicting preparation, expired and altered expiry rejection, rollback after injected metadata insertion failure, tampering, cross-session ID conflicts, schema 3 migration, blocked unprepared queues and maximum-size initial packets. All 139 workspace tests pass; Clippy with warnings denied passes. The integration test exercises the server store directly, not a completed network client.

Transport request bodies now persist, but the authenticated HTTP retry/receipt loop, inbound routing/peer mapping, explicit expired-queue recovery, platform key protection and application integration remain open. Ongoing post-quantum ratcheting, encrypted history recovery and independent security review remain separate gates.

### Durable server receipt increment

The receipt-free outgoing acknowledgement implementation is removed. `acknowledge_sent` now requires a server receipt for a prepared request, checks a positive sequence and matching frozen expiry, and commits encrypted receipt storage together with packet removal. A repeat must match the original receipt exactly; another sequence cannot overwrite acceptance. Failed receipt insertion/update or packet removal rolls back the entire transaction. Delayed valid responses can complete locally expired requests without changing their original expiry.

Client schema 5 adds an encrypted receipt column to delivery metadata. It migrates earlier schemas transactionally and leaves previously cleared packets without receipt data unchanged: acceptance is unknown, not inferred or recreated. `delivery_receipt` supports readback after restart. Receipt records bind their purpose, session and message IDs using the existing storage envelope. Limits and dependencies are unchanged.

Five added tests cover invalid/expired-mismatched receipts, unprepared packets, readback and exact retries after restart, competing receipt values, injected failures at each write, encrypted receipt tampering and schema 4 migration without invented receipt data. Existing packet and handshake tests now exercise the receipt-requiring path; the integration test passes the actual server-store receipt. All 144 workspace tests and Clippy with warnings denied pass.

This validates and persists server acceptance data; it does not authenticate the network response itself or prove peer decryption/read status. The HTTP adapter must correlate a response with the exact submitted message, since the receipt body contains sequence and expiry only. Authenticated network retry/backoff, peer-to-device mapping, platform key protection, ongoing post-quantum ratcheting and encrypted-history recovery remain unfinished.

### Triple Ratchet and durable hybrid sessions

The experimental native session path now uses the Triple Ratchet. A libcrux incremental ML-KEM-1024 adapter, bounded RaptorQ fragment codec, all eleven ML-KEM Braid states, sparse message-key ratchet with previous-chain sealing, and hybrid key composition are implemented. The classical component's key operations are reused; its former native handshake/session path is replaced. Full composite headers are authenticated, and failures discard both ratchet candidates, including skipped-key consumption and Braid changes.

PQXDH initial profile 2 binds Triple Ratchet selection through a distinct initial-message KDF and associated data. Changing the profile in transit fails authentication; native acceptance has no classical fallback. Existing public PQXDH bundles remain valid key material. Raw profile 1 stays available only as a primitive evaluation/reference path. Client schema 6 preserves earlier stored records while requiring fresh handshakes for legacy sessions; it never reinterprets or silently upgrades their classical state.

Encrypted hybrid checkpoints cover both ratchets, Braid fragments/cursors, bounded skipped keys and epoch state. Restore reconstructs library objects from compact seeds and bounded public records, validates consistency, and rejects malformed or oversized state. Checkpoint buffers are preallocated/zeroizing and capped at 32 KiB. Native send/receive/handshake transactions now persist these complete checkpoints. Tests inject database failures at PQ key creation and epoch completion and verify that both states roll back together. Full packet digests feed incoming keyed retry commitments to accommodate the larger authenticated header.

The precise profile, pinned-specification ambiguities, epoch/counter interpretation, bounds, fixture hashes and review limitations are in `docs/TripleRatchet.md`. No upstream Signal implementation code is incorporated. New dependencies are libcrux-ml-kem 0.0.10 (ML-KEM-1024/incremental only) and RaptorQ 2.0.1; required transitive notices are retained. RustCrypto remains used by PQXDH and provides an independent ML-KEM comparison. The dependency scan reports no known vulnerabilities and one informational unmaintained warning in optional `cfg(hax)` tooling; it is not suppressed.

Validation now comprises 173 passing workspace tests (172 in the full run plus the added targeted PQ-transition storage regression), Clippy with warnings denied, independent OpenSSL/libsodium verification of both handshake profiles, OpenSSL KDF/MAC vectors, and C AddressSanitizer/UndefinedBehaviorSanitizer checks. All Braid states are restored in tests; repeated epochs exercise loss, reordering, asymmetric sends, every-byte tampering and maximum skipped-key backlogs. Crypto and native persistence release builds pass for Android arm64. The server Docker image builds and passes the existing non-root/read-only recovery/transport smoke test; test containers and credentials are removed.

This closes the initial implementation of ongoing post-quantum ratcheting and its durable native handoff, not the security/deployment gates. Independent protocol/security review, device identity authorization, authenticated network integration, platform key protection, memory/timing review, recovery archives and the remaining product/backend features are still required. Encrypted-event capabilities remain disabled, and the UI has not been started.

### Encrypted history recovery storage and native transactions

The experimental recovery format now encrypts retained text/history metadata and deletion markers under an independently generated recovery secret. A canonical homeserver plus stable account reference binds the scope. Authenticated records, ordered pages and chained manifests use the existing AES-256-GCM-SIV storage primitive; identifiers hash ciphertext, and strict parent references prevent substitution. Pages hold at most 256 records and manifests at most 512 pages. Neither live ratchet state, identity/prekey secrets, transport credentials nor verification decisions are archive record types. Exact framing and limits are in `docs/Recovery.md`.

Server schema 8 adds account-scoped immutable ciphertext storage and compare-and-swap manifest heads. Exact retries do not advance a head twice. Mailbox and recovery storage share account quota atomically; deletion batches require the current head, never delete the current manifest, and retain bounded object-ID tombstones. Account reauthorization preserves archive access without granting a recovery key. Operator restore retains ciphertext but explicitly flags its head as a restored checkpoint; the native importer refuses that flag until reconciliation is implemented. `recovery_storage: [0]` advertises only opaque storage; end-user `recovery` remains disabled.

Native schema 7 retained separately encrypted outgoing text after transport acknowledgement; legacy missing text is not invented. Schema 8 adds protected recovery configuration, encrypted history records, a durable exact-ciphertext upload queue and verified import staging. Upload publication requires all object acknowledgements. Local edits and deletion markers can enter the next snapshot while a prior snapshot is uploading. Competing authenticated successor manifests merge with preserved local edits. Imports revalidate every page/record before atomically committing history plus their trusted head; failures preserve the previous visible state. Older records cannot replace higher revisions, and deleted entry IDs cannot become retained content again. Restoring history does not create any live session/identity/prekey/inbox/outbox records.

All 192 workspace tests and Clippy with warnings denied pass. Tests cover account isolation, quota sharing, stale/concurrent publication, authenticated HTTP route policy, object deletion rollback, operator restore flags, 257-record multi-page interrupted import, exact upload retries across restarts, injected snapshot/import storage failures, malformed/substituted objects, competing backups, deletion persistence and fresh-client recovery through actual server storage after account reauthorization. Android arm64 release compilation, the server Docker build and its non-root/read-only operational smoke test pass. No new dependencies were needed for this increment.

This completes an experimental text-history storage/transaction path, not onboarding or recovery acceptance. Authenticated HTTP integration, automatic scheduling, platform key protection, live-history classification/deletion integration, media, operator-restore reconciliation, retained-manifest catch-up, garbage collection, independent review and the remaining deployment gates stay open. A fresh device with only the recovery secret cannot independently detect a malicious server serving an older valid archive.

### Durable HTTPS transport, Android wrapping and initial confirmation

The Rust client now uses certificate-verified HTTPS with pinned ureq 3.4.0/rustls dependencies. It disables redirects, environment proxies and compression, bounds request/response sizes and deadlines, and limits outstanding OS DNS workers to four even when resolution times out. Errors never echo credentials or remote bodies. Optional private CA roots replace the public root set and remain certificate/hostname verified. Native schema 9 encrypts credentials, pending enrollment/rotation, bound server/account/device data and private trust roots. Enrollment and credential rotation recover exact server-side commits after local failure without inventing replacement identities.

Outbox requests now submit over HTTPS and persist validated receipts. Recovery transfer uploads at most 16 objects per step, resumes downloads within a page, and persists each acknowledgement/record. Completed-page caching does not replace final manifest/page/record authentication. Real TLS fixtures exercise wrong CAs/hostnames, redirects, response bounds, DNS timeouts, ambiguous enrollment/rotation, outbox receipt failure, rate-limited interrupted upload and interrupted fresh-device recovery after account reauthorization. All certificates, domains and credentials are synthetic. Native client dependency notices are in `licenses/Client-ThirdParty.txt`.

The Android adapter wraps an independent 256-bit database key with a hardware-backed Android Keystore AES-GCM key. It prefers StrongBox where available, requires hardware-backed fallback, uses private credential-encrypted non-backed-up storage, durable atomic file replacement and exclusive locking, and clears borrowed plaintext key bytes. Missing aliases/files or altered ciphertext cannot reset an established database. A narrow Rust JNI adapter verifies storage reopen; five instrumentation tests pass on the dev phone. No production UI was added. Two pre-existing Compose compilation issues and stale build outputs were corrected so the test APKs build.

A native `SGHI` initial envelope now contains both PQXDH profile-2 ciphertext and the first empty Triple Ratchet confirmation. The recipient authenticates both before committing its one-time prekey consumption, session and initial plaintext, allowing an immediate reply after one delivery and after restart. Context hashes the complete inner initial ciphertext. Native initial text is bounded at 65,374 bytes to preserve the mailbox ceiling. Raw initial packets from earlier experiments remain stored but cannot be retransmitted; no silent rewriting occurs. Tests reject every altered confirmation byte, substitution across handshakes, omitted/advanced confirmations, legacy retransmission and oversized initial text.

All 207 Rust workspace tests and Clippy with warnings denied pass. The five Android instrumentation tests passed before the final native-envelope-only change; the Android platform adapter itself is unchanged. Backend completion, independent review, verified peer/device routing, automated scheduling, live-event/retention/media integration, other platform acceptance and group/federation design remain open. No secure-messaging or deployment-readiness claim is made.

### Prekey publication, retirement and durable claims

The server now returns a JSON publication receipt containing the prekey ID and original absolute expiry. Exact retries preserve that expiry even after assignment; the earlier empty response is replaced. Native schema 10 freezes public metadata after private-key commitment, authenticates local slot-to-bundle routing and persists publication receipts. Lost responses retry the same key. Private keys remain available for one full seven-day mailbox lifetime beyond publication expiry, then retire in authenticated atomic batches of at most 16. Consumed/retired slot IDs remain tombstones; no regeneration or republication occurs. This bounded retention assumes an honest sender starts within bundle validity and a trustworthy local wall clock. Automatic replenishment and last-resort prekeys remain separate work.

Native schema 11 persists claim IDs with immutable recipient-device and independently selected encryption-identity fields before HTTP assignment. Responses must pass signature, expected-identity and KEM-identifier checks before the exact bundle commits. An ambiguous response retries the original request rather than consuming another key. Claims are bounded at 64 pending and 4,096 lifetime IDs; abandonment retains a tombstone. Initial creation checks the stored claim and expiry in the same transaction as the session/outbox handoff. Expired bundles cannot start new sessions, while an already committed initial packet remains retryable. Existing low-level and claimed handshakes share one implementation.

Seven additional native tests exercise actual HTTPS publication/claiming, lost-response recovery, delayed incoming messages, bounded retirement, concurrent preparation, injected storage failures, authenticated expiry-index tampering, altered signatures/KEM IDs, claim limits, abandonment and schema 9/10 migration. All 214 workspace tests and Clippy with warnings denied pass, and the server container builds. Peer verification/device binding and automatic inbound routing are still gates; a server response alone never establishes the expected peer identity.

### Local retention deadlines and complete claimed-session handoff

Self-review against Signal Sesame revision 2 (2017-04-14, https://signal.org/docs/specifications/sesame/) exposed an unnecessary dependence on synchronized phone/server clocks in private-prekey retirement. Native schema 12 replaces that retirement index with an authenticated local deadline: after publication acknowledgement, retain for the complete advertised lifetime plus the seven-day delivery window. Exact retries never extend an established local deadline. Earlier acknowledged records have no inferred deadline and remain protected until a conservative new deadline is recorded. A controlled-clock migration test deliberately separates server and local clock origins by decades. Local wall-clock integrity still matters; this is not a claim of full Sesame implementation.

Claimed-session creation now freezes the recipient device and original delivery expiry in the same transaction as the ratchet state, initial ciphertext and one-time-key reservation. The existing transport preparation implementation is shared. Retrying a claimed initial message cannot redirect it, extend its expiry or exceed the retained prekey delivery window. Failure at either outbox or delivery insertion rolls back the entire handoff. The HTTPS integration test now carries publication, claim, initial delivery, immediate reply and durable acknowledgements through the actual server.

All native tests and Clippy with warnings denied pass after this change; the added migration regression brings the workspace's tested total to 215 (full 214-test baseline plus this native increment). The previous schema-10 wall-clock retirement rule above is superseded by this local deadline. The server container's operational smoke test also passed before this client-only change.

### Signed device bindings and verified peer-bound sessions

Server schema 9 stores immutable device statements. `PUT /client/v0/device-binding` publishes the authenticated device's server, username, account, device and encryption identity; `GET /client/v0/devices/{id}/binding` requires native authentication and a contact relationship in either direction, or the same account. The server enforces account/device fields and immutable encryption identity, including prekey agreement. Clients verify the XEdDSA signature; server publication alone never establishes peer trust. The signed encoding is `Sigil/device-binding/v0\0`, server length (u16 BE)/ASCII server, username length (u8)/ASCII username, account/device/identity (32 bytes each), then signature (64 bytes). Fingerprints hash the signing bytes, excluding randomized signature bytes. `device_binding: [0]` advertises this experimental directory API.

Native schema 13 encrypts the exact own publication and peer trust records. Explicit confirmation requires the full independently compared fingerprint. New devices are unverified; changed bindings are quarantined, and replaying the original does not dismiss the change. Blocking preserves history. Native schema 14 binds session checkpoint AEAD to the peer reference, so deleting or substituting the clear peer lookup cannot downgrade an established session. Peer-aware claims and both initial directions commit the binding with the handshake. New sends, pending sends and receives recheck peer status; frozen deliveries must address that peer's device. Low-level APIs with manually supplied expected identities remain for evaluation and independently trusted callers. Full device linking, QR/emoji UX, replacement approval and federation are not implemented.

Nine added protocol/server/native tests verify signature coverage, immutable publication and identity agreement, access controls/revocation, historical migration/restore, explicit confirmation, concurrent key changes, peer-index tampering, destination substitution and transaction rollback. The complete workspace reached 224 passing tests and clean Clippy before the subsequent signer/routing changes.

### Canonical XEdDSA signing and hash-state cleanup

Self-review against XEdDSA revision 1 found a noncanonical positive-orientation nonce scalar encoding and unprotected local secret copies in the earlier signing dependency. That dependency is removed. The replacement uses existing curve25519-dalek primitives, canonical reduced scalars, branchless orientation selection, fresh 64-byte OS randomness and zeroizing owned secrets. SHA-2 0.11.0 with zeroize and HMAC/HKDF 0.13.0 preserve all saved KDF/MAC outputs while improving hash-state cleanup. Independent C/OpenSSL/libsodium fixtures cover 32 keys across both orientations, and existing PQXDH fixtures continue to verify. C checks also pass under address/undefined-behavior sanitizers. The full workspace passed 226 tests and Clippy after this change. Exact sources, fixture hashes and remaining dependency/compiler erasure limitations are in `TripleRatchet.md`; this self-review is not the requested independent audit.

### Durable verified incoming delivery

Native schema 15 adds an authenticated incoming journal and peer-session lookup index. Initial packets route through the recipient's authenticated prekey metadata and the explicitly verified sender binding; successful initial acceptance derives a stable local session ID. Established packets try at most eight peer-bound sessions, without mutating failed candidates. Multiple successful candidates are rejected as ambiguous. Corrupt local checkpoints fail explicitly. Peer-aware creation refuses a ninth session atomically, preserving existing sessions and the durable pending claim. This bounded approach does not yet implement Sesame active/inactive convergence or session retirement.

`accept_delivery` commits plaintext, ratchet/prekey changes and the acknowledgement journal together. `receive_mailbox_online` attempts at most 16 deliveries, retaining individual failures without trusting unknown peers or acknowledging bad content. `acknowledge_incoming_online` verifies committed local content and retries at most 16 journaled sequence deletions; an HTTP success followed by a failed local write safely retries after restart. Journal metadata binds the own device, sequence, peer, local session, message ID, complete packet digest, original expiry and acknowledgement status. It retains at most 4,096 lifetime entries; compaction/retention remains work. Server sequence, sender and expiry metadata are routing hints, not end-to-end authenticated application fields. No new wire format or enabled encrypted-event contract is claimed.

Four new native tests cover actual HTTPS initial/reply routing, different local session IDs, injected initial and established receive failures, acknowledgement ambiguity/restart, blocked/unknown senders, ciphertext and metadata substitution, forged journal rows, migration without invented receipts, eight concurrent sessions and atomic ninth-session rejection. Production UI and the independent audit remain gated on backend completion.

### Mailbox scan progress without acknowledging rejected content

Self-review found that a fixed first-page poll could let sixteen rejected deliveries hide all later messages. The mailbox API now accepts a bounded nonnegative `after` sequence cursor while preserving the same authenticated recipient scope, 16-item limit and acknowledgement semantics. Unknown, duplicate, malformed and overflowing query parameters fail. Native schema 16 encrypts a compare-and-swap scan cursor bound to the own device. Each poll advances past attempted items and resets to zero after reaching the end, so rejected/unverified messages are revisited without blocking later content. A failed cursor write cannot undo or duplicate already committed message acceptance. Cursor metadata never authorizes deletion.

The HTTPS regression queues sixteen invalid packets before a valid initial message, restarts across page boundaries, injects a cursor-write failure after acceptance, and verifies later delivery, exact local retry, safe acknowledgement and revisiting of rejected entries. Migration from schema 15 preserves the incoming journal while starting a new scan. Server tests cover recipient isolation, non-consuming pagination and invalid query encodings. The complete workspace passed 231 tests and Clippy after this increment.

### Skipped-key retention under permanent packet loss

Self-review identified a liveness problem in both ratchets: permanently lost messages filled the 128-key skipped cache, after which later gaps could stall the session indefinitely. Both caches now share bounded FIFO retention. Adding a new skipped key evicts the oldest retained key when full; the classical and sparse KDF work budgets remain limited to 128 newly skipped keys per incoming packet, including previous/current-chain work together. Post-quantum epoch pruning remains in force. Eviction happens only in candidates and becomes durable only after the complete message authenticates and the client transaction commits. It cannot be triggered persistently by a forged ciphertext.

Checkpoint versions `SGRS` 2 and `SGTS` 2 preserve cache insertion order without increasing the encoded size limits. Version-1 checkpoints remain readable with every existing key retained; because their original chronological order was not recorded, their existing encoded order supplies the initial eviction order. New writes explicitly use version 2. This does not alter the wire/KDF profile or introduce live-database rollback recovery. Timer/event-age expiration beyond the capacity and PQ-epoch policies remains a review item.

Tests sustain 2,000 one-way transmissions with 1,000 permanently lost packets, preserve the most recent 128 delayed packets across repeated checkpoints, and verify that forged ciphertext cannot evict the oldest key. Additional checks cover version-1 migration and a combined old/new-chain gap that exceeds the per-packet work budget. Signal Double Ratchet revision 4 sections 8.4 and 8.7 provide the rationale for bounded deterministic skipped-key deletion; the exact Sigil policy remains subject to independent review.

### Direct-text event binding

`protocol::event::Text` defines a bounded versioned inner event containing the logical message ID, stable account-pair conversation reference, both complete device-binding fingerprints, timestamp and UTF-8 body. It is checked inside the incoming transaction after cryptographic authentication and before commit, including cached retries and acknowledgement eligibility. `start_claimed_text` and `send_text` supply the application binding; the latter freezes delivery metadata with ciphertext/state in one transaction. Raw byte-oriented evaluation APIs remain explicit and do not silently acquire these guarantees. Maximum text size is 65,224 bytes for both initial and later messages. The complete encrypted-event capability remains disabled. Exact encoding and limitations are in `Events.md`.

One protocol and four native tests cover strict framing/UTF-8, maximum Unicode text over HTTPS, exact timestamp/body retries, delivery-write rollback, substituted outer IDs and correctly encrypted content with wrong application bindings. All 239 workspace tests and Clippy pass after this increment. The prior Android run passed 74 crypto and 30 native tests before this event-only change; repeat validation is still required for the latest native application path.

`GroupsFederation.md` records the separate authority, credential-issuer, membership-concurrency, key-rotation and federation-transport requirements against primary sources. It explicitly does not mark the group/federation design gate complete or incorporate Signal implementation code.

### Logical text retries and transactional recovery capture

Native schema 17 adds a bounded encrypted incoming logical-event ledger. Reusing a sender's message ID with different authenticated content or a different local session fails before message state commits. Cached accepted messages establish the ledger before acknowledgement, including migration from schema 16. Two tests cover fresh-prekey cross-session replay, altered content, migration, ledger corruption and injected writes. Cross-session retransmission remains unsupported until the session-replacement contract is defined.

Configured recovery now captures typed outgoing and newly accepted incoming ordinary text in the same transaction as message/session state. History IDs bind the conversation, authenticated author identity and logical message ID. Direction is account-relative. Shared retention logic preserves later revisions and tombstones on retries, rejects mismatched account scopes and pending imports, and permits new records while an earlier upload snapshot is frozen. It does not backfill earlier history or implement local deletion/cancellation.

Two further tests inject archive-write failures at all four initial/reply send/receive boundaries, verify exact retries and rollback, publish successive snapshots, and restore both directions over HTTPS onto a freshly reauthorized device without importing live cryptographic state or peer verification. All 243 workspace tests and warnings-denied Clippy pass. The latest dev-phone run passes 74 crypto and 38 native client tests; the existing five Android Keystore instrumentation checks also passed. The current Docker build and operational smoke pass. Backend completion, the requested independent audit and production UI remain pending.

### Multi-epoch tampering and backup schema rejection

The existing every-byte Triple Ratchet mutation test now spans alternating asymmetric traffic, both sender roles and epoch parities, covering every emitted Braid message kind. After rejected mutations it compares the complete plaintext checkpoint, including both ratchets, skipped keys and partial fragment decoders, then reopens the checkpoint and accepts the legitimate packet. The run completes at least four PQ epochs. Standalone Ct1Ack is recognized by framing but the current sender emits the combined EkCt1Ack form.

Self-review reproduced a restore weakness: a modified SQLite backup containing a `BEFORE UPDATE` trigger with `RAISE(IGNORE)` could bypass device credential invalidation without reporting an SQL failure. Startup, backup and restore now reject triggers and views, which no supported server schema uses. `trusted_schema=OFF` is set before schema/migration reads. Regression tests reject the bypass and a view masquerading as configuration, preserve the source and leave no published destination. This is a concrete schema defense, not authentication of arbitrary backup files or protection against an attacker continuously controlling the server's private filesystem. SQLite's [database-file hardening guidance](https://www.sqlite.org/security.html) provides the rationale for restricting unused schema execution features.

### Bounded discovery of recovered history

`recovery_records(after)` returns at most 16 fully authenticated committed records, including tombstones, through a primary-key range scan. A fresh device can discover its recovered records without already knowing their IDs. Staged imports remain invisible until completion, and a corrupt entry fails the whole returned page. The cursor orders stable IDs rather than timestamps and is not a change feed; callers restart after archive changes. Tests cover zero-valued IDs, pagination across restart, deletions, substituted encrypted records, and discovery only after complete HTTPS recovery. The plaintext `Text` type also no longer derives `Debug`, avoiding an unnecessary route for accidental content logging.

Native client startup now rejects triggers and views before opening live state, and both stores enable SQLite defensive mode. An added native regression preserves the sealed session when rejecting a state-suppression trigger or added view; after explicitly removing the synthetic schema modification, the original ciphertext retry remains unchanged. All 247 workspace tests and warnings-denied Clippy pass after these changes.

### Final overnight validation and persistence review

The ratchet delivery test now covers eight reproducible schedules and 8,000 messages with sender bursts, permanent loss, reordering, forged ciphertext, duplicate delivery and encrypted checkpoint reloads. Each schedule reaches at least five PQ epochs. All 247 workspace tests, warnings-denied Clippy and formatting checks pass. The latest dev-phone run executes all six native test binaries: 74 crypto tests plus 84 client tests (38 unit, 14 durability, 14 handshake, 7 recovery, 11 transport), including abrupt process exits. Three ignored integration helpers are invoked by their parent tests; the separate crypto fixture generator remains intentionally ignored. The five Android Keystore instrumentation tests passed earlier. The current Docker image and operational smoke pass.

A separate synthetic probe confirmed that a retired encrypted session checkpoint remains extractable from a live WAL and can be opened with the current client storage key and original revision binding. This makes durable key erasure a concrete open review item, beyond the existing warning about physical deletion. It requires endpoint key compromise, not merely a malicious homeserver. `TripleRatchet.md` records the boundary and the crash/concurrency/platform questions that must be resolved before making stronger forward-secrecy claims for deleted content. The probe used disposable synthetic storage outside the repository; no personal data was accessed.

### Deployment persistence and portable backups

Backend deployment work resumes with production UI paused and independent audit still pending. Existing server database paths now reject nonregular files and group/world permissions before SQLite opens them. Tests verify rejection preserves bytes and an explicitly corrected private file opens normally; the historical schema-one fixture now creates its database privately.

A new actual-process test verifies acknowledged mailbox/recovery writes and exact retries across SIGKILL, followed by offline CLI backup/restore with credential revocation and preserved recovery ciphertext. `server/tests/container.sh` repeats the lifecycle using disposable persistent Docker volumes, non-root execution, a read-only root filesystem and the supplied deployment resource limits. It also checks backup exclusion while the server holds the storage lock and account reauthorization after restore.

The container test exposed backups retaining WAL journal mode: a writable-source restore passed while a read-only backup mount failed. Backup now switches the completed destination to DELETE journal mode and checks close success before synchronizing it. The regression checks standalone-file headers and absence of WAL sidecars; the container acceptance now restores successfully from a read-only mount. All 248 workspace tests, warnings-denied Clippy and formatting checks pass. `Deployment.md` records operator procedures and clearly separates these operational checks from remaining backend, disaster-recovery, actual-hardware and independent audit gates.

### Trusted-checkpoint repair and history-only handoff

Native recovery now permits explicit repair of an operator-restored server head only when its generation and manifest exactly match the surviving archive's authenticated local anchor. Fresh/unanchored clients, older or conflicting heads and pending operations fail closed. Authorization is sealed with the anchor, survives restart and permits only a complete new local snapshot through the existing object acknowledgements and compare-and-swap publication. Imports remain blocked during repair. The successful local publication transaction clears authorization; lost responses and failed commits retain the exact pending ciphertext for retry. Existing sealed archive states remain readable, while older readers reject the repair marker.

Because operator restore revokes credentials, `copy_recovery_history_to` transfers only authenticated committed history, its independent recovery secret and trusted anchor into a freshly enrolled client for the same account. The destination must contain no archive or live messaging/verification state. It uses the destination storage key, checks each record and publishes atomically without modifying the source. It refuses pending source operations and does not carry repair authorization. No live identity, session, prekey or verification state is copied. The replacement explicitly obtains repair authorization through its own authenticated HTTPS connection before preparing/uploading the successor.

The actual server-store backup/restore test now repairs through a newly reauthorized account, preserves a later local tombstone and new message, retries after a lost publication response, and imports the repaired snapshot into a fresh archive. Additional tests cover stale/forked heads, missing anchors, pending-operation preservation, injected authorization/commit failures, and HTTPS handoff with revoked old credentials, different local wrapping keys, wrong account scope, failed destination writes and absence of live state. All 250 workspace tests, warnings-denied Clippy and formatting checks pass. The prior Android/device and Docker runs precede this native-only increment; they were not repeated here.

This closes the exact-trusted-checkpoint repair path, not general disaster recovery. Older server backups, lost local anchors, interrupted pre-restore uploads, competing repairs and user-facing orchestration remain open. No UI changes or independent audit were started.

### Interrupted-upload repair after server restore

Exact-trusted-checkpoint repair now accepts an already prepared direct-successor upload. It authenticates the pending manifest's link to the unchanged trusted anchor and resets every object acknowledgement in the same transaction as repair authorization. The intended head and randomized ciphertext are never regenerated or discarded. Restore can lose previously acknowledged objects; they must all be sent again before publication. Failed authorization rolls acknowledgement resets back. Repeated explicit authorization resets progress again, while normal upload retries simply resume.

History-only handoff now also transfers a pending direct successor. It traverses the authenticated frozen manifest, pages and record references, validates every ciphertext, rejects missing/corrupt/extra staging objects and queues the same bytes with no acknowledgements in the destination. Current retained records, including edits newer than the frozen snapshot, transfer separately as before. The source and its acknowledgements remain unchanged. Pending imports and missing trusted anchors remain blocked. Post-freeze edits enter the following snapshot rather than changing an ambiguous upload.

Expanded regressions perform a real server backup, upload and acknowledge successor objects after that backup, restore the earlier database, confirm those objects are absent, and repair with the exact original ciphertext across restart and a lost publication response. HTTPS tests transfer the pending snapshot into a newly reauthorized client using a different wrapping key and reject damaged staging/failed destination writes atomically. All 250 workspace tests, warnings-denied Clippy and formatting checks pass. This native-only increment did not repeat device or Docker runs. Restored heads equal to an unacknowledged successor, older/forked backups, pending imports, missing anchors and general recovery orchestration remain separate gates.

### Resolve publication acknowledged only by the restored server

Restore reconciliation now also accepts a server head matching the exact locally prepared direct successor of an existing trusted anchor. The client authenticates its stored manifest and predecessor link, then atomically advances the anchor, clears the resolved upload staging and retains repair authorization. It does not accept a server-selected competing successor. Current local records, including edits and tombstones made after the frozen snapshot, are untouched and enter the next repair snapshot. Handoff may have reset object acknowledgements; they are not needed to resolve an exact published manifest, and the new repair snapshot uploads all its objects before publication.

Two parameterized regression scenarios cover the trusted-predecessor and already-published-successor cases without duplicating their fixtures. The server-store scenario takes a real backup after publication while the client still has its pending upload, restores and reauthorizes, resolves the publication, then verifies a fresh import includes a later deletion and new record. It rejects a forked head, damaged local manifest and injected commit failure while preserving the prior anchor and staging. The HTTPS scenario resolves the same condition after history-only handoff under a different wrapping key and verifies repair across restart. All 252 workspace tests, warnings-denied Clippy and formatting checks pass. No device/Docker rerun or UI changes were needed for this native-only change.

Older/forked restored checkpoints, competing repair, pending imports, missing trusted anchors (including an initial ambiguous upload) and general recovery orchestration remain open. Backend readiness and independent audit are still pending.

### Proven-ancestor repair without client rollback

Native schema 18 retains a bounded ledger of at most 64 encrypted checkpoint manifests when publication or import commits, including resolution of a pending publication. Restore reconciliation walks authenticated predecessor links from the trusted anchor. Only an exact ancestor within that window can authorize older-backup repair; missing, corrupt, forked or out-of-window proofs fail closed. Migration retains history and the current anchor but cannot fabricate prior proof. History-only handoff authenticates and copies the bounded ledger under the same account scope.

The newest client anchor and current records never roll back. Sealed repair state separately binds the restored server CAS base. A new or already pending snapshot continues the newest local anchor and advances above it, preserving exact pending ciphertext and resetting object acknowledgements. `PublishHead.restore_generation` is an optional restore-only generation jump: it requires explicit acknowledgement, a nonzero exact restored CAS base and a strictly higher generation within the signed storage range. Unflagged heads cannot authorize it; exact retries remain valid after successful publication clears the flag. Normal publication and older clients omit the field and retain successor-only behavior. Server schema remains 9; deploy the updated server image before using this new repair request.

Regressions cover actual older-backup restore with later local deletions and new records; HTTPS generation-jump repair after history-only handoff; a 64-link proof boundary; corrupted and forked proofs; schema-17 migration without invented ancestry; unauthorized/unflagged/out-of-range/stale server requests; account isolation; and exact retry across server restart. All 255 workspace tests, warnings-denied Clippy and formatting checks pass. The rebuilt Docker image passes persistent restart, exact retries, backup locking, read-only backup restore, credential revocation and retained-ciphertext acceptance. Device tests were not repeated for this increment.

Unproven older backups, missing trusted anchors, pending imports, competing repairs, multi-device catch-up across unavailable intermediate snapshots and user-facing recovery orchestration remain open. No UI work or independent audit was started; full backend deployment readiness remains pending.

### Durable multi-generation recovery catch-up

Anchored imports now stage a target up to 64 generations ahead and authenticate every predecessor link back to the exact trusted checkpoint. `Download::Manifest` requests one missing ancestry object per step; each validated object is persisted, so restart resumes the same pinned target. All page/record/import-finalization paths recheck chain completion. The trusted anchor and visible history remain unchanged until the entire latest snapshot commits, together with the bounded manifest ledger. Corrupted, substituted, forked, missing or over-limit chains fail closed; cancellation removes staging only. Distant imports cannot discard an ambiguous pending upload.

Repair uploads now also queue the available original encrypted ancestor manifests from the local ledger. A restored server may have lost those objects even when the repairing client retains them. They use the same bounded queue and acknowledgements as snapshot objects and must be uploaded before publishing. Pending snapshot ciphertext remains unchanged; the extra proof objects are authenticated again during history-only handoff. This allows a lagging reader to catch up across a repaired generation jump without downloading obsolete snapshot records or disabling rollback protection.

The new regression authenticates a multi-link chain across repeated client restarts, blocks history access and retention during staging, rejects wrong/corrupt objects and a completed fork, injects final commit failure, checks cancellation and enforces the 64-link bound. Expanded real HTTPS tests restore/re-publish ancestor proofs, reauthorize a lagging reader, establish its earlier trusted anchor and catch up to the repaired snapshot across restarts with current deletion tombstones. Existing repair tests verify original pending objects are byte-identical while proof objects are added, and the 64-proof repair uses multiple bounded upload batches. All 256 workspace tests, warnings-denied Clippy and formatting checks pass. Server/container behavior was unchanged; native device tests were not repeated.

Missing proofs beyond the retained window, missing trusted anchors, competing repairs/pending uploads and user-facing recovery orchestration remain open. Full backend readiness and independent audit remain pending; no UI work was started.

### Resolve a competing direct-successor repair

A restore-authorized client previously rejected every import, leaving it stuck when another repair won publication. It now accepts an unflagged competing direct successor only after authenticating the manifest's link to its exact local anchor. Handover atomically replaces losing upload staging and clears both repair authorization and the obsolete restored-server CAS base. Current retained records, including post-freeze edits and tombstones, remain intact; ordinary monotonic import merges the winning snapshot before a new successor can be prepared. Failed handover leaves the original pending ciphertext and authorization unchanged. Forked or distant repair winners remain blocked.

Two real HTTPS scenarios cover ordinary restore repair and an older-backup generation jump. Another writer wins CAS; the loser receives 409, preserves its pending state, rejects a signed manifest with the wrong predecessor, survives an injected handover failure, imports the winner across restart, preserves its local deletion, incorporates competing history and successfully publishes the merged successor. The generation-jump case also verifies that the old CAS base no longer contaminates normal publication. Tests respect the existing server Retry-After response and resume persisted upload progress without weakening rate limits.

All 258 workspace tests, warnings-denied Clippy and formatting checks pass. This native-only change did not repeat device or Docker runs. Distant/divergent competing repairs, missing trust/proof, and user-facing recovery orchestration remain open, alongside the other backend deployment gates. No UI changes or independent audit were started.

### Verify distant competing publications before replacing an upload

Native schema 19 adds bounded competing-proof staging separate from the existing upload queue. The encrypted target binds the current sealed archive control state; at most 64 predecessor manifests must authenticate back to the exact trusted anchor. The pending upload, acknowledgements, repair authorization and current records remain intact during verification. Cancellation removes only proof staging. Another committed control-state change invalidates the candidate, while local record edits and upload-object acknowledgements can continue. History-only handoff refuses an active candidate.

Only complete proof permits an atomic handover into ordinary import staging, clearing the losing upload and obsolete repair state. Import still validates all snapshot references and commits history and the anchor together. Failed handover preserves the original queue and proof progress. The HTTPS download adapter drives this path after an otherwise blocked distant conflict, fetching one required ancestry manifest per step and resuming after restart. Missing, over-limit and forked proofs fail closed; a stale candidate remains explicitly cancellable.

Regressions verify cancellation preserves byte-identical pending ciphertext, wrong/forked/out-of-window proofs leave the upload intact, injected handover failure rolls back, stale control state cannot be overwritten, later tombstones survive merge and successful import incorporates competing records. A real HTTPS test covers an older-backup repair whose winner advances again before the loser reconciles; the loser verifies the full chain across restart and publishes its merged successor. Rate-limited upload steps respect Retry-After and resume persisted progress. All 260 workspace tests, warnings-denied Clippy and formatting checks pass. Server/container behavior was unchanged; device tests were not repeated.

Missing or out-of-window proofs, missing trusted anchors, branches not connected to the trusted anchor and user-facing recovery orchestration remain open. Other backend deployment gates and the independent audit remain pending; no UI work was started.

### Explicit obsolete-session retirement

Backend review identified the eight-session-per-peer cap as a practical lifecycle blocker: obsolete sessions had no retirement path. Native schema 20 now supports explicit `retire_session`. It refuses queued outgoing packets, authenticates the checkpoint without requiring the peer to remain unblocked, and atomically replaces live state with a separately bound encrypted retirement tombstone at the next revision. Idempotent retries verify that tombstone. Live send/decrypt paths reject retired sessions, and routing plus peer slot accounting exclude them.

History, delivery receipts and session/initialization ID records remain. Retired IDs cannot be reused, and the existing 1,024 lifetime-session limit still applies. This does not implement automatic session selection, convergence, expiry or replacement, and it does not claim durable forensic erasure of previous WAL/snapshot/flash copies.

Tests cover refusal with pending ciphertext and exact retry preservation, injected retirement-write failure, restart/idempotence, rejection of new traffic and ID reuse, retained incoming/outgoing history and receipts, tampered tombstone bindings, and freeing an active peer slot even while the peer is blocked. All 262 workspace tests, warnings-denied Clippy and formatting checks pass. Server/container behavior was unchanged; device tests were not repeated. No UI changes or independent audit were started.

### Explicit expiry of queued deliveries

Native schema 21 adds authenticated local delivery expiry. `expire_delivery` requires a prepared request and a caller-supplied trusted clock at or beyond its frozen deadline. It atomically records an expiry marker and clears pending ciphertext without changing ratchet state, history or ID commitments. This unblocks later queued requests and permits explicit session retirement when no packets remain. Unprepared packets remain protected. Exact retries of expired messages report expiry; changed content still conflicts.

Local expiry does not establish non-delivery. A matching authenticated server receipt can supersede it even after retirement; concurrent receipt and expiry writes serialize with acceptance winning. No server receipt is fabricated. Tests cover deadline refusal, restart/idempotence, unchanged checkpoints/history, queue progress, retirement, late receipts, both transaction-write failures, marker corruption/substitution, schema-20 migration and concurrent acceptance. All 265 workspace tests, warnings-denied Clippy and formatting checks pass. Device/container runs were not repeated for this native-only increment. Automatic expiry scheduling, trusted-clock policy and active-session convergence remain pending; this does not complete the backend or its independent audit.

### Bounded send-worker expiry

The HTTPS send worker now resolves frozen delivery deadlines within each batch instead of stopping until callers expire individual IDs. It inspects at most 16 queued packets, validates all selected entries, and commits authenticated expiry together before transmitting remaining requests. Unprepared or corrupt successors and failed expiry writes prevent partial cleanup. Shared expiry and queue-reading helpers replace duplicated logic; no dependencies or schema changes were needed.

`send_pending_online` returns `SendProgress { accepted, expired }`, separating local retry cessation from server acceptance. Callers must consider both counts when deciding whether work remains. Transport or receipt-storage errors may follow durable expiry or earlier acceptance; restart resumes those transitions and retains exact unsent ciphertext. Late matching receipts still supersede expiry. The supplied clock remains trusted caller input, and background scheduling/session convergence remain open.

A real HTTPS regression processes 17 expired packets and one live successor across bounded batches, injected expiry-write failure, restart, server acceptance followed by failed local receipt storage, and exact retry without duplicate delivery. It verifies unchanged ratchet checkpoints, recipient decryption through skipped messages and subsequent retirement. Another regression rejects unprepared/corrupt successors without partial expiry. All 267 workspace tests, warnings-denied Clippy and formatting checks pass. Native-only changes did not repeat device/container runs. Backend deployment gates and independent audit remain unfinished; no UI work was started.

### Durable active-session selection

Native schema 22 adds an authenticated active-session reference per verified device. New peer-bound sessions select themselves. Newly authenticated established traffic selects its receiving session atomically with checkpoint advancement, event validation and history/journal persistence. Replayed initial packets and cached incoming deliveries do not change selection. Retirement clears a matching active reference in its existing transaction, including while a peer is blocked; it does not automatically revive an older session. Migration does not invent a selection for existing sessions.

`send_peer_text` selects a session for new text while exact retries follow the original frozen delivery record, checking peer binding and preserving ciphertext across selection changes. Existing explicit-session sending and peer-aware sending share one transaction helper. The reference binds the peer fingerprint and fails closed on corruption or changed trust. No new dependencies were introduced.

Two regressions exercise multiple real prekey-established sessions, activation through authenticated replies, selection-write rollback, corrupted packets, cached and resequenced initial replay, restart, exact retries after switching, blocked peers, schema-21 migration, corrupted stored references and atomic retirement cleanup. All 269 workspace tests, warnings-denied Clippy and formatting checks pass. Native-only changes did not repeat device/container runs.

The implemented selection rule follows Sesame revision 2 sections 3.2–3.4, but full session lifecycle remains incomplete: initiating-envelope retransmission/peer confirmation, lost-session retry requests, approved device reconciliation, multi-device fan-out and automatic stale-session retirement remain open. The full backend scope remains in force before the planned audit; no independent audit or UI work was started.

### Repeated initiating envelopes and authenticated peer confirmation

The previous native envelope carried application text in PQXDH and only an empty ratchet bootstrap; subsequent packets omitted the handshake even before peer confirmation. Losing the first packet therefore orphaned later traffic. Envelope version 2 instead encrypts empty PQXDH content and carries application text in the attached Triple Ratchet packet. Native schema 23 retains the exact sealed handshake header. New sends attach it until authenticated peer traffic commits; server acceptance never confirms the session. Confirmation derives from existing authenticated ratchet state and survives checkpoint reload without another status flag.

The receiver routes repeated envelopes by a stable digest of the handshake header plus its device/peer context. A later packet can establish the session within existing skipped-key derivation limits, consuming the prekey only once. Subsequent envelopes validate the retained header, slot and expected sender and advance the same session. Delayed earlier packets use skipped keys. Fresh repeated-envelope messages update active selection; cached deliveries do not. Queued envelopes remain exact retries after confirmation, while new packets omit the header. Retirement removes retained headers transactionally.

This replaces the version-1 bootstrap implementation and preserves the existing mailbox and text limits. Old version-1/raw initial packets remain history-only; migration cannot invent missing headers for unconfirmed older initiators. No ciphertext is silently regenerated. Existing established ratchet checkpoints and cryptographic primitives are unchanged. The protocol framing change requires matching experimental clients; it does not enable the complete encrypted-event capability.

Real HTTPS tests lose the first packet after server acceptance, establish from its successor, decrypt the delayed first packet in the same session, inject confirmation-write failure, restart both sides of confirmation, preserve queued retries and accept wrapped/bare reordered traffic. Other checks cover header tampering, failed header persistence, schema-22 migration without invented state, substitution/reordering, maximum framing and checkpoint confirmation after failed authentication. All 272 workspace tests, warnings-denied Clippy and formatting checks pass. Device/container runs were not repeated; actual HTTPS mailbox transport was exercised. Full backend scope remains required before audit, including general lost-session retry requests and recovery beyond the bounded skipped-key window. No audit or UI work was started.

### Freeze the unconfirmed initiating delivery window

A fresh verification run reproduced the prior 272 passing workspace tests. Follow-up implementation review found that repeated initiating envelopes could acquire later delivery deadlines indefinitely even though the original prekey-retention window was bounded. Native schema 24 now seals the first prepared expiry into the sender's retained handshake header. Peer-aware initiation freezes that deadline atomically with initial delivery preparation. Later unconfirmed default expiries are capped; explicit extensions fail. An expired new text send rolls back the ratchet, outbox and retained history together. Authenticated peer confirmation permits normal new-delivery lifetimes, while exact retries keep their original deadlines.

Earlier sender headers with existing deliveries lack an authenticated deadline. Migration refuses to invent one from mutable queue ordering or reset it to the current clock; those sessions need a fresh handshake or authenticated peer reply before new delivery preparation. Existing frozen requests remain retryable. No dependency or wire-envelope change was introduced.

Two added regressions cover deadline capping, attempted extension, rollback after expiry, release only following authenticated peer traffic, corrupt headers, failure during header/delivery writes and schema-23 migration without deadline reset. All 274 workspace tests, warnings-denied Clippy and formatting checks pass. Device/container runs were not repeated. Code remains local and uncommitted; these tests support specific implemented behaviors, not independent cryptographic assurance or full backend completion. The audit remains deferred until the full agreed backend is implemented.

### Signed retry requests and durable fresh-claim staging

Native schema 25 and a bounded versioned retry-control frame implement the request/authentication portion of lost-session recovery. The verified recipient signs the original packet's message ID, both device fingerprints and a frozen expiry. A deterministic request ID prevents duplicate requests from renewing the same record; the exact randomized signature is persisted before HTTPS transmission. Controls contain metadata only and are signed, not encrypted, as explicitly documented in TripleRatchet.md.

Sender acceptance checks the signature, current trust, routing, expiry, original delivery recipient and retained text's sender/recipient fingerprints. Only then does one transaction stage a fresh peer-bound prekey claim and accepted-request record. No session is replaced or plaintext resent. Stored requests can be reloaded after restart with trust/expiry revalidation. A retired original session can still supply authenticated retained delivery evidence. Outgoing and accepted ledgers and pending claims remain bounded.

Tests cover canonical/truncated/oversized frames, every modified signed byte, wrong routing, signed unknown-message requests, expiry, blocked peers, immutable duplicate requests, failed request/receipt/claim-ledger writes, restart, exact HTTPS retry after server acceptance and successful use of the staged prekey claim without changing the selected session. All 277 workspace tests, warnings-denied Clippy and formatting checks pass. Device/container runs were not repeated; real HTTPS mailbox transport was exercised.

Lost-session recovery still needs automatic control dispatch/acknowledgement, re-encryption with distinct transport identifiers, logical-message deduplication, deletion/retention checks at resend time, retry-chain bounds and completed-control cleanup. This is a completed request-staging increment, not completed message recovery. Full backend scope remains in force before audit; no UI or audit work was started.

### Fresh-session text resend with stable logical identity

Native schema 26 completes the explicit resend step following an accepted signed retry request. The staged fresh prekey claim and existing initiation transaction now create/select a new session and queue the unchanged canonical text. The signed request ID is the new transport ID; the original logical ID, body and timestamp remain unchanged. Exact retries after restart reuse committed ciphertext. Shared initiation code replaces the former method body rather than duplicating it; no new wire frame or storage table is needed. The schema gate prevents older clients reopening cross-session resend history they cannot validate.

Recipient acceptance requires an authenticated local request chain of at most three hops. The logical-event ledger rejects changed plaintext and flags exact duplicates, including delayed originals, while recovery retains one logical record. Configured recovery tombstones or changed bodies block stale resend preparation, revalidation in the standard HTTPS worker, and incoming acceptance before ratchet/prekey state commits. This does not implement general message deletion or recall packets already in flight.

Three added regressions cover real HTTPS delivery, injected write rollback, unchanged active selection after failure, restart-stable ciphertext, outer-ID substitution, delayed-original deduplication, one archive record, sender deletion after queuing, receiver deletion before acceptance, and rejection of a fourth resend without staging another claim/session. All 280 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-resend-workspace.log` and `/tmp/sigil-resend-clippy.log`. Device/container runs were not repeated.

Automatic retry-control dispatch/acknowledgement, deleted/expired-control discard policy and completed-control cleanup remain before lost-session recovery is complete. The full backend scope is still required before audit; no UI or audit work was started. Code remains local and uncommitted.

### Mailbox retry-control dispatch and durable acknowledgement

Native schema 27 adds a sealed, sequence-bound retry acknowledgement journal capped at 4,096 entries. `receive_mailbox_online(now)` now distinguishes ordinary encrypted text from signed retry controls and returns an explicit event variant. Controls stage their fresh claim, accepted request and acknowledgement evidence atomically; parser/authentication or journal failures leave no acknowledgement candidate. The existing cyclic scan still moves past individual failures without discarding them.

The existing acknowledgement worker merges text/control candidates into one sequence-ordered batch of at most 16. Control acknowledgements require authenticated durable acceptance, survive server success followed by local write failure, and remain permitted after peer blocking or request expiry without authorizing another resend. Sealed evidence binds the sequence, owning device, request and acknowledgement state. Text/control sequence collisions are rejected. Migration leaves earlier accepted requests unacknowledged until their delivery is accepted again.

Two added HTTPS regressions exercise mixed-event dispatch, atomic journal failure, unchanged claim count after rollback, lost acknowledgement receipt, restart, continued use of the staged claim after acknowledgement, malformed controls, sequence substitution, corrupted journal ciphertext and acknowledgement after blocking. All 282 workspace tests, warnings-denied Clippy and formatting checks pass. Logs are `/tmp/sigil-dispatch-workspace.log` and `/tmp/sigil-dispatch-clippy.log`; device/container runs were not repeated.

Pending-work scheduling through claim/resend completion, rejection/discard policy for deleted or expired requests and completed-control cleanup remain. Full backend completion still precedes audit; no UI or audit work was started. Changes remain local and uncommitted.

### Bounded restart-safe retry work scheduling

Native schema 28 adds a sealed device-bound scan cursor and `resume_retries_online(now)`. Each invocation scans at most 16 accepted controls, resumes fresh prekey claiming when no response is prepared, queues the response, and submits only that response. Existing queued ciphertext skips claim/re-encryption and does not change active-session selection. Existing authenticated delivery receipts provide completion without another status flag or network transmission. The same deterministic session derivation now serves preparation and scheduling.

Per-item local errors permit later work; network errors stop the batch so the caller can honor Retry-After. Cursor persistence compares the previous sealed value, wraps after the end, and does not undo prior durable work if its write fails. Completion remains server acceptance, not peer decryption. Completed records are still scanned pending cleanup. The application must schedule worker invocations and provide the trusted clock.

Two regressions cover an acknowledged request resumed after restart, automatic prekey claiming, server acceptance followed by failed local receipt storage, exact retry, failed cursor persistence after durable receipt storage, completion after blocking/expiry without another session or transmission, an 18-request blocked queue scanned in batches of 16 and 2 across restart, and corrupted cursor rejection. All 284 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-scheduler-workspace.log` and `/tmp/sigil-scheduler-clippy.log`. Device/container runs were not repeated.

Deleted/expired-control discard policy, queued-response cancellation and completed-control cleanup remain before lost-session recovery is complete. Full backend scope remains required before audit. No audit or UI work was started; changes remain local and uncommitted.

### Explicit retry cancellation and active-queue cleanup

Native schema 29 adds an indexed finished state authenticated inside the accepted retry record. Delivery receipt persistence marks retry work completed atomically, so completed controls leave active scheduling without another response. Older accepted receipts are classified when scanned. Signed controls and journal evidence remain available for replay detection, delayed delivery and retry-chain validation.

`cancel_retry_request` atomically abandons the claim, clears only the prepared response packet if present, and seals cancellation. It works before claiming or after queuing, survives restart, and cannot be undone by replaying the signed request. The existing claim-abandon operation shares its transaction helper. Cancellation preserves unrelated packets, ratchet/session state and history. Durable acceptance supersedes cancellation; late authenticated receipts remain recordable and mark completion. Transport expiry handling recognizes cancelled responses without inventing an expiry or claiming non-delivery.

Two added regressions cover cancellation before claiming, prepared/in-flight response cancellation, injected final-write rollback preserving the exact packet and claim, restart, replay without reactivation, abandonment, acknowledgement after cancellation, a late server receipt, scheduler exclusion and a tampered finished index. The scheduler regression now checks that completed work is omitted while its receipt remains accessible. All 286 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-cancel-workspace.log` and `/tmp/sigil-cancel-clippy.log`. Device/container runs were not repeated.

This completes explicit cancellation and removal from active scheduling, not physical ledger reclamation or secure erasure. Existing lifetime caps remain. Automatic deleted/expired-content cancellation and discard policy, plus safe reclamation of replay/chain evidence, remain before lost-session recovery is complete. Full backend scope still precedes audit; no UI or audit work was started. Changes remain local and uncommitted.

### Automatic cancellation of obsolete accepted recovery work

The bounded retry scheduler now cancels already accepted requests whose authenticated deadline has passed or whose authenticated recovery record proves deletion or a changed body. `Error::Obsolete` distinguishes retained-content changes from generic conflicts. The decision and existing claim/packet/finished-state cancellation commit in one transaction. The scheduler rechecks after claim I/O before sending, and durable server acceptance retains precedence. No schema, dependency or wire-format change was needed.

Clock rollback beyond the allowed signed-request window, blocked peers, storage/authentication failures, missing evidence and pending recovery imports remain errors rather than cancellation authority. Failed cancellation writes leave the original claim and queued ciphertext intact. Callers continue to supply the trusted clock; forward clock errors cannot be distinguished universally from actual expiry.

Two added regressions cover expired work before response preparation, deleted/superseded prepared responses, final-write failure and rollback, claim abandonment, queue removal, unchanged session count, acknowledgement of accepted controls and no response transmission. Negative cases preserve work through blocking, corrupted archived ciphertext and clock rollback, then successfully send the exact prepared response when conditions recover. All 288 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-obsolete-workspace.log` and `/tmp/sigil-obsolete-clippy.log`. Device/container runs were not repeated.

Controls initially received after deletion/expiry still need a separately authenticated discard path; they currently fail acceptance without acknowledgement. Safe reclamation of retained retry/acknowledgement evidence remains open, with existing lifetime caps intact. Full backend scope still precedes audit. No UI or audit work was started; changes remain local and uncommitted.

### Authenticated discard of obsolete incoming retry controls

Mailbox dispatch now authenticates and durably resolves controls first seen after signed expiry, deletion or retained-body replacement. Shared evidence validation checks signature, current verified peer, device fingerprints, routing/expiry, original delivery/content and bounded retry chain before allowing discard. Corruption, missing evidence, blocked peers and clock rollback never grant acknowledgement authority. The strict `accept_retry_request` API retains its existing rejection behavior.

New obsolete controls create a cancelled proof record and acknowledgement journal atomically, with no prekey claim or session. Mailbox callers receive `DiscardedRetry`. Revisited accepted work that became obsolete uses the shared cancellation transaction to abandon its claim and clear its queued response, while known server acceptance retains precedence. Existing terminal controls cannot reopen work. The same durable acknowledgement worker handles accepted and discarded controls after restart. No schema or wire-format change was needed.

Three added regressions cover expired/deleted/superseded controls, journal-write rollback, no new claims/sessions, restart and acknowledgement, terminal replay, modified signed fields, blocked peers, clock rollback, corrupt history, and atomic cancellation of a previously queued response on redispatch. All 291 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-discard-workspace.log` and `/tmp/sigil-discard-clippy.log`. Device/container runs were not repeated.

Safe reclamation of retained retry/acknowledgement proofs remains open, with lifetime caps intact. Full backend scope remains required before audit. No UI or audit work was started; changes remain local and uncommitted.

### Bounded reclamation of completed retry acknowledgement journals

Native schema 30 adds a sealed maintenance cursor and `reclaim_retry_journals(now)`. Each call authenticates at most 16 journal entries and reclaims only server-acknowledged, terminal controls whose signed deadline passed. Completed responses additionally require their authenticated matching delivery receipt. Pending, active and still-live records remain. Deletions and cyclic scan progress commit together; restart resumes past ineligible prefixes without advancing any mailbox watermark.

Signed request/chain proofs, delivery receipts, logical-event deduplication and recovery history remain intact. A replay after journal reclamation must authenticate again and can only resolve against retained terminal work; it cannot recreate a claim or session. A fresh acknowledgement journal can then handle repeated server delivery. This frees bounded journal capacity, not guaranteed physical disk erasure.

Three regressions cover acknowledgement/deadline/terminal gating, cursor-write rollback after deletion, restart, replay without new work, a 16-entry pending prefix followed by two reclaimable entries, forged acknowledgement flags, corrupt cursors, preserved active work and delayed response/original deduplication. All 294 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-journal-gc-workspace.log` and `/tmp/sigil-journal-gc-clippy.log`. Device/container runs were not repeated.

Signed retry-request and outgoing-control evidence still needs a safe reclamation strategy; its lifetime caps remain. Full backend scope remains required before audit. No UI or audit work was started; changes remain local and uncommitted.

### Expired outgoing-control reclamation with immutable expiry tombstones

Native schema 31 adds compact outgoing-control tombstones and an inbox-message index. `reclaim_outgoing_retry_controls(now)` authenticates at most 1,024 outgoing records to discover dependencies, then replaces at most 16 expired, unreferenced controls in one transaction. Retained inbox responses and descendant outgoing requests pin their full proofs. Retiring a child can release its parent on a later pass. No accepted-request or message history is deleted.

Authenticated tombstones retain the owning fingerprint and original expiry under the deterministic request ID. They prevent the same failed transport ID from obtaining a renewed request deadline after reclamation. The 1,024-entry full-control capacity is freed; tombstones have a separate 4,096-entry cap, after which retirement stops without deleting evidence. This is compact retention rather than unlimited storage or guaranteed disk erasure. An unseen response after expiry/reclamation no longer has authorization for its differing transport ID and is rejected; already retained responses retain their proof.

Three regressions cover live-control preservation, failed-delete rollback, restart, renewal rejection, corrupted tombstones, original-message acceptance, descendant and received-response dependencies, cached response replay, dependency corruption before any retirement, and a batch of 18 retired as 16 then 2. All 297 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-control-gc-workspace.log` and `/tmp/sigil-control-gc-clippy.log`. Device/container runs were not repeated.

Accepted-request proof reclamation and remaining lifetime retention limits are still open. Full backend scope remains required before audit. No UI or audit work was started; changes remain local and uncommitted.

### Compact reclamation of unreferenced accepted retry proofs

Native schema 32 adds compact accepted-control tombstones. `reclaim_accepted_retry_controls(now)` authenticates at most 4,096 full controls and 4,096 journal references, then retires at most 16 expired cancelled/discarded controls that never prepared a response. Active work, response outbox/delivery records, child requests and remaining journals pin full proofs. Prepared-response records deliberately remain available for history and late receipts. Journal maintenance runs first to release eligible references.

Each tombstone seals owner, peer, original packet digest and signed expiry under the deterministic request ID. Replayed controls must pass normal peer/message evidence authentication and exactly match that digest before receiving a discard journal. Renewal or replacement signatures cannot reopen work. Acknowledgement and journal reclamation recognize compact proofs without recreating full records or claims. The compact ledger is capped at 4,096 and safely stops retirement at capacity. This reclaims eligible full-ledger slots, not unlimited retention or physical disk erasure.

Two added regressions cover journal pinning, failed-delete rollback, restart, exact replay through compact proof, renewed signed expiry rejection, corrupt tombstones, active/live/prepared-response preservation, and corrupt dependency rejection. All 299 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-accepted-gc-workspace.log` and `/tmp/sigil-accepted-gc-clippy.log`. Device/container runs were not repeated.

Remaining lifetime retention limits and end-to-end recovery acceptance under those limits remain open. Full backend scope remains required before audit. No UI or audit work was started; changes remain local and uncommitted.

### Cross-feature recovery and quota acceptance

Added acceptance coverage rather than new protocol behavior. The real HTTPS scenario destroys the recipient's original private prekey, verifies failed decryption without acknowledgement, dispatches and acknowledges a signed retry, encounters fresh-key unavailability without creating a session, restarts both clients, publishes a new key, and lets the scheduler recover the original logical text with one retained history record. A peer reply confirms the fresh session; another restart and ordinary message succeed. Local maintenance checks retain live proof, then reclaim the eligible journal at a synthetic deadline horizon while pinning response-dependent controls.

A second scenario fills all 1,024 full outgoing-control slots through request preparation, checks quota refusal, reclaims 16 expired records and successfully prepares new work. It then fills the 4,096 tombstone ceiling with authenticated synthetic records and checks that retirement stops without discarding full evidence. This verifies bounded behavior and slot recovery; it does not eliminate quota limits or simulate long-term home deployment.

All 301 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-recovery-acceptance-workspace.log` and `/tmp/sigil-recovery-acceptance-clippy.log`. Device/container runs were not repeated. No implementation failure was reproduced by these scenarios.

The test confirms a remaining mailbox behavior: the original undecryptable packet stays unacknowledged until server expiry even after successful logical recovery. Proof-based acknowledgement of such recovered leftovers remains separate work. Full backend scope remains required before audit. No UI or audit work was started; changes remain local and uncommitted.

### Proof-based acknowledgement of recovered mailbox leftovers

Native schema 33 adds a sealed recovered-delivery journal. On failed ordinary mailbox acceptance, `resolve_failed_delivery` follows at most three deterministic retry IDs and uses indexed inbox lookup for a stored replacement. It verifies the outgoing chain, replacement event bindings and an already committed logical-event ledger entry before journaling the original sequence, peer, transport ID, packet digest, expiry and replacement/session. No failed original is decrypted or inserted as another text event by this path.

The mailbox returns `RecoveredDelivery`; the existing acknowledgement worker now shares its 16-item batch across normal text, retry controls and recovered originals. Proof is revalidated before network acknowledgement, and local receipt-write failure safely retries after restart. Cross-journal sequence reuse, packet/expiry substitution, missing committed proof and journal corruption fail closed. Response-dependent outgoing proofs remain pinned by retained inbox data and their chain dependencies.

The real HTTPS lost-prekey acceptance now resolves and acknowledges the original on a subsequent cyclic scan, leaving the mailbox empty. One new regression covers no acknowledgement before committed recovery, failed journal insertion, conflicting packet/expiry, corrupted journal, server success followed by failed local receipt persistence, restart and exact acknowledgement retry. The existing three-hop resend regression also resolves its original. All 302 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-resolved-workspace.log` and `/tmp/sigil-resolved-clippy.log`. Device/container runs were not repeated.

The new journal retains at most 4,096 entries; reclamation and broader lifetime retention remain open. Full backend scope still precedes audit. No UI or audit work was started; changes remain local and uncommitted.

### Bounded reclamation of recovered-delivery acknowledgements

Native schema 34 extends the existing maintenance cursor table to separate retry-control and recovered-delivery scans, preserving the earlier cursor unchanged. `reclaim_recovered_journals(now)` shares the journal-maintenance implementation and reclaims at most 16 expired, authenticated server acknowledgements per call. Pending and live entries remain; replacement messages, request proofs and logical history are untouched. Each cursor has a distinct authentication label, and deletion/progress commit atomically.

Two new regressions cover pending/live preservation, forged acknowledgement flags, failed cursor writes after deletion, restart, re-proving recovery after reclamation, unchanged inbox/session counts, a 16-entry pending prefix followed by two eligible records and cross-cursor substitution. A boundary fixture initially attempted to acknowledge synthetic sequence numbers never issued by the server; it was corrected to seed authenticated local acknowledgement records. The separate HTTPS regression exercises actual server acknowledgement and lost receipt persistence.

All 304 workspace tests, warnings-denied Clippy and formatting checks pass. Logs: `/tmp/sigil-resolved-gc-workspace.log` and `/tmp/sigil-resolved-gc-clippy.log`. Device/container runs were not repeated. This completes the bounded cleanup path for the new recovered-delivery journal; broader lifetime retention limits and full backend scope remain. No UI or audit work was started; changes remain local and uncommitted.

### Account device inventory

`GET /client/v0/devices?after=<device-id>` returns the authenticated account ID and up to 32 devices ordered by opaque device ID. Each entry contains its ID, label, credential expiry and revocation flag. Revoked and expired devices remain inspectable by a currently authorized account device. `next_after` is non-null only when another page exists; concurrent changes are observed on subsequent requests, so this is not a multi-page snapshot. Unknown or malformed query parameters are rejected. Existing native-only origin checks and authenticated read budgets apply.

Native `HttpsClient::devices` bounds response size, validates identifiers, labels, timestamps and pagination ordering. `ClientStore::devices_online` additionally checks the persisted account ID. Inventory never establishes encryption trust, links devices, changes verified peers or selects ratchet sessions. QR linking and authenticated multi-device reconciliation remain unfinished.

Server schema 10 adds an account/device index transactionally; native schema remains 34. Regression coverage exercises account isolation, revocation history, pagination, expired/revoked caller denial, HTTP authentication/origin/query rejection, schema-9 migration preserving sessions, and native HTTPS inventory across restart. No UI or audit work was started.

Validation: 308 tests passed across the workspace components, including the corrected migration fixture and a complete server-suite rerun. Clippy with warnings denied and formatting checks passed. The inspection connection in the migration fixture is reopened after migration to avoid testing stale SQLite schema metadata. Server verification log: `/tmp/sigil-device-inventory-server-final.log`; Clippy: `/tmp/sigil-device-list-clippy.log`. Device/container acceptance was not repeated for this increment. Changes remain local and uncommitted; the full backend is not audit-ready.

The final combined `cargo test --workspace` rerun also passed all 308 tests: `/tmp/sigil-device-inventory-workspace-passed.log`.

### Native device revocation

`HttpsClient::revoke_device` and `ClientStore::revoke_device_online` connect the native client to the existing account-scoped DELETE endpoint. Device IDs are validated before constructing the URL; only an empty HTTP 204 confirms success. Revoking another account's device fails. Revocation can be retried while the caller is still authorized. Self-revocation invalidates that caller's credential, so a subsequent 401 cannot confirm whether an earlier request succeeded. Transport errors and ambiguous responses remain errors; there is no inferred success or automatic trust change.

The adapter retains local encrypted history, credentials and ratchet state. This is server authorization revocation, not local logout/key erasure, authenticated device-set reconciliation or notification to contacts. No schema or dependency change is required. Tests cover account isolation, malformed IDs, revocation visibility, retry across native restart, target credential invalidation, self-revocation and unchanged sealed local connection state. HTTPS response tests reject 200/202 and preserve 401/404 errors.

Validation: all 310 workspace tests, warnings-denied Clippy and formatting passed. Logs: `/tmp/sigil-device-revocation-workspace.log` and `/tmp/sigil-device-revocation-clippy.log`. Device/container runs were not repeated for this native-only adapter change. Full device linking/reconciliation and the broader backend scope remain unfinished; no UI or audit work was started. Changes remain local and uncommitted.

### Account device review before linking/reconciliation

`ClientStore::review_devices_online` combines the account's paginated server inventory with authenticated local peer records. Each `DeviceReview` contains a device ID, current-device marker, optional server metadata and optional local `Peer` status. Unknown server devices have no local peer; locally observed account devices omitted by the server remain visible with no inventory entry. Blocking, verification and previously observed key-change warnings retain their existing meanings. Server revocation metadata does not clear local verification or authorize deleting retained sessions.

The operation reads at most 256 server entries and authenticates at most 4,096 local peers in one local read transaction after network I/O. It rejects account mismatches, a missing current device, excessive inventory, corrupted local records and server inventory conflicting with a locally known device's account. All local peers are authenticated before filtering; an unauthenticated clear lookup cannot hide account devices. Concurrent server changes can make a review inconsistent and require refresh; the server pages are not a snapshot.

This is read-only review, not a trusted device roster: it does not fetch new signed bindings, discover previously unobserved key changes, authorize device linking, grant verification, select sessions or fan out messages. QR/emoji linking and authenticated multi-device reconciliation remain unfinished. No schema or dependency change is required.

Two HTTPS regressions exercise multiple pages, unknown/missing devices, verified and blocked peers, existing key-change quarantine, account isolation, native restart, byte-identical local peer records, corrupted unrelated local records and a server moving a verified device into the wrong account.

Validation: all 312 workspace tests, warnings-denied Clippy and formatting passed. Logs: `/tmp/sigil-device-review-workspace.log` and `/tmp/sigil-device-review-clippy.log`. Device/container acceptance was not repeated for this native-only change. Changes remain local and uncommitted; full backend completion and the subsequent audit remain ahead. No UI or audit work was started.

### Device-link consent foundation

Added a fixed, versioned linking transcript and native independent-device consent primitives. The transcript commits both canonical device-binding fingerprints, separate challenges, a provisioning public key, transport-credential commitment and a bounded expiry. Both device bindings must match account/address context and use distinct device identities. Full-digest confirmation and role-separated XEdDSA signatures bind the exact transcript. Verification additionally requires an independently trusted sponsor fingerprint.

The native joining-binding signer uses the new installation's own stored identity. A configured installation cannot join, including when its connection record is malformed. A sponsor must match its own enrolled binding and locally usable, unexpired connection profile. These operations create no peer trust, server authorization or ratchet transfer. No schema or dependency change is required.

See [DeviceLinking.md](DeviceLinking.md) for the exact encoding, trust boundaries and remaining integration. Durable challenges/consent journals, one-use grants, encrypted provisioning, the actual QR/emoji confirmation protocol, server enrollment and authenticated device reconciliation remain unfinished. Link consent is a primitive, not an enabled or audited linking protocol. Tests cover exact parsing, truncation, lifetime limits, both roles, every transcript byte, swapped signatures, sponsor substitution, account mismatch, persisted independent keys, no trust mutation and rejection of configured/corrupted joining state.

Validation: all 314 workspace tests, warnings-denied Clippy and formatting passed. Logs: `/tmp/sigil-link-final-workspace.log` and `/tmp/sigil-link-clippy.log`. Device/container acceptance was not repeated. Changes remain local and uncommitted; no UI or independent audit work was started.

### Durable device-link bindings and consent

Native schema 35 replaces fresh-signature retries with one sealed, installation-bound journal for prospective joining bindings and role-specific consent. Exact bindings/transcripts/signatures commit before returning. A repeated device ID cannot change sponsor context; a repeated local challenge cannot change transcript or expiry. Cached consent is reauthenticated and its signature checked. Current role/configuration and lifetime checks still apply before returning a retry.

`cancel_device_link_consent` durably and idempotently stops local retries, including across restart; failed cancellation writes roll back. Cancellation cannot retract an already transmitted signature or revoke remote authorization. The shared 256-record bound retains cancelled and expired records to prevent rebinding; new work fails at capacity while retries and cancellation remain usable. Safe lifetime reclamation, generated pending offers/key material, encrypted provisioning, QR/emoji confirmation and server one-use enrollment remain unfinished.

Regression coverage adds schema-34 migration preserving the installation key, failed binding/consent inserts, failed cancellation updates, exact restart bytes, changed-transcript rejection, durable cancellation, record substitution and full-capacity behavior. The old fresh-signature path was replaced rather than retained alongside the journal.

Validation: all 315 workspace tests, warnings-denied Clippy and formatting passed after updating historical migration expectations to schema 35. Logs: `/tmp/sigil-link-journal-verified.log` and `/tmp/sigil-link-journal-clippy.log`. Device/container acceptance was not repeated. Changes remain local and uncommitted; no UI or independent audit work was started.

### Durable pending joining offers

Added generated, sealed joining offers with independent device IDs, challenges, provisioning keys and random transport credentials. Only the exact 184-byte public offer is returned. The shared journal commits private material before exposure, preserves the original offer across restart and rejects expired/cancelled attempt reuse. `approve_device_link_offer` binds approval to the saved public fields and deadline and commits binding/consent atomically. Cancellation removes private material logically and cancels any existing local consent in one transaction while retaining a tombstone.

Two regressions cover offer parsing/truncation, restart equality, expiry/clock rollback, configured-device rejection, transcript substitution, exact consent retries, credential commitments, cancellation rollback after consent mutation, cancellation across restart, fresh-attempt independence, failed creation and corrupted cached records. The existing consent cancellation implementation was factored into the shared transaction helper. No new schema, dependency or handwritten browser code was needed.

Sponsor-side pending state, authenticated encrypted provisioning, QR/emoji confirmation, server one-use enrollment and device reconciliation remain unfinished. No linking capability or endpoint is enabled.

Validation: all 317 workspace tests, warnings-denied Clippy and formatting passed. Logs: `/tmp/sigil-link-offer-workspace.log` and `/tmp/sigil-link-offer-clippy.log`. Device/container acceptance was not repeated. Changes remain local and uncommitted; no UI or independent audit work was started.

### Backend milestone 1 completed: sessions and devices

The complete first milestone is implemented in the server and shared Rust client. [SessionsDevices.md](SessionsDevices.md) records the acceptance criteria, behavior and boundaries. This is **1 of the 12 backend milestones**, not full-backend completion, independent audit approval or production deployment readiness. Production UI remains paused.

Native schema 36 adds sealed, bounded session-maintenance state. The policy protects active/queued sessions, expires prepared delivery work and requires a full observed seven-day inactivity grace before logical retirement. Explicit replacement approval seals permanent old-device supersession with the new fingerprint, removes old selection and grants new trust in one transaction. History survives; old trust and queued sending cannot resume through directory replay or unblock.

Session convergence now uses the shared authenticated initial-transcript hash for established traffic. Fresh initial acceptance can select a replacement session for recovery. This replaces unconstrained last-packet selection, which could oscillate under crossed traffic. Regression tests cover simultaneous opposite initiations and late divergence of already-confirmed sessions, restart, replay and unchanged queued ciphertext.

[DeviceLinking.md](DeviceLinking.md) now describes the complete durable three-scan physical QR exchange and supplementary emoji confirmation. Shared proof verification and encrypted provisioning replace the unfinished channel/authorization boundary of earlier entries. Both devices retain independent identities. Server schema 11 adds one-use proof and cancellation ledgers, live-sponsor authorization, scope/credential/device quotas, exact retry recovery and cancellation that wins a concurrent grant. Native completion atomically adopts the joining connection and verified peers; a failed local commit after server success is recoverable. Cancellation blocks local trust before networking. Server restore and revocation cannot be bypassed with a previously committed proof.

The server now uses sigil-crypto at runtime for proof verification, and the container includes its dependency notices. Tests exchange real encrypted packets through the TLS fixture between linked devices; no production UI or private identity/ratchet transfer was introduced. Physical QR scanning is a stated trust requirement, not an unaudited short-string-only network protocol. General multi-device synchronization, groups, history transfer, long-term retention and forensic key erasure remain their other milestones.

Validation: 325 workspace tests, warnings-denied Clippy and formatting pass. Container build and acceptance cover persistent restart/SIGKILL, exact retries, resource limits, backup locking, restore revocation and retained recovery ciphertext. Logs: `/tmp/sigil-m1-final-tests.log`, `/tmp/sigil-m1-final-clippy.log`, `/tmp/sigil-m1-container-build.log`, `/tmp/sigil-m1-container-acceptance.log`. Android/iOS/macOS acceptance was not repeated for this backend milestone. Changes remain local and uncommitted; no independent audit or UI work was started.

### Correction after in-depth milestone 1 review

The preceding milestone-complete claim is withdrawn. [SessionsDevicesReview.md](SessionsDevicesReview.md) records one reproduced reliability defect: ordinary peer sending can stay on an expired unconfirmed session even after another session receives an authenticated reply and remains usable. An opt-in regression fails with `Expired`; this is a concrete completion blocker, not evidence of a confidentiality break.

The review separately documents retirement requiring recovery for an unread unexpired packet (a tradeoff already permitted by the API), existing lifetime bounds, and missing application lifecycle integration. These are not mislabeled as newly discovered security vulnerabilities. Tested linking authorization, cancellation, replacement and explicit recovery behavior remain useful implemented work. A new passing endorsement test exercises native trust/quarantine/rollback checks.

Milestone 1 is reopened; do not proceed on the assumption that all its acceptance questions are closed. Production code, cryptographic primitives, schemas and dependencies were not changed by this review. The baseline workspace now has 326 passing tests and two opt-in review probes; running the probes separately yields one expected failure and one passing observation of the retirement policy. This review is not the independent audit.

### Reviewed selection defect repaired

An authenticated reply now replaces an unconfirmed active session regardless of transcript-hash ordering. Confirmed sessions retain the shared ordering needed for convergence. For new peer text, an expired unconfirmed selection can fall back to a confirmed live peer session. The bounded scan authenticates candidate state and fails closed on corruption or trust failure. Selection and the new packet commit atomically; existing message retries never move to another session. With no confirmed alternative, the method returns `Expired` without mutating ratchet/message state and leaves fresh-handshake recovery to the caller.

The former ignored failure now runs as an ordinary regression. It checks restart from an old selection, failed promotion, blocked/corrupt alternatives, outbox-failure rollback, frozen queued ciphertext, exact new-message retries and actual recipient decryption. A second test covers absence of a confirmed alternative. Workspace validation: 328 passed, five ignored (four pre-existing helpers/generator and the retirement-policy observation). Added recipient-decryption/retry assertions passed in the updated focused regression. Clippy and formatting pass. Logs are recorded in SessionsDevicesReview.md.

This closes the demonstrated sending defect, not all milestone 1 questions. Retirement availability policy, lifetime limits and application lifecycle integration remain explicit work. No cryptographic primitive, wire format, schema or dependency changed. AGENTS.md now records the user's requirement to prioritize evidenced security/correctness, distinguish bugs from policy/integration questions, and avoid unsupported completion/security claims.

### Retirement now waits for a fresh empty mailbox

Chose a conservative availability policy for the retirement review question. Offline `maintain_sessions` continues bounded outbound expiry and inactivity tracking but does not automatically remove keys. `maintain_sessions_online` adds a bounded, authenticated mailbox query from sequence zero; only a fresh empty response permits eligible retirement during that call. It rechecks the encrypted connection profile inside the maintenance transaction, so a connection change cannot reuse an old check. No reusable drain permit or schema change was introduced; both entry points share one maintenance implementation.

The former opt-in retirement observation is now an ordinary regression using actual server queue submission and HTTPS receive/acknowledgement. It verifies that unread, unexpired traffic remains decryptable across the grace deadline and restart, that an earlier empty check is not reused, and that reception refreshes inactivity state. Other tests cover authorization failure, connection rotation and rollback of retirement on a final cursor-write failure. All 330 workspace tests, Clippy and formatting pass; the four ignored entries are existing process helpers/reference-input generation.

Any queued traffic defers automatic retirement across peers, potentially retaining keys longer. The check cannot rule out a later arrival or dishonest server response; recovery still handles those cases. Explicit retirement remains a deliberate override with delayed-decryption consequences. This resolves the known-backlog policy, not forensic key erasure, lifetime storage limits or application worker orchestration. No wire format, cryptographic primitive, dependency or schema changed. Logs: `/tmp/sigil-retirement-workspace.log`, `/tmp/sigil-retirement-clippy.log`.

### Shared bounded online orchestration

Added `sync_step_online(now)` to the shared Rust client. It sequences existing bounded mailbox reception, durable acknowledgements, accepted recovery work and guarded session maintenance. Results from completed stages remain available when a later stage fails; failures can also follow partial durable effects inside that stage. The report preserves individual receive/recovery errors and identifies a network-stopped recovery item without losing its Retry-After information. No new journal, schema, cryptographic primitive or wire format was introduced.

Two real-HTTPS regressions cover a server acknowledgement followed by failed local commit and restart, and explicit lost-prekey recovery interrupted by unavailable keys, resumed after restart and key publication. Undecryptable reception remains unacknowledged and creates no automatic new recovery request. The existing journals/cursors remain authoritative.

This is partial lifecycle integration: platform invocation/timers/backoff, prekey provisioning, ordinary outbound scheduling and the decision to initiate recovery still need integration. Milestone 1 remains open; lifetime storage limits remain a separate unresolved item. The sandbox initially blocked local fixture socket binding; tests were rerun with authorized loopback access. This environmental failure was not a product defect.

Validation: 332 workspace tests pass; four pre-existing helper/reference-generation entries remain ignored. Warnings-denied Clippy and formatting pass. Logs: `/tmp/sigil-worker-workspace.log` and `/tmp/sigil-worker-clippy.log`. Server/container and mobile acceptance were not repeated for this shared-client-only orchestration change.

### Ordinary outbound scheduling follow-up

Added `resume_outbound_online(now)` and integrated it after authorized recovery in the shared sync step. A pass visits at most 16 peer-bound, nonretired suite-2 sessions, resolving one oldest queued packet per session through the existing transport implementation. Native schema 37 adds a sealed cyclic cursor and a partial pending-session index. The cursor supports zero session IDs, persists across restart, advances past local errors and stops after the first network error. Callers retain the original network error for backoff. Packet journals remain authoritative if receipt or cursor commits fail; no cryptographic primitive or wire format changed.

Three HTTPS regressions demonstrate bounded traversal and delivery beyond corrupt session checkpoints; exact retry after server acceptance followed by a failed local receipt commit, without duplicate delivery or queue reordering; and network failure stopping a batch while restart resumes at the next session. The latter also upgrades schema 36 with queued initial packets intact. A stale schema-version assertion in the prekey migration test failed on the first full run and was corrected to 37.

Validation: 335 workspace tests pass, with the same four pre-existing ignored helper/reference entries. Warnings-denied Clippy and formatting pass. Logs: `/tmp/sigil-outbound-workspace.log` and `/tmp/sigil-outbound-clippy.log`. Mobile and deployment acceptance were not rerun for this shared-client change. Milestone 1 remains open: prekey provisioning, platform timers/backoff and invocation, recovery-trigger decisions and lifetime storage limits remain unresolved. The outbound stage is bounded independently of recovery; an empty pass resets its cursor, and an unprepared oldest packet still requires application intervention.

### Prepared prekey publication scheduling follow-up

Added `resume_prekey_publications_online()` and integrated it after acknowledgements, before recovery, in the shared sync step. Native schema 38 adds a sealed cyclic cursor and a pending-publication index. A pass resumes at most 16 explicitly prepared slots through the existing publication implementation, including older receipts awaiting a local retention deadline. Local errors remain per-slot results; the first network error stops the pass and later sync stages. The cursor supports zero-valued IDs and restart, wraps with an empty pass, and compares the previous sealed state before committing to avoid overwriting concurrent progress.

Publication keeps the original post-acknowledgement wall clock for retention. The scheduler does not substitute the sync start time, generate replacement private keys, or retire material. Three HTTPS regressions cover failed receipt and cursor commits after server acceptance, an intervening remote claim remaining assigned after exact retry, bounded progress beyond corrupt publication records, restart, network failure and schema-37 migration with prepared work intact. Original bundles, server expiry, private slots and remote assignment remain preserved in the interrupted-publication test.

Automatic replenishment is still open: no server inventory endpoint exists, and local private-key counts cannot establish how many published keys remain unclaimed. Slots without publication metadata still require explicit preparation. Platform timers/backoff and invocation, recovery-trigger decisions and lifetime storage limits remain unresolved; milestone 1 remains open. No server API, wire format or cryptographic primitive changed.

Validation: 338 workspace tests pass; four pre-existing helper/reference entries remain ignored. Warnings-denied Clippy and formatting pass. Clippy initially required moving the new test module after implementation items; that ordering was corrected. Logs: `/tmp/sigil-prekey-work-workspace.log` and `/tmp/sigil-prekey-work-clippy.log`. Mobile and deployment acceptance were not rerun for this shared-client change.

### Prekey inventory and replenishment follow-up

Added native-authenticated `GET /client/v0/prekeys`, reporting only the current device's unclaimed, unexpired, uncleared public-bundle count. Authorization and counting share a database transaction. The native HTTPS client validates the response against the existing 64-bundle bound. No schema migration was needed: server schema remains 11 and native schema remains 38.

The shared sync step now calls `replenish_prekey_online()` after resuming prepared publications. The provisional stock target is eight; each invocation can allocate and publish at most one bundle, including optional EC material with a seven-day publication lifetime. Inventory is a supply hint, never erasure authority. Outstanding publications defer allocation; changing local slot count during the inventory request rejects stale refill work. The existing live and lifetime limits remain in force. Local supply errors remain visible without stopping existing queued messages; network errors stop later network stages and retain their original details for backoff.

Refactored private-prekey creation into a shared transactional helper. Explicit publication preparation and replenishment now commit private material and frozen public metadata together, removing the former two-commit preparation implementation. A failed metadata insertion rolls back a newly generated private slot. Existing manually created slots still resume through explicit preparation. Failed HTTP or receipt writes leave the exact prepared publication available to the existing retry worker.

Added tests for device-scoped inventory, claimed/expired/cleared exclusions and revoked authorization; extended HTTP access tests to cover the new route and browser-origin rejection. Three client regressions cover stock targets and replenishment after remote claims/expiry, failed preparation and ambiguous publication across restart, and message delivery continuing when all 64 private slots are occupied. The integrated lost-prekey recovery test now replenishes automatically; it still requires an explicit recovery request and verifies eventual recovered-original acknowledgement.

Validation: 342 workspace tests pass, with four pre-existing helper/reference entries ignored. Warnings-denied Clippy and formatting pass. Logs: `/tmp/sigil-prekey-supply-workspace.log` and `/tmp/sigil-prekey-supply-clippy.log`. Server router tests and real HTTPS client/server tests passed; container and mobile acceptance were not rerun. No cryptographic primitive or encrypted-message format changed. Private-prekey retirement scheduling, platform timers/backoff and invocation, recovery-trigger policy and lifetime capacity remain open. Refill adds no persistent rate limit beyond its per-call allocation bound and existing capacities. Milestone 1 remains open.

### Guarded private-prekey retirement scheduling follow-up

Integrated private-prekey retirement into the existing online maintenance transaction used by the shared sync step. `SessionMaintenance::prekeys_retired` reports at most 16 eligible removals per invocation. The existing authenticated deadline/removal implementation is now a shared transactional helper, replacing the standalone method body rather than duplicating it. The explicit `retire_prekeys(now)` API remains available as caller-directed deadline cleanup.

Automatic cleanup requires the same fresh empty-mailbox response and unchanged connection state as automatic session retirement. It also shares the sealed maintenance clock's rollback check. Unread traffic, including undecryptable traffic, defers automatic key retirement. Offline maintenance never removes private prekeys. Publication retirement markers, private-state removal and maintenance progress commit together; missing authenticated deadlines remain protected. Retained slot IDs remain tombstones. Replenishment runs earlier in the pass, so newly released live-slot capacity becomes available on the next pass.

Two HTTPS regressions demonstrate an overdue private key surviving unread initial delivery until decryption/acknowledgement, and a second-removal storage failure rolling back the whole cleanup transaction and maintenance cursor. Restart resumes batches of 16 and 1, a backward clock is rejected, and retired IDs cannot be recreated. The delayed-message test deliberately uses independent local/server clock origins to exercise the mailbox guard despite a locally elapsed deadline.

Validation: 344 workspace tests pass; the same four pre-existing helper/reference entries remain ignored. Warnings-denied Clippy and formatting pass. Logs: `/tmp/sigil-prekey-retirement-workspace.log` and `/tmp/sigil-prekey-retirement-clippy.log`. No schema, server endpoint, cryptographic primitive or wire-format change was needed. Mobile/container acceptance was not rerun for this shared-client change. Lifetime capacity, platform timers/backoff and invocation, and recovery-trigger decisions remain open; milestone 1 is not complete. Retirement still depends on a trusted local clock and mailbox response, cannot exclude later arrivals, and is not a forensic-erasure guarantee.

### Durable sync scheduling follow-up

Added `sync_due_online()` around the existing bounded sync pass. Native schema 39 adds a sealed device-bound schedule containing the last scheduling time, next eligible time and capped failure streak. Early calls return without HTTP. A one-minute reservation commits before network work, preventing immediate retries after restart or failed completion persistence. Normal completion schedules a five-second interval; stage failures exponentially back off from five to 300 seconds. Parsed numeric Retry-After delays extend this interval, measured from completion. Per-item local errors remain visible without throttling unrelated messaging.

`ScheduledSync` retains the full work report when final scheduling persistence fails and exposes a separate scheduling error. Callers must handle that error before resuming automation: only the reservation may have committed, so an uncommitted server delay is not guaranteed across restart. Late completions can extend but never shorten a newer deadline or clear another worker's failure streak. The reservation is not a lock after expiry; platform adapters still need serialized workers, wakeups and foreground/background lifecycle policy. Lower-level online APIs remain explicit scheduling bypasses. Existing parsing supports numeric retry delays up to one day, not HTTP-date values.

Three HTTPS regressions cover Retry-After measured after a simulated slow pass, schema-38 upgrade, restart and early-call suppression, success resetting the streak, preflight/final scheduling write failures, corruption and backward-clock rejection, exponential capping, and stale completion preserving a newer deadline. The failed final write retains the network work result and initial reservation.

Validation: 347 workspace tests pass, with the same four pre-existing helper/reference entries ignored. Warnings-denied Clippy and formatting pass. Logs: `/tmp/sigil-schedule-workspace.log` and `/tmp/sigil-schedule-clippy.log`. Server schema remains 11; no server endpoint, cryptographic primitive or encrypted-message format changed. Mobile/container acceptance was not rerun for this shared-client change. Platform lifecycle integration, recovery-trigger decisions and lifetime capacity remain open; milestone 1 is not complete.

### Backend item 1: fixed completion boundary

The authoritative remaining checklist is now [SessionsDevices.md — fixed backend completion checklist](SessionsDevices.md#fixed-backend-completion-checklist). It preserves the original sessions/devices scope and does not replace product milestones A–I. There are six implementation acceptance items: R1 ordinary sending without a usable session; R2 recovery initiation/outgoing-control resumption; L1 session/message-record turnover; L2 prekey/claim turnover; L3 recovery-proof turnover; L4 device/link turnover. V1 is one integrated final backend acceptance gate. Each item has source evidence and explicit pass conditions. Work order is R1, R2, L1–L4, then V1.

Platform lifecycle/wakeups, production UI/camera work, unrelated backend items 2–11 and the independent audit are explicitly outside this item's exit condition. Existing shared-backend implementations are retained; useful enhancements alone do not add blockers. New blocking work must be tied to a demonstrated regression or a specific original requirement and explained before changing the fixed checklist. Lifetime-capacity resolution must preserve replay/authorization evidence and retained history, not merely raise constants or discard records.

This was a source/acceptance reconciliation, with documentation changes only. Existing regressions `expired_selection_without_a_confirmed_alternative_preserves_state` and `full_outgoing_quota_recovers_slots_but_tombstone_limit_preserves_evidence` were rerun and passed; they establish current safe refusal/cap behavior, not completion of orchestration or lifetime turnover. The prior full-suite baseline remains 347 passing tests; no new full-suite run or independent audit was performed for this reconciliation.

### Backend item 1: fixed checklist closed

R1/R2/L1–L4 and V1 have passed the acceptance recorded in [SessionsDevices.md](SessionsDevices.md). Ordinary sends now durably prepare/resume a session for a verified peer. Reception exposes explicit recovery actions and the shared worker resumes outgoing controls. Session/device lifetime counters have been replaced by configurable storage budgets and indexed active-work limits while retaining history, consumed-key evidence, receipts, cancellation proofs and superseded trust.

Server schema 12 adds account storage reservations; native schema 42 adds bounded lifecycle indexes and preserves the schema-40/41 send/control journals. Device review now returns bounded pages instead of accumulating lifetime inventory. Crypto primitives and encrypted-message formats are unchanged.

Validation: 355 workspace tests pass; all 126 optimized client tests pass, including four bulk boundary tests skipped in debug builds. Across these runs, 359 distinct tests pass. The four older helper/reference entries remain skipped. Warnings-denied Clippy, formatting, image build and disposable-container restart/backup/restore acceptance pass. Boundary scenarios cover the former 1,024/4,096 session/message/control limits and 256 device/link limits with restart, migration and replay checks. The new send/recovery paths also have commit-failure and expiry regressions.

This closes backend item 1, not the full backend. Items 2–12 remain. UI/platform lifecycle work stays separate, and independent security review follows backend implementation completion.
