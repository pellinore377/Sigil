# Group protocol contract

Group state, transport account authorization and independently verified device identity are separate. This experimental Sigil profile uses published designs, not Signal implementation code. Remaining implementation work is in [Status.md](Status.md).

## Membership and ordering

Genesis pins group identity and authority. Signed transitions bind predecessor, revision, exact member/device/role set and author. Authorization is checked against the predecessor. Joining devices consent; adding a device also requires an existing member-device approval. Members can leave/change their own devices within policy; administration governs other changes.

The authority orders by compare-and-swap and returns signed receipts. Concurrent proposals never union member sets or use wall clocks to restore removed members. Authenticated equivocation freezes new work. The authority adapter requires an administrator to order fully approved changes; member relay retains the original signed encrypted proposal. One relay slot per author/group preserves exact retries, including receipt retrieval after removal.

Every committed transition defines a new Sender Key epoch. Cutover atomically retires live old keys and cancels pending old-state fan-out. Already accepted history remains readable; newly arriving old-state traffic is rejected after local cutover. Withheld updates and already in-flight traffic prevent instantaneous global removal guarantees.

## Private authority credentials

The reference is the [private-groups paper](https://eprint.iacr.org/2019/1416), draft 2020-11-09, SHA-256 `9205034e9d1448a5f020b7a78aea6290de005f311429cacc6781b1f544ca51fc`. Issuer and verifier share a pinned service. Foreign members use their home server’s authenticated federation proxy; issuance binds their published signed device identity, while subsequent group requests omit account/device identifiers. Federation authentication never substitutes for membership proofs or signed changes. No post-quantum membership privacy is claimed.

Profile 0 uses ristretto255 and independently derived SHA-512 points under `Sigil/private-credentials/v0`, with u32 length-prefixed labels: Gw, Gwprime, Gx0, Gx1, Gy1–Gy3, Gm3, GV, Ga1, Ga2. No known-scalar multiple of one shared base supplies these parameters.

UIDs are 16-byte issuer-scoped identifiers. One attribute hashes `UID || uid`; reversible encoding places UID in bytes 1–16 of a zeroed candidate and a little-endian u16 counter in bytes 17–18, choosing the first canonical nonidentity encoding. Decoding checks reserved bytes and recomputes that first encoding. This bounded UID-dependent search is not a constant-time plaintext encoder or service decryption oracle.

The issuer has seven secret scalars and 64-byte public parameters. Issuance is 352 bytes and binds issuer, UID attributes, redemption day and context. A 32-byte group master derives two scalars through SHA-512 with profile domain, `group-key`, index and master. Presentations contain eight points and a six-witness proof, totaling 480 bytes.

Fiat–Shamir binds proof kind, counts, length-prefixed context and ordered bases/targets/commitments; presentations additionally bind issuer/group parameters and day. Application contexts bind authority, operation, group, predecessor, body hash, nonce and a deadline of at most 120 seconds. Nonces commit with authorization. Daily credentials are cached under device/profile/day binding; issued credentials retain through-day validity.

Signed authority profiles pin server, generation, issuer and signing identity. Device-bound issuance UIDs require service-side collision retention. Credential/issuer checkpoints use separate authenticated storage bindings. Group masters and credentials never enter ordinary history recovery.

## Invitations and delivery

Administrator-signed `SGGI` offers bind target device, proposed member/role, expiry, genesis and master/profile context. They travel through verified encrypted direct channels. Receiving records an offer; explicit acceptance persists joining intent. The target replays signed history from genesis and freezes its exact Add approval against the current head. Rebase preserves the approved target/policy and requires a fresh signature. The inviter checks it before signing/ordering.

Temporary invited-device authority permits history reads only. Grants expire within seven days, are consumed on joining, and are cancelled by inviter demotion/removal, explicit cancellation or server restore. One active grant per target; at most 1,024 per group. Permanent IDs and 384-byte row reservations prevent reopening cancelled/consumed grants. Targets can decline only their own existing offers.

`SGGI`/`SGGA` controls are bounded to 4 KiB; ordinary history retains a 201-byte authenticated `SGIM` receipt. Queued outgoing invitations refresh membership and recheck cancellation and administrator authorization before sending; same-account device offers require current membership instead. Cancellation durably blocks queued sends and retries remote revocation; it cannot retract shared signatures, ordered membership or received plaintext.

Verified same-account device onboarding uses `SGGD`, explicit target consent and `SGGB` replay of the full signed membership log in 3 KiB fragments. Assembly and approval survive restart. An existing member device may sponsor addition without being an administrator; administrator ordering remains required. The joining device gets no temporary authority grant and performs normal group synchronization only after membership. Concurrent changes require replay and fresh approval against the new head.

Group-only PQXDH channels authorize exact signed membership without promoting direct-contact trust. Blocks, changed identities and replacements still gate traffic. An unverified initial may install only authorized group controls, never ordinary direct content. Background work synchronizes one group and one device per pass, processes approved relay work, grants recipient-owned mailbox admission and prepares distributions or current-position recovery.

New service pins wrap signed Sender Key packets in recipient-specific `SGGE` envelopes. AEAD binds group/recipient route, sender/recipient device, transport ID and expiry. Wrappers are frozen for exact retry. Existing pre-envelope groups preserve legacy retries through their current epoch and require wrapping after the next authenticated transition.

## Limits and history

The transport/authority still observes account/device routes, group handles/roster sizes/roles, stable recipient tags, lengths and timing. Former members may retain the master; outer wrapping neither revokes it nor heals Sender Key exposure. See [SenderKeys.md](SenderKeys.md).

Membership is bounded to 256 members and 1,024 devices, with at most 256 devices per member. Proposals, checkpoints, control assembly and pending fan-out have separate enforced bounds.

Earlier history requires an admin-signed `SGHG` grant binding source, target member/device, time range, current head and expiry (at most seven days). The supplier may be a nonadmin member. Any head or policy change invalidates pending transfer. `SGHT` transfers retained content through fresh group-authorized channels in acknowledged 48 KiB fragments, with restartable cursors and at most 16 active transfers per group. Content is rechecked before sending and importing; grant authorization and import share one write transaction. Deleted/expired content and unsupported retention formats are excluded. Live keys are never transferred.

Imported history preserves supplying-device provenance in a separate store; it does not assert freshly authenticated original authorship, execute structured actions or enter ordinary recovery. Retained file descriptors support authenticated download. Complete means the supplier's eligible retained records were transferred, not that all historical messages were recovered. Offline supply remains pending until expiry; absent supply is unavailable. Expiry releases pending work; revocation cannot retract plaintext already received.
