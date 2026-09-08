# Backend audit procedure

Find exploitable weaknesses and unsupported security claims. Finding counts are not a success metric; passing tests or finding nothing does not establish security. This procedure covers shared Rust and server deployment. Hardware-only claims remain unverified until device acceptance.

Judge Sigil's cryptographic correctness and intended security properties, not byte-for-byte identity or interoperability with Signal. Interpretations and substitutions are legitimate review inputs, not findings by themselves. For each difference, identify the affected assumption/property and assess whether Sigil preserves it. Report a vulnerability only with a demonstrated attack or substantiated security-property violation; an unresolved argument belongs under specification/security questions. Neither “different from Signal” nor “documented in Sigil” settles correctness.

## Baseline and threat model

Freeze a content-hashed snapshot including uncommitted/untracked source, lockfiles, fixtures and build configuration. Record compiler, features, dependency versions and reproduction commands. Give reviewers separate copies; preserve the original baseline for reproducing findings. Do not edit `plan.md` or access `Sigil-dep`.

Map secrets, trust boundaries, entry points and authorization decisions. Distinguish unauthenticated outsiders, malicious authenticated users, removed members/devices, compromised homeservers/federation peers, network attackers and temporary endpoint-state compromise. State which keys, credentials, files and interfaces each attacker controls. Server compromise must not automatically imply endpoint-key possession. Persistent endpoint compromise has different limits from recovery after compromise ends.

## Review assignments

Run at most three worker agents concurrently, in successive waves. Start each with this procedure and the frozen snapshot, without the implementer's conversation or other reviewers' conclusions. The coordinator checks coverage, reproduces findings and investigates interactions between subsystems. After independent passes, exchange findings for challenge and reproduction.

| Assignment | Starting points and required attacks |
| --- | --- |
| Cryptographic correctness | `crypto/src/{handshake,ratchet,spqr,braid,triple}*`, checkpoints and native framing. Map intended security properties and their specification basis to code and evidence: initialization, KDF inputs/domains, identity/transcript binding, authentication before state mutation, counters, epochs, skipped keys, invalid public keys, randomness, key/nonce reuse and downgrade rejection. Assess substitutions by their effect on these properties. |
| Independent cryptographic adversary | Independently derive expected traces for Sigil's declared profile from published constructions and explicitly assessed adaptations, before consulting its expected outputs. Distinguish original-spec reference vectors from Sigil-profile vectors; different bytes alone are not a failure. Exercise reordered/lost/duplicated fragments, reflected/cross-session packets, corruption, exhaustion and compromise/recovery schedules. Check hybrid key combination, forward secrecy and post-compromise recovery assumptions; review constant-time behavior and secret copies. |
| Persistence and recovery | `client/src/{private_db,erasure,retirement,recovery*,conversation_*}`, caches and server storage. Interrupt every security-sensitive commit/handoff; test rollback, concurrent writers, retries, old snapshots, deletion resurrection, archive substitution and unintended live-ratchet restoration. Inspect accessible DB/journal/backup files for retained secrets. |
| Identity and groups | Device linking/replacement, peers/session selection, `group_*`, Sender Keys and private credentials. Try identity substitution, forged approvals, stale authority, membership races, removed-device access, unauthorized history and cross-group key/proof reuse. Check metadata claims separately from content secrecy. |
| Server and federation | Routes, authentication, OIDC/admin, federation, push and maintenance. Test cross-account/object access, replay, signature canonicalization, SSRF/DNS rebinding/redirects, quota bypass, amplification, privilege escalation, malicious backup/import and release verification. |
| Content and integrations | Protocol decoders, SigilText, conversation actions, maps/providers. Test parser ambiguity, forged authors/permissions, conflicting operations, redaction/view-once bypass, private-query disclosure and unbounded allocation/work. Trace input through actual callers. |
| Attachments and calls | Media sandbox, encrypted transfer lifecycle, call signaling/SFrame/forwarding. Test path/symlink escape, hostile archives/codecs/XML, process limits, descriptor substitution, nonce/replay handling, membership rotation, reconnect and unauthorized media access. |
| Dependencies and operations | Cargo/build/FFI boundaries, unsafe code, Docker, configuration, installation/upgrade/restore. Reassess all advisories and mitigation bypasses against shipped feature paths; inspect secret exposure, privileges and resource exhaustion. |

Pin the references listed in [TripleRatchet.md](TripleRatchet.md), verify their hashes and check later errata. Use the published [ratchet](https://signal.org/docs/specifications/doubleratchet/), [PQXDH](https://signal.org/docs/specifications/pqxdh/) and [Braid](https://signal.org/docs/specifications/mlkembraid/) specifications, not Signal implementation code. Evaluate the security reasoning for Sigil's documented Braid/SPQR interpretations, RaptorQ substitution and AEAD/KDF profile. Identify which published arguments still apply and which need additional justification. Do not require replacement solely to match Signal. Unresolved reasoning limits the associated security claim.

## Evidence rules

- Use synthetic local deployments only. No real users, personal files, external services or unrelated hosts as attack targets.
- Read target code freely, but attack only through capabilities granted by the declared threat model. Editing a victim's DB, borrowing its secrets, mocking authorization or calling privileged helpers cannot demonstrate a remote exploit. Label white-box invariant tests separately.
- Keep the target and assertions unchanged during discovery. No disabled protections, weakened expected results, skipped failures or passing clones substituted for the real path. Fault injection must represent a stated failure/compromise model.
- Build independent oracles from specifications and established non-Signal primitives. No copied Signal code, production helpers reused to calculate supposedly independent expectations, retrieved exploit answers presented as discoveries, or agreement between two Sigil endpoints treated as conformance proof. Cite external material and existing advisories.
- Require controls: the legitimate operation works; the attack violates a named property. Preserve/minimize the reproducer. Demonstrate that it fails on the baseline and that the fix blocks it while legitimate behavior still works.
- Findings record baseline hash, location, attacker capabilities, steps/input, observed impact, expected property, severity rationale and uncertainty. Separate confirmed vulnerabilities, specification questions, hardening suggestions and coverage gaps. Do not label something a zero-day merely because an agent proposed it.

## Adversarial review and closure

Give Claude the same procedure and baseline first, withholding our findings until its independent pass ends. Then exchange findings, reproducers and coverage gaps for adversarial review. The user handles the handoff. If `/claudex` becomes available, inspect its instructions and transmitted scope before use.

Reproduce reports before accepting them. Fix root causes in isolated changes, remove superseded code and add meaningful regression tests. A reviewer who did not author each fix checks the original exploit and adjacent paths. Preserve baseline-versus-fixed evidence, including any controlled observation hooks.

Finish with an integrated review of all changes and interactions. Rerun affected exploit/failure tests, specification checks, fuzz targets, workspace checks and deployment acceptance. Close the audit only when assignments have explicit coverage results, confirmed findings are fixed and retested, and unresolved questions/untested boundaries are disclosed. No unexplained test exclusions or blanket claim that encryption is proven secure. Keep current findings/coverage in `Status.md`, durable regressions in tests, and history in Git.
