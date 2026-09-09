# Session and device contract

Identity verification, account authorization and group membership are separate. See [DeviceLinking.md](DeviceLinking.md) and [TripleRatchet.md](TripleRatchet.md).

## Delivery and recovery

- New sends freeze canonical content, timestamp and verified recipient before network work. Use a confirmed selected session or a durable fresh prekey claim. Exact retries retain their original session, ciphertext, routing and expiry.
- Authenticated replies supersede unconfirmed initials; confirmed sessions use shared transcript ordering. Replayed/cached deliveries cannot change selection.
- Missing prekey/session evidence may produce a recovery offer. Explicit caller approval rechecks trust and commits one signed request. Invalid traffic, capacity/storage failures and identity changes do not authorize recovery. Response chains allow at most three hops.
- Sync uses bounded work with durable cursors. The scheduled entry point reserves one minute before I/O, normally waits five seconds, backs off to 300 seconds and honors Retry-After from completion. Platforms own serialized workers, trusted clocks and wakeups.
- Automatic session/prekey retirement requires a fresh empty mailbox check from sequence zero and seven-day observed inactivity grace. Active selections and pending packets remain protected. Offline maintenance cannot infer absence of delayed traffic.
- Replacement requires explicit old/new fingerprint approval for distinct devices of the same account. Server inventory, unblock and reauthorization cannot revive superseded trust.

## Contact requests

A request is signed metadata, separate from message delivery and device trust. It binds the target server/account, expiry and sender's signed device binding. Decisions bind the exact request signature, preventing delayed approval of a renewed introduction. Acceptance never grants mailbox/prekey access; fingerprint verification remains explicit. Android retains the draft until verification and persists requests/decisions before network work.

Native APIs use `/client/v0/contact-requests`: GET/POST collection, GET/PUT `policy`, PUT `blocked`, GET `outgoing/{recipient}`, and GET/PUT `{id}`. The incoming GET requires `signature`; PUT takes `state` and `signature`. Federation carries authenticated `contact_request` and `contact_status` services. Requests expire within seven days; blocks survive expiry/restore. Limits: 64 incoming pending requests, 32 outgoing per account, 128 pending per foreign origin, 4,096 recipient records and 65,536 server records. Each retained row charges 2,048 bytes. Restore discards requests while preserving blocks and opt-out.

## Capacity and persistence


Retained sessions/messages, consumed prekeys, completed controls and link tombstones use storage budgets rather than fixed lifetime event counters. Exhaustion rejects writes; it never authorizes deleting history or replay evidence.

| Active resource | Bound |
| --- | --- |
| Sessions / pending packets | Eight live sessions per peer; 256 pending packets per session; 256 pending text intents |
| Private prekeys / claims | 64 live slots and 64 pending claims; replenishment targets eight bundles |
| Server mailbox | 256 pending per recipient; 64 per sender/recipient |
| Recovery controls | 1,024 outgoing and 4,096 accepted unfinished requests; cleanup at most 16 per pass |
| Devices / peers | 256 active devices per account; 4,096 nonsuperseded native peers |

The default native main-database page budget is 1 GiB; callers can consistently supply 1 MiB–1 TiB through `open_with_storage_limit`. Rollback journals and migration vacuum need additional space. Server quotas charge live bytes and retained record reservations transactionally. Increasing a budget preserves the same identities/state. Physical storage erasure has [platform limits](Security.md#erasure); restoring live client snapshots is unsupported.
