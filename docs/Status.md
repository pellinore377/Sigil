# Current work

Backend/shared Rust items **#1–#12 are implemented and self-reviewed**. Production UI remains paused. Next: audit the complete backend, resolve findings, then perform home-server deployment acceptance. Scope remains [plan.md](plan.md); do not edit it. No independent cryptographic audit or production-security certification is claimed.

## Validation

Server schema **26**, native **68**, attachment cache **5**. Final release workspace: **730 passed, zero failures, eleven ignored**. Clippy, formatting and whitespace checks pass. The ignored entries are parent-invoked crash helpers, a fixture generator and separately exercised acceptance/load tests; commands are in the README.

Erasure tests cover superseded checkpoints, cache descriptors, deleted direct/group messages, structured actions, history-sync fragments, legacy migration, interrupted cleanup, restart and replay rejection. Live SQLite files use secure deletion and durable DELETE journaling. Retained receipts and commitments preserve retry/replay protection; [physical and incomplete-transfer limits](Security.md#erasure) remain explicit.

Four AddressSanitizer fuzz targets completed approximately **17.9 million executions** without a product crash. Published-profile rejection tests and independent OpenSSL/libsodium PQXDH fixtures pass. Fuzzing is bounded evidence, not a proof of security.

Container acceptance passes with the actual schema-25 binary upgraded to 26, downgrade rejection, abrupt restart, idempotent retries, offline/guided backup/import/restore and credential revocation. Separate tests pass for two-server outages/history sharing/files/calls, the 1 GiB attachment lifecycle, isolated Linux previews and Coturn UDP/TCP/TLS.

## Backend measurements

Synthetic HTTPS with simulated 50 ms request RTT; established sessions:

| Workload | Result |
| --- | --- |
| 4 CPUs/8 GiB, 50 accounts, 20 active devices, 200 messages alongside 160 MiB uploads | Durable feedback 2.3 ms p95; recipient decryption 119 ms p95 |
| 100,000 encrypted messages | 64-candidate search page 16.2 ms p95; complete streamed scan 17.1 s |
| Two servers, 20 messages, additional simulated 50 ms inter-server RTT | Recipient decryption 164 ms p95 |

## Remaining acceptance

The complete-backend audit is next. Dependency review retains three advisories and one build-tool maintenance warning with reachability/mitigation notes in [Security.md](Security.md); `cargo audit` is not clean.

Hardware/client acceptance still covers platform key storage/destruction and backup exclusions, scheduling, OIDC callbacks, recovery/Admin screens, live push providers, codecs/audio routing, call setup latency, GPS/maps/alarms, accessibility, battery and UI performance. TURN TCP/TLS acceptance uses a test bridge. Linux preview isolation does not certify other platforms. Production layouts remain paused until requested.
