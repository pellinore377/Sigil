# Group and federation security analysis

Status: design constraints and acceptance gates, not an implemented or reviewed protocol. Pairwise Triple Ratchet does not complete this milestone. The plan still calls for Sender Keys/private groups; no replacement group protocol has been selected and no upstream implementation code has been incorporated.

Local implementation progress (2026-09-06): [Groups.md](Groups.md) now defines and tests membership authorization and encrypted local ordering journals. [SenderKeys.md](SenderKeys.md) records the next primitive/integration contract. These increments do not close this document's credential privacy, authority service, federation or independent review gates.

## What the references establish

Signal's private-group design encrypts membership entries and uses keyed-verification anonymous credentials to authorize access. Its issuer and verifier are the same service. Adapting that trust structure to independently operated homeservers therefore requires an additional design; this is an inference from the centralized model, not a federation property proved by the paper. The paper also explicitly leaves state-version integrity and removal-time master-key refresh as extensions. Existing or former members can disclose information they already learned. [Private-group paper, draft 2020-11-09](https://eprint.iacr.org/2019/1416), [Signal's design explanation](https://signal.org/blog/signal-private-group-system/).

The private-group paper uses classical discrete-log/DDH assumptions. It does not establish post-quantum membership privacy. Its downloaded PDF SHA-256 is `9205034e9d1448a5f020b7a78aea6290de005f311429cacc6781b1f544ca51fc`. This reference must not be presented as a proof of Sigil's future group system.

Sender Keys use separate sender chains distributed to group members. Symmetric chain advancement alone cannot heal exposure of the current chain. Fresh key distribution and its authenticated channels matter; refreshing one sender does not refresh every other compromised sender. This limitation must be reflected in the group security claims even when distribution uses Triple Ratchet. [Balbás, Collins and Gajland, Sender Keys analysis, version 2](https://arxiv.org/abs/2301.07045v2). PDF SHA-256: `23b7285aa1dcbec272764b865dee8fa0b0dde8db2755dd1daabadd197a0bcc81`.

The current [libsignal license](https://github.com/signalapp/libsignal/blob/main/LICENSE) is AGPLv3. It has not been adopted. A dependency/license decision and suitable reviewed primitives remain prerequisites for implementing anonymous credentials; this document does not authorize importing that implementation or writing a new proof system from scratch.

## Required separation of authority

| Authority | May establish | Must not establish by itself |
| --- | --- | --- |
| Homeserver account authorization | Access to that account's server resources | An existing verified encryption identity or group membership |
| Independently verified device binding | The peer's server/account/device/key association | Authorization to replace another device or add every account device to a group |
| Approved device-link transcript | The specific new device authorized by an already trusted device | Arbitrary server-supplied device-list changes |
| Group membership transition | The next authorized member/device/role set | Retroactive authority over earlier messages or other groups |
| Federation transport authentication | Which homeserver submitted a bounded request | End-to-end sender identity, content integrity, or a peer read receipt |

Device linking must bind both device keys, account/server identities, challenge randomness and the complete confirmation transcript. Key changes and new devices cannot inherit verification through OIDC, an administrator action, a copied display name or a restored server database.

## Group transition requirements

The following are Sigil design requirements, not a claim that the cited protocols already implement them:

1. Every group has an unambiguous genesis identifier and authenticated state history. A transition binds the predecessor hash, revision, exact member/device/role set and authorizing devices. Clients validate authorization against the predecessor, not merely the server's current response.
2. Concurrent administrative changes must not silently merge membership sets, resurrect a removed member or resolve authorization by wall-clock ordering. The coordination/fork-resolution algorithm remains to be selected. Until then, detecting a fork must stop new group-key distribution. Availability during partitions is an explicit tradeoff, not an excuse to ignore conflicting state.
3. Membership or device removal must retire affected sender epochs and distribute fresh entropy only to the new authorized device set. Define the atomic cutover, treatment of delayed old-epoch messages, offline senders and acknowledgement failures before implementation. A removed member's retained plaintext cannot be revoked.
4. Bind sender-key distributions and group messages to group identity, membership-state hash/epoch, sender device/key reference, chain identifier, message counter and format version. Reject a valid distribution transplanted from another group or epoch. Persist counters, ciphertext and fan-out retry identity together before transport.
5. A fresh sender key sent over a compromised or not-yet-healed pairwise channel does not establish group post-compromise recovery. The refresh policy and exact classical/post-quantum claims need a composition review. Classical identity/signature authentication is a separate limitation.
6. Recovery exports retained history, never resumable live sender chains. Prior-history grants require explicit group authorization and must honor disappearing/view-once exclusion and deletion policy. Restored devices establish fresh credentials, verification/linking and sessions.

## Federation requirements

Federation remains disabled. Its first contract must specify canonical server identities, bootstrap and rotation of server verification keys, bounded discovery and independently versioned requests. Server signing keys must be separate from Admin and client credentials.

[HTTP Message Signatures, RFC 9421](https://www.rfc-editor.org/rfc/rfc9421.html) is a candidate transport mechanism, not an adopted implementation. A concrete profile would need fixed covered components, method/target/body binding, creation/expiry rules and nonce replay handling. [Digest Fields, RFC 9530](https://www.rfc-editor.org/rfc/rfc9530.html) provides body-integrity fields; a bare digest is not authentication. TLS hostname and certificate verification remain mandatory.

Outbound federation must enforce destination policy after resolution and pin the validated connection addresses, preserve the intended TLS hostname, reject redirects and bound DNS/connect/body work. Public federation must not become an SSRF route to loopback, link-local or private services. Operator-configured private/VPN peers need a distinct explicit trust policy. None of these rules may rely on untrusted forwarding headers.

Recipient admission and quotas must survive a remote server inventing unlimited local accounts/device IDs. Retry identifiers must be scoped to authenticated origin and destination, and exact replay cannot extend expiry or multiply stored payloads. Per-origin resource budgets, durable outbound receipts and failure isolation are required. A remote acknowledgement means that server accepted responsibility; it is not proof that a device decrypted anything.

Encrypted membership storage does not hide all delivery metadata. Routing endpoints, timing, sizes, fan-out patterns and server collusion remain observable unless a separately designed anonymous-delivery layer addresses them. Sigil must state those limits without claiming anonymous or post-quantum-private groups.

## Acceptance matrix before implementation is enabled

| Scenario | Required result |
| --- | --- |
| Malicious homeserver changes a verified peer/device list | No silent trust transfer or new group-key delivery |
| Two administrators change the same predecessor | Detect conflict; never union removed members back in |
| Removed member and server replay old group state | No new-epoch access or silent state rollback |
| Sender key copied across groups, devices or epochs | Reject before plaintext acceptance |
| Crash during key rotation or fan-out | Exact retry with no counter/nonce reuse or partial local membership commit |
| Lost, reordered or duplicated distributions/messages | Bounded recovery with explicit undecryptable states |
| Account restore or new phone with old backup | Recover permitted history; never resume old sender/ratchet keys |
| Compromised sender chain followed by refresh | Demonstrate the exact recovery boundary and distribution assumptions |
| Cross-server request/body/target substitution or replay | Reject; preserve quotas and retry identity |
| DNS rebinding, redirect, slow peer or invalid certificate | Refuse or time out within bounded resources |
| Credential issuer compromise or collusion with a former member | Document and test the chosen privacy/authentication limits |

The next group deliverable is a concrete authority, concurrency and credential-issuer protocol with independent review. This analysis identifies the work; it does not mark that gate complete.
