# Backend source-code security review

Review Sigil's own backend and shared Rust for cryptographic correctness, security defects and unsupported claims. Work consists of source inspection, reasoning from published specifications, synthetic automated tests and corrective patches. It does not include penetration testing, live-system probing, exploit development, external target scanning or attempts to bypass platform safeguards.

Finding counts are not a success metric. Passing tests, reviewer agreement or finding nothing does not establish security. Hardware-only claims remain unverified until device acceptance.

## Correctness standard

Judge intended security properties, not byte-for-byte identity or interoperability with Signal. Interpretations and substitutions are legitimate review inputs, not findings by themselves. For each difference, identify the affected assumption/property and assess whether Sigil preserves it. A substantiated property violation is a defect; an unresolved argument is a security question. Neither a specification difference nor its documentation settles correctness.

Use the published references pinned in [TripleRatchet.md](TripleRatchet.md); verify hashes and check errata. Do not copy or import Signal implementation code. Assess Braid/SPQR interpretations, RaptorQ and the AEAD/KDF profile by their security reasoning. Identify which published arguments apply and where justification remains incomplete.

## Baseline and reviewers

Freeze the exact source, lockfiles, fixtures and build configuration, including outstanding changes. Record the commit/content hashes, compiler, features and proof commands. Preserve the baseline for comparison. Do not edit `plan.md` or access `Sigil-dep`.

Use at most three worker agents concurrently in successive waves. Give each an isolated copy and this procedure without other reviewers' conclusions. The coordinator verifies coverage, checks evidence and reviews interactions between subsystems. Map secrets, trust boundaries and authorization assumptions from code; distinguish server-held data from endpoint-held keys.

| Assignment | Required source review |
| --- | --- |
| Cryptographic correctness | Handshake, ratchet, SPQR, Braid, Triple Ratchet, checkpoints and native framing: initialization, KDF domains/inputs, identity binding, authentication before state changes, counters, skipped keys, randomness and key/nonce lifecycle. |
| Independent cryptographic reasoning | Independently derive expected properties and test vectors for Sigil's declared profile. Review hybrid key combination, forward secrecy, recovery after temporary state exposure, constant-time behavior and secret copies. Distinguish mathematical reasoning from properties demonstrated by tests. |
| Persistence and recovery | Transactions, crash consistency, concurrent writers, retries, retirement, deletion, journals/caches, archive authenticity and exclusion of live ratchet state from history recovery. |
| Identity and groups | Device linking/replacement, session selection, membership/admin checks, Sender Keys, private credentials, key rotation, revocation and authorization for earlier history. |
| Server and federation | Route authorization, account/object ownership, OIDC/admin, signatures, federation, push, egress restrictions, quotas, maintenance and release verification. |
| Content and integrations | Protocol decoding, SigilText permissions, conversation conflicts, redaction/view-once behavior, provider disclosure and bounds on processing/allocation. |
| Attachments and calls | Transfer encryption and lifecycle, filesystem/process isolation, parser limits, call participant authorization, media keys, replay handling, membership changes and reconnects. |
| Dependencies and operations | Shipped dependency paths/advisories, unsafe code/FFI, Docker privileges, configuration, installation, migration and restore correctness. |

## Evidence and tests

- Use only synthetic data and disposable local test fixtures. Never access personal credentials or test third-party systems.
- Trace suspected defects through actual callers and state transitions. Record the baseline, source location, preconditions, affected property, evidence and practical consequence. Keep confirmed defects, reasoning gaps and optional improvements separate.
- Validate code-level defects with minimal unit, property or integration tests where feasible. Cover both expected legitimate behavior and the failure condition. Use existing test interfaces for commit failures, malformed inputs, ordering and restart behavior; do not construct operational exploit tooling.
- Derive independent expectations from published constructions and assessed adaptations. Do not use production helpers to compute their own expected answers or present two Sigil endpoints agreeing as independent verification.
- Preserve original assertions and protections. Do not alter implementation to manufacture a finding, hide failing checks, relax expected behavior to pass, or claim a test demonstrates more than it observes. Label internal instrumentation and its limits.

## Second review, fixes and closure

Use the installed `claudex-loop` runtime for Claude's independent source review of the same baseline and procedure, before sharing our findings. Preserve the requested Claude model and report the actual model when available. Claude's source-reading review cannot itself establish that tests pass; the coordinator runs them and validates reported evidence. Keep detailed runtime artifacts outside the repository and leave `plan.md` untouched.

Then exchange findings for critical review. Fix confirmed root causes, remove superseded code and add regression tests. A reviewer who did not author each fix checks it and related code. Show the regression failing against the baseline and passing after correction while legitimate behavior remains intact.

Finish with an integrated source review and the relevant automated correctness, failure, workspace and installation/upgrade/restore checks. Close only with explicit coverage, confirmed defects corrected and verified, and unresolved reasoning or platform limits disclosed. Keep the current summary in `Status.md`, lasting regressions in tests and history in Git. Never claim this review proves encryption secure.
