# Milestone 1: sessions and devices

**Implementation acceptance complete, 2026-09-06.** The fixed six items R1/R2/L1–L4 and final gate V1 are implemented and verified for server services and shared Rust client logic. This is item 1 of the twelve-part backend breakdown, not completion of the full backend or the product milestones in [plan.md](plan.md). The independent security audit remains separate.

## Fixed backend completion checklist

- [x] **R1 — Ordinary sends without a usable session.** A durable shared entry point freezes text and its independently verified recipient before claiming a prekey. First sends and expired-initial/no-alternative sends establish a fresh session. Interrupted claims, preparation and submission resume exactly; missing stock stays queued for backoff. Existing ciphertext never moves to another ratchet. Blocked, unverified and superseded peers receive no implicit approval.
- [x] **R2 — Recovery decisions and outgoing-control resumption.** Normal reception returns a defined recovery action for eligible missing-prekey/session failures. Explicit approval durably prepares one exact control; the shared worker resumes submission. Invalid traffic, trust failures, local-state failures, capacity exhaustion and expiry do not imply permission to recover. Restart, submission failure, cancellation, three-attempt chains, duplicate suppression and acknowledgement of the original delivery are covered.
- [x] **L1 — Session/message turnover.** Retained sessions, incoming/outgoing history, logical events and mailbox receipts no longer consume fixed lifetime event counters. Storage budgets and live-queue limits remain. Boundary acceptance crosses 1,024 retained sessions and 4,096 messages, then continues through restart and HTTPS delivery with readable history and duplicate suppression.
- [x] **L2 — Prekey/claim turnover.** Client private-slot tombstones, claims and server public-key records cross 4,096 retained entries. Live private keys, available server bundles and pending claims retain their active limits. Exact interrupted claims and delayed valid initials remain supported; consumed/abandoned identities cannot become fresh keys. Expiry releases bundle storage transactionally without deleting assignment evidence.
- [x] **L3 — Recovery-proof turnover.** Completed/expired controls and retained acknowledgement/proof records no longer exhaust active-work allowances. Cleanup is bounded, restartable and preserves parent/child, response and acknowledgement dependencies. Acceptance crosses 1,024 outgoing controls and the 4,096-record journal/tombstone boundaries while retaining recovery authorization and logical deduplication.
- [x] **L4 — Device/link turnover.** Revoked/expired server devices and superseded local peers do not occupy active-device/contact allowances. Link records and cancelled challenges use storage budgets instead of 256 lifetime slots. Tests cross the historical limits, perform fresh linking/replacement, preserve history and reject old credentials, grants and trust revival. Device review is paginated.
- [x] **V1 — Integrated backend acceptance.** Workspace and optimized client suites, failure/migration regressions, warnings-denied Clippy, formatting, image build and disposable-container acceptance pass. The scenarios exercise two synthetic clients and an actual local HTTPS server; bulk messaging also exercises server authorization/storage directly to avoid thousands of TLS handshakes. No new cryptography or encrypted-message format was introduced.

The starting defects and limitations remain recorded in [SessionsDevicesReview.md](SessionsDevicesReview.md) and the historical entries in [plan.md](plan.md). They are not additional open milestone requirements.

## Shared send and recovery contract

`queue_peer_text(peer, id, body, timestamp, now)` seals the canonical text, original timestamp and recipient fingerprint before network work. Repeating an ID requires the same text and peer. `resume_send_intents_online(now)` handles at most 16 pending intents, with a sealed cyclic cursor. It first reconciles any committed delivery/content; otherwise it uses the selected session or durably prepares a claim and initial session. A seven-day ambiguous attempt or expired claim advances its claim generation without changing logical content. Prepared packets retain their original session, routing, expiry and receipts.

An initial packet uses a different commitment context from an ordinary message. Reconciliation therefore authenticates the retained content and original delivery journal instead of trying to reconstruct an ordinary-message commitment. Tests inject failures after remote claim acceptance and after local packet commit, reopen the database and verify one exact delivered message.

`IncomingAttempt::recovery` is `None`, `Offer(RecoveryAction)` or `Refused(RecoveryBlock)`. An offer exposes the delivery sequence and verified peer fingerprint. Missing/consumed prekeys and an ordinary packet with no remaining live bound session can produce an offer. Authentication failure with existing sessions does not establish missing-state evidence. Malformed packet encodings remain invalid traffic even when the crypto parser reports its size-limit error.

`approve_recovery(action, now)` requires an explicit caller decision. It checks time before any decryption or state mutation, rechecks current trust and whether recovery is still needed, and commits the exact signed request. It does not automatically approve repeated failures. `resume_retry_controls_online(now)` resumes at most 16 prepared controls with a durable cursor and cached receipts. Lost local receipt commits retry the same remote message. Existing response cancellation, immutable expiry, retained authorization and three-attempt recovery chains remain enforced.

`sync_step_online(now)` performs bounded mailbox reception, durable acknowledgement, prepared prekey publication, prekey replenishment, outgoing recovery controls, accepted recovery responses, queued text preparation, pending packet submission and guarded session/prekey maintenance. It preserves individual results and identifies stage/network failures. Local per-item errors do not silently acknowledge failed traffic. Network errors stop later network work.

`sync_due_online()` retains the existing sealed scheduling contract: a one-minute pre-I/O reservation, five-second normal interval, exponential failure delays up to 300 seconds and parsed numeric Retry-After extensions measured from completion. Late completion cannot shorten a newer deadline. The returned result exposes scheduling-persistence errors. Platforms own serialized workers, clocks, wakeups and foreground/background policy; there is no hidden timer or platform lifecycle implementation.

## Storage instead of lifetime event counters

Native schema **42** and server schema **12** migrate supported predecessors transactionally. Native migration authenticates retained peer/control/journal state when building the new indexes. Server migration reconstructs account reservations from retained records. No migration discards history, ratchets, credentials or replay proofs to make space.

| Record family | Retention and remaining bounds |
| --- | --- |
| Client sessions, inbox/outbox, incoming and text-event journals | Retained within the database page budget. Eight live sessions per peer and 256 pending packets per session remain; 256 pending text intents are allowed. Retired sessions keep authenticated markers and readable history. |
| Client private prekeys and claims | Consumed/retired slots and abandoned claims remain evidence. At most 64 live private slots and 64 pending claims. Indexed allocation checks avoid scanning all retained keys. Replenishment targets eight available bundles, at most one new publication per call. |
| Server prekeys/mailbox | At most 64 available bundles per device, 256 pending messages per recipient and 64 per sender/recipient pair. Claimed keys, message hashes, identities and exact receipts remain retained. Expiry clears at most 64 payloads/bundles per table per batch. |
| Recovery controls/proofs | Active unexpired controls are bounded at 1,024 outgoing and 4,096 accepted unfinished requests. Completed or expired evidence uses storage. Cleanup examines at most 16 records per pass, with sealed cyclic cursors and indexed dependencies. Pins preserve response authorization and parent/child chains; compact tombstones retain expiry/cancellation evidence. |
| Devices, links and peers | Server linking allows 256 active, unexpired devices per account. Reauthorization revokes prior credentials atomically. Historical link/cancellation/device records remain retained. The client allows 4,096 nonsuperseded peer records; obsolete peer records do not occupy those slots. Individual link records remain bounded to 32,768 sealed bytes. |

`ClientStore::open` applies a **1 GiB main-database page budget**. `open_with_storage_limit` accepts 1 MiB–1 TiB, rounded down to whole database pages. Callers persist and consistently supply their chosen budget on every open. A database larger than the requested budget is refused, not truncated. Page exhaustion rolls back writes; increasing the budget permits the same database/identity/ratchet to continue. WAL, checkpoint and other filesystem overhead require additional space. This is a storage bound, not universal disk accounting or forensic erasure.

Server `default_quota_bytes` covers recovery ciphertext, live recipient mailbox payloads and retained account storage reservations. Reservations charge the sender 512 bytes per retained mailbox record; public prekeys reserve 512 bytes plus stored bundle length; devices reserve 2,048 bytes; linking proofs reserve 512 bytes plus proof length; cancelled challenges reserve 256 bytes. Fixed reservations include row/index overhead. They are service accounting units, not an exact measurement of SQLite/WAL file sizes. Bundle cleanup refunds only the bytes removed in the same transaction. Restore rebuilds reservations after payload/key clearing. Duplicate operations do not reserve twice.

Exhaustion means an explicit storage refusal, not permission to evict history or replay evidence. Operators can raise the server quota and callers can raise the native budget without resetting an account. Existing over-budget records survive migration and configuration changes. This policy deliberately retains evidence; unlimited history in finite storage is not promised.

`review_devices_online(cursor)` returns a bounded `DeviceReviewPage`: one server inventory page, followed by pages of at most 16 authenticated local peers. Follow `next` through both passes and merge populated fields by device. Local observations include devices omitted by the server. A missing field in one page is not a revocation/deletion decision. Cursor use is bound to the same connection session, and server observations cannot override verified local account bindings.

## Original five requirements

| Requirement | Preserved behavior and evidence |
| --- | --- |
| Automatic selection/convergence | Authenticated replies supersede unconfirmed initials; confirmed sessions use shared transcript ordering. Crossed initiations converge. Expired selection repairs use confirmed alternatives or the new durable fresh-handshake path. Exact retries never change ratchets. |
| Lost-handshake/session recovery | Repeated initial headers until confirmation; frozen deadlines; signed, peer-bound recovery controls; fresh-prekey response chains; explicit shared recovery actions and normal worker completion. Tests cover missing prekeys, retired sessions, restart, damaged traffic, trust changes and recovered-original acknowledgement. |
| Retirement | Seven-day observed inactivity grace. Offline maintenance never automatically erases session keys. Online retirement requires a fresh mailbox check from sequence zero; any backlog defers automatic retirement. Active selections and pending packets remain protected. Private prekey retirement uses the same guard and authenticated deadlines. |
| Authenticated QR/emoji linking | Durable three-scan physical exchange, independent installation identities, full transcript confirmations and role signatures, encrypted provisioning, one-use server enrollment and exact restart recovery. Emoji comparison supplements the direct scans. See [DeviceLinking.md](DeviceLinking.md). |
| Replacement approval | Explicit comparison of old/new full fingerprints; same-account, distinct-device replacement; atomic supersession and selection removal; no restoration of old trust through unblock, directory replay or reused server credentials. Retained history remains readable. |

Mailbox honesty and a trusted local clock remain assumptions of automatic retirement. Later arrivals can require recovery. Explicit retirement is a deliberate override. SQLite logical deletion is not forensic key erasure, and restoring a live client database remains unsupported.

## Acceptance evidence

```sh
cargo test --locked --workspace
cargo test --locked --release -p sigil-client --lib
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
docker build -t sigil-backend:dev .
bash server/tests/container.sh
```

The workspace run reports **355 passed, eight skipped**. Four skipped entries are pre-existing helper/reference generators; the other four are bulk cryptographic lifetime checks enabled in release builds. The optimized client run reports **126 passed, zero skipped**, including all four. Across both runs, **359 distinct tests pass**. All commands above pass; test counts alone are not the completion criteria.

Boundary evidence lives in [message_lifetime_tests.rs](../client/src/message_lifetime_tests.rs), [claim_tests.rs](../client/src/claim_tests.rs), [retry_acceptance_tests.rs](../client/src/retry_acceptance_tests.rs), [peer_tests.rs](../client/src/peer_tests.rs), and the server mailbox/device/enrollment/maintenance tests. [send_intent_tests.rs](../client/src/send_intent_tests.rs), [recovery_action_tests.rs](../client/src/recovery_action_tests.rs) and worker tests exercise the new orchestration and commit boundaries. Existing selection, delayed-delivery, cancellation, linking and replacement regressions remain enabled.

The container check passes non-root/read-only execution, resource restrictions, persistent restart, exact retries, backup locking, restore credential revocation and retained recovery ciphertext. Schema migration tests cover prior database versions, including server 11→12 and native 41→42 with retained work. There is no claim of a released-image upgrade test or deployment to the user's home server. Temporary run logs are `/tmp/sigil-six-accepted-workspace.log` and `/tmp/sigil-six-accepted-release.log`.

## Outside this milestone

Production UI/camera integration, platform lifecycle/wakeups and Apple acceptance remain separate. Backend items 2–12 remain unfinished, including the planned security/operational implementation work. Independent security/cryptographic review follows backend implementation completion. No audit or Signal interoperability claim follows from these tests. The finish line for item 1 is closed; unrelated enhancements do not reopen it without a reproduced regression or an unmet original requirement.
