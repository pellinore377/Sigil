# Sessions and devices: implementation review

Historical review, 2026-09-06, against milestone 1 and the accounts/devices requirements in `plan.md`. The earlier unsupported completion claim was withdrawn. The reproduced selection defect, retirement policy and subsequent fixed R1/R2/L1–L4 implementation work have now passed [the final completion checklist](SessionsDevices.md#fixed-backend-completion-checklist), including V1 acceptance. Platform integration remains outside this backend item. The findings and completion decision below describe the earlier review, not the current milestone status or additional completion gates.

## Follow-up policy: check the mailbox before automatic retirement

Offline maintenance now retains keys while tracking inactivity and expiring outbound work. `maintain_sessions_online` checks the mailbox from sequence zero, then permits eligible retirement only if that fresh response is empty and the connection profile still matches inside the transaction. It never persists or reuses the check. A nonempty mailbox, failed request or changed connection cannot authorize automatic retirement.

The former retirement probe is now `retirement_waits_for_unread_unexpired_packets_to_be_processed`, enabled in the ordinary suite. It places a valid packet in the server queue near the grace deadline, verifies maintenance preserves it across restart, then decrypts and acknowledges it over the existing HTTPS path. An earlier empty check does not authorize later retirement. Reception refreshes activity. Tests also cover failed authorization, connection rotation and transaction rollback.

This is a conservative policy choice, not a claim that the original probe proved permanent loss. Any queued traffic now defers automatic retirement across peers, so keys may remain longer. Later arrivals and dishonest server responses can still require recovery; explicit retirement remains an intentional override. See SessionsDevices.md for the current contract.

Validation: 330 workspace tests pass, with four pre-existing ignored helpers/reference-input generation. Both review regressions now run normally. Clippy and formatting pass. Logs: `/tmp/sigil-retirement-workspace.log`, `/tmp/sigil-retirement-clippy.log`. No schema, wire format, dependency or cryptographic primitive changed.

## Follow-up repair: selection defect fixed

An authenticated reply now supersedes an unconfirmed current session. Confirmed sessions retain shared transcript-hash ordering. New peer sends also repair an expired initial selection by choosing a confirmed alternative, with bounded candidate authentication and atomic selection/outbox persistence. Exact queued retries remain on their original sessions. Corruption and failed trust checks cannot trigger a permissive fallback; without an alternative, sending returns `Expired` without changing ratchet/message state.

The former failing probe is now an ordinary passing regression, extended to cover restart with old selection, failed promotion, corrupted alternative state, blocking, outbox-write rollback, unchanged queued ciphertext, exact fallback retries and recipient decryption. A separate regression verifies the no-alternative case.

```sh
cargo test -p sigil-client --lib unconfirmed_selection_yields
cargo test -p sigil-client --lib expired_selection_without
```

Validation: 328 workspace tests passed with five ignored entries (four pre-existing helpers/generator plus the retirement-policy probe). The recipient-decryption/exact-retry assertions were then added and the updated focused regression passed. Warnings-denied Clippy and formatting pass. Logs: `/tmp/sigil-selection-fixed-workspace.log`, `/tmp/sigil-selection-fixed-regression.log`, `/tmp/sigil-selection-fixed-clippy.log`. No schema, wire format, dependency or cryptographic primitive changed. The evidence-first requirement is recorded in `AGENTS.md` for future objectives.

The sections below preserve the original review evidence; the sending defect and known-backlog retirement policy have follow-up implementations above.

This is a source and regression-test review by the implementing assistant, not the planned independent security audit. No production behavior, cryptographic primitive, schema or dependency was changed during this review. Changes are review tests and corrections to completion documentation.

## 1. Original confirmed defect, now resolved: expired initial selection

Priority: high for milestone completion; a reliability defect, not a demonstrated confidentiality break.

`client/src/selection.rs:85` chooses between established incoming traffic and the current session solely by their initial-transcript hashes. It does not distinguish a current session that has never received a peer packet. `client/src/event.rs:190` then uses that selection for ordinary peer sending. `client/src/handshake.rs:360` correctly rejects a new send once an unconfirmed initial's frozen deadline expires, but no selection fallback follows.

The regression creates two genuine peer-bound handshakes. It makes the lower-hash session active and leaves that initiation undelivered; the peer receives the other initiation and sends an authenticated reply. Local selection still points to the unconfirmed session. After its deadline, an explicit send on the confirmed alternative succeeds while ordinary `send_peer_text` returns `Expired`.

The test explicitly selects the lower-hash session through the same transaction helper used by session creation. This represents that initiation being newest without relying on randomly generated hashes arriving in a particular order; it does not modify keys or forge a checkpoint. It uses a synthetic future clock, not a week-long wall-clock test.

Original reproduction before repair (the test was subsequently renamed and enabled above):

```sh
cargo test -p sigil-client --lib review_unconfirmed_selection -- --ignored
```

Result before repair: failed with `ordinary send must recover a usable selection; got Err(Expired)`. It was ignored in the original ordinary suite; after repair it runs normally and passes.

Repair requirement: define usable-session selection and expired/unconfirmed-session recovery together. Preserve exact queued ciphertext, trust checks and transactional state; test interrupted fallback, absent usable alternatives, simultaneous initiation and crossed established traffic. The implemented repair is above. Creating fresh sessions when no alternative exists, including handling prekey shortage, remains the caller's recovery workflow.

## 2. Original retirement observation; backlog policy now implemented above

`client/src/retirement.rs:132` protects local active selection and outbound pending packets. Retirement after observed inactivity does not depend on draining the inbound mailbox. A peer can send a still-valid packet on an older session near the end of the grace interval; maintenance can remove that session before the recipient processes the packet.

A second probe establishes a confirmed session, makes it inactive, sends an authenticated packet on it just before the grace deadline, then runs maintenance. The packet is unexpired but no longer decrypts afterward. It invokes the supported explicit-session send API to represent the remote peer continuing to use that session; it does not prove the ordinary selection path will always produce this schedule. The probe documents current behavior and passes by observing the failed decryption.

```sh
cargo test -p sigil-client --lib review_retirement_can_require -- --ignored
```

This is **not** a newly discovered contradiction of the current retirement API: that API already says later unseen packets may require session recovery. Nor does the probe demonstrate permanent message loss or broken cryptography. The open decision is whether that availability tradeoff is acceptable, or retirement should require a successful mailbox-processing boundary and an explicitly defined latency policy.

Signal's Sesame specification distinguishes stale-record deletion after a latency interval and mailbox processing, and separately describes session expiration. It is a useful review reference; Sigil's custom hash ordering is not established as correct merely by citing Sesame. [Sesame §§3.1 and 4.2](https://signal.org/docs/specifications/sesame/).

## 3. Documented limits that need lifetime policy

These are existing, intentional resource bounds, not vulnerabilities discovered by forcing tests to fail:

- `client/src/lib.rs:540` caps all session rows at 1,024. Logical retirement retains those rows, so freeing a per-peer slot does not free the installation-wide lifetime capacity.
- `client/src/peers.rs:455` limits a peer to eight live sessions. A ninth handshake fails until an eligible session is explicitly or automatically retired.
- `client/src/link_journal.rs:54` caps the shared link journal at 256 records, including cancelled attempts and consent records. One linking attempt can consume several records. There is no safe journal reclamation path yet.
- Server linking also retains device and cancellation ledgers with stated limits.

These must be surfaced honestly and assigned a safe lifetime/reclamation policy before general deployment. Deleting anti-replay tombstones or silently resetting the client database is not an acceptable workaround. Some retention work belongs to milestones 5/12; the milestone tracker must state that dependency rather than imply unlimited device/session lifecycle support.

## 4. Integration boundaries, not automatically implementation bugs

Source searches find no application caller of `maintain_sessions`. `receive_mailbox_online` reports undecryptable deliveries but does not itself create their retry requests; `prepare_retry_request` is explicitly caller-triggered. Fresh-prekey availability, worker scheduling, error handling and trusted clock supply remain caller responsibilities.

Those are legitimate library boundaries, especially while UI work is paused. They nevertheless mean that library tests do not establish an automatically recovering application. A shared Rust lifecycle driver can be tested without building production layout; its ownership and acceptance should be explicit.

Likewise, the implemented linking flow exchanges three typed URI payloads through direct physical scans. The tests pass strings between Rust stores; actual QR rendering/scanning and human confirmation are not tested. This is compatible with deferring UI, but it is not a tested single-scan or emoji-only network-linking experience.

`revoke_device_online` intentionally revokes server authorization while retaining local encryption trust/history. `approve_peer_replacement` and sponsored-link cancellation have different local trust effects. The application must call the correct operation; server revocation must not be described as erasing local ratchet keys or propagating a cryptographically authenticated roster update to every contact.

## 5. Checks that held up

The exercised paths did not produce a new failing case for:

- Server linking proof signatures/account scope, live sponsor authorization, one-use challenges, transactional grant creation, exact retries, cancellation races, revocation and server restore.
- Independent installation identities, persisted exact encrypted linking frames, confirmation mismatch/substitution rejection and recovery from server success followed by a failed local commit.
- Replacement approval preserving history while atomically removing old selection, granting the approved replacement and preventing ordinary old-trust revival.
- Existing lost-prekey recovery tests, which explicitly initiate recovery, replenish keys and drive the durable workers through restart and ambiguous network results.

A previously uncovered native endorsement entry point now has a focused test: `reviewed_link_endorsement_requires_existing_trust_and_preserves_quarantine`. It verifies rejection of an unverified sponsor, expired endorsement, blocked child/sponsor and changed sponsor binding; a failed peer insert leaves no partial trust, and valid endorsement succeeds. This test passes. It does not assert current server authorization or distributed revocation, which that local proof API does not check.

No conclusion here establishes cryptographic security or substitutes for the independent audit. Equally, nothing in the evidence justifies describing the functioning linking/replacement work as imaginary or wholly broken.

## Completion decision

Milestone 1 remains open. The reproduced sending failure is fixed and automatic retirement now checks for a known mailbox backlog. Resolve lifetime limits and distinguish library completion from application lifecycle integration before making another unconditional completion claim. The existing working mechanisms remain in place.

The earlier mistake was my completion claim and insufficient adversarial acceptance coverage. The user's enthusiasm is not an excuse for that claim. Future status reports must disclose known failing probes separately from passing baseline tests.

Evidence logs: `/tmp/sigil-m1-review-repros.log`, `/tmp/sigil-m1-review-endorsement.log`, `/tmp/sigil-m1-review-workspace.log`, `/tmp/sigil-m1-review-clippy.log`. The two review probes are opt-in; ordinary workspace test results do not run them. Container/mobile acceptance was not repeated because production implementation was unchanged.

Original review verification: workspace tests report 326 passed and six ignored entries. Four ignored entries predate this review (three child-process helpers and one reference-input generator); the other two are the review probes above. Running the two probes explicitly reports one failure (the selection defect) and one pass (the observed retirement behavior). Warnings-denied Clippy and formatting pass.
