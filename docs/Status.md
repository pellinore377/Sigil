# Current work

Backend/shared Rust items **#1–#12 are implemented**. The container now includes a Compose/Wasm setup wizard and Admin dashboard: ownership claim, Argon2id password, verified OIDC linking, account/group deletion, registration and appearance settings. HTTPS discovery supports separate identity and service domains. Advanced service/maintenance controls remain APIs; messaging layouts remain paused. [plan.md](plan.md) is unchanged.

Codex reviewed all eight backend areas in [Audit.md](Audit.md); Claude completed four independent source-review batches. Confirmed restore-journal, quota, maintenance-artifact and federation-counter defects are corrected. That independent audit predates the browser additions; those have received self-review and regression testing.

Claude ran through `claudex-loop` with requested/observed Fable 5.1; usage also reported auxiliary Haiku. Review coverage is bounded, not proof that every line received independent review. No confirmed ratchet encryption defect was found. Unused production handshake interfaces and an unused Braid wire kind were removed; independent legacy fixtures remain test-only. This is not security certification.

## Validation

Server schema **28**, native **68**, attachment cache **5**. Release workspace: **751 passed, zero failures, eleven ignored**. Clippy, formatting and whitespace checks pass. Excluded entries are parent-invoked crash helpers, a fixture generator and separate acceptance/load tests; commands are in the README.

Regressions reproduce the four defects before correction and pass afterward. Tests also cover reciprocal authorization rollback, retries/refunds, migration/restore accounting, live-artifact preservation and bounded deletion retries. Independent HMAC/HKDF expectations verify 36 initial Triple Ratchet packets; this does not prove later-epoch or post-compromise security. Three timing-sensitive debug group scenarios failed; all three pass unchanged in release, including the complete workspace run.

Erasure tests cover superseded checkpoints, cache descriptors, deleted direct/group messages, structured actions, history-sync fragments, legacy migration, interrupted cleanup, restart and replay rejection. Live SQLite files use secure deletion and durable DELETE journaling. Retained receipts and commitments preserve retry/replay protection; [physical and incomplete-transfer limits](Security.md#erasure) remain explicit.

Four AddressSanitizer fuzz targets completed approximately **17.9 million executions** without a product crash. Published-profile rejection tests and independent OpenSSL/libsodium PQXDH fixtures pass. Fuzzing is bounded evidence, not a proof of security.

Container acceptance passes with the actual schema-27 binary upgraded to 28, downgrade rejection, abrupt restart, idempotent retries, offline/guided backup/import/restore and credential revocation. Synthetic OIDC regressions cover signed claims, browser binding, replay rejection, configuration reverification and password-lockout prevention. Earlier separate acceptance covers two-server outages/history sharing/files/calls, the 1 GiB attachment lifecycle, isolated Linux previews and Coturn UDP/TCP/TLS.

Chromium acceptance covers the local wizard, native password inputs, editing accessibility, confirmation navigation, domain discovery, saved settings, login/logout/reload and mobile dark appearance. Static HTML and API routes share the same anti-framing policy.

## Backend measurements

Synthetic HTTPS with simulated 50 ms request RTT; established sessions:

| Workload | Result |
| --- | --- |
| 4 CPUs/8 GiB, 50 accounts, 20 active devices, 200 messages alongside 160 MiB uploads | Durable feedback 2.3 ms p95; recipient decryption 119 ms p95 |
| 100,000 encrypted messages | 64-candidate search page 16.2 ms p95; complete streamed scan 17.1 s |
| Two servers, 20 messages, additional simulated 50 ms inter-server RTT | Recipient decryption 164 ms p95 |

## Remaining acceptance

Formal composition, quantitative RaptorQ healing bounds and target-machine constant-time/erasure guarantees remain unproven. Unrecoverable gaps beyond the skipped-key budget can stall a session; automatic reset is refused. Local mailbox cursors expose aggregate activity, and group credential issuance trusts the user's homeserver. Dependency review retains three advisories and one build-tool maintenance warning with reachability/mitigation notes in [Security.md](Security.md); `cargo audit` is not clean.

Hardware/client acceptance still covers platform key storage/destruction and backup exclusions, scheduling, native OIDC callbacks, recovery screens, live push providers, codecs/audio routing, call setup latency, GPS/maps/alarms, accessibility, battery and UI performance. Real Pocket ID login/profile claims remain untested. TURN TCP/TLS acceptance uses a test bridge. Linux preview isolation does not certify other platforms.
