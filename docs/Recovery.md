# Experimental encrypted history recovery

Recovery archives contain retained history, not live session databases. Restoring
must create fresh transport credentials and fresh encryption sessions. Identity
private keys, ratchet checkpoints, prekeys, transport credentials and verification
decisions are not archive record types.

The encrypted object format, native snapshot/import store and opaque server
storage are implemented experimentally. End-user recovery is not enabled and
independent backup/security review remains required.

## Key and object model

A separately generated random 256-bit recovery secret derives an archive storage
key with a distinct HKDF-SHA-256 domain. A 32-byte account-scope binding must cover
the canonical homeserver and stable account reference. It is authenticated with
every object; display names and current transport-device IDs are not scopes.
Archive keys must never derive from live message/session keys or user passwords.

The native scope is SHA-256 of `Sigil/recovery/account/v0`, the canonical DNS
name's two-byte big-endian length, that ASCII name, and the 32-byte stable account
ID. Native schema 8 wraps the recovery secret and trusted/pending heads with the
independent local storage key. The server receives none of those keys.

Objects use the existing AES-256-GCM-SIV storage primitive with fresh random
nonces, separate recovery-purpose associated data, and strict bounded framing.
The object identifier is SHA-256 of its ciphertext envelope. Authenticated parent
objects contain child identifiers, so the server cannot substitute another valid
object or splice a record from another snapshot without detection.

An encrypted manifest identifies its generation, preceding manifest, and ordered
page identifiers. An encrypted page holds at most 256 sorted unique record
references. Each reference binds a stable archive entry ID, positive revision,
and encrypted record identifier. A manifest references at most 512 pages.
These bounds are experimental capacity limits, not a final retention policy.

Records contain only history metadata and retained content, or a deletion
tombstone. Their IDs/revisions must match their authenticated page references.
Recovery callers must explicitly classify retained content; view-once,
disappearing and unclassified transient events are excluded. Import applies
monotonic record revisions and preserves tombstones against older records.
Archive possession is not proof of another participant's authorship or current
identity verification.

## Publication and recovery

Upload immutable encrypted records/pages before publishing the manifest. The
server's account-scoped head uses compare-and-swap with a monotonic generation
and exact retry semantics. A failed or interrupted upload must leave the old
head intact. Account quota includes recovery objects as well as queued delivery.
No automatic eviction of retained history is allowed on quota exhaustion.

The client must durably retain the exact encrypted objects and intended head
before transmission. It verifies the head against its decrypted manifest, then
validates pages and records before importing history. Partial recovery needs
explicit progress and must never populate live ratchet/session tables. Media
references can be restored before media content when their format is implemented.

The native upload queue returns at most 16 objects. Acknowledgements require
authenticated server responses; publication is unavailable until every object
has an upload acknowledgement. Local edits can continue while a snapshot is
frozen, and enter the next snapshot. A competing authenticated successor can
replace pending upload staging and merge with preserved local edits. Network
requests must be serialized by the caller. Imports stage one page at a time,
revalidate every reference at finalization, and commit records plus the trusted
head atomically. Cancelling import discards staging only. Deleted entry IDs
cannot become retained content again, even under a higher revision.

The native HTTPS adapter uploads at most 16 objects per step and persists each
acknowledgement before proceeding. Downloads commit a page or at most 16 records
per step, allowing restart within a page. Schema 9 caches completed pages but
finalization still authenticates all references and records; the cache cannot
authorize an import. Server publication followed by local failure retries the
same head. Network requests verify certificates, refuse redirects and enforce
bounded response bodies/timeouts. Credentials and custom trust roots are stored
under the independent local storage key. Scheduling/onboarding remain separate.

Existing anchors accept the same manifest, a direct authenticated successor, or
a chain of at most 64 links ending at the exact trusted generation and identifier.
Multi-generation imports pin the target manifest and durably stage one missing
predecessor manifest per download step. Every link authenticates before pages or
records can be staged. All import entry points recheck the chain, and finalization
commits history, the newest anchor and the bounded proof ledger atomically.
Forked, corrupt, missing and over-limit chains never expose partial history or
advance the anchor. Cancellation discards staging only. A pending upload uses a
separate competing-proof path before a distant import can replace its staging.
Fresh imports require explicit acknowledgement that
there is no independent rollback anchor. The importer rejects server heads
flagged as restored checkpoints. A limited repair path now exists for a surviving
archive with an independently trusted anchor: `reconcile_restored_recovery_head`
requires an authenticated server response whose generation and manifest exactly
equal that anchor, the locally prepared direct successor, or a proven ancestor.
It refuses fresh archives, unproven or conflicting heads, and
pending imports. It never discards an ambiguous upload or imports
the restored snapshot. `reconcile_restored_recovery` obtains that response through
the connected account's HTTPS adapter.

Repair authorization is sealed with the unchanged anchor and survives restart.
Without a pending upload, the next ordinary snapshot includes the complete current
local archive, including newer revisions and deletion tombstones. A pending direct
successor instead retains its exact manifest and ciphertext. Repair resets every
object acknowledgement because a restored backup may have lost those objects.
Repeated explicit repair authorization resets them again; normal retries resume
through upload steps without reauthorizing. Edits/deletions made after the frozen
snapshot remain local and enter the following snapshot, as with ordinary uploads.
If the restored head instead equals the pending successor, the client verifies
the locally sealed manifest's link to the trusted anchor, then atomically advances
the anchor, clears resolved upload staging and retains repair authorization. This
resolves a lost publication acknowledgement; it does not accept a competing
successor chosen by the server. Local retained records and later edits remain
untouched. The next snapshot incorporates those edits and repairs from the new
anchor. A failed transaction retains the original anchor, staging and exact retry.
Transferred object acknowledgements may be reset: resolving an exact published
manifest does not rely on their values, and the subsequent repair uploads its
complete object set again.

All objects in the repair snapshot must be acknowledged
before publishing an explicitly acknowledged restored-head CAS. Successful local
publication commit clears authorization atomically; a lost response or failed
commit retries the same ciphertext and intended successor. While repair is
authorized, an unflagged winning direct successor may resolve a concurrent repair:
its manifest must authenticate the exact local anchor before handover discards
the losing staging and clears both repair authorization and the obsolete server
CAS base. Local records and post-freeze edits remain untouched until ordinary
monotonic import completes. A failed handover transaction preserves the original
pending ciphertext and repair state. Other imports remain blocked during repair.
After a publication conflict, callers can use the existing download steps to
import the direct winner, then prepare/upload their merged successor. They must
continue to respect `Retry-After` during this flow.
For a winner several generations ahead, native schema 19 stages at most 64
ancestry manifests separately from the unresolved upload. Its sealed target binds
the archive control-state ciphertext that started the check. Downloads persist one
required manifest at a time. Wrong objects, a chain ending at another anchor,
over-limit targets and stale control state fail closed. Local history edits and
object acknowledgements may continue without changing that control state.
Cancellation deletes only the proof candidate. Once the complete chain authenticates,
one transaction replaces the upload with import staging and clears obsolete repair
state. History and the trusted anchor still wait for normal import finalization.
If another operation commits new archive control state, callers must cancel the
stale candidate explicitly. History-only handoff refuses active proof staging.
The HTTPS download adapter drives this path automatically after a distant conflict.

Older sealed archive states remain readable; states
with the repair marker fail closed in older readers.

Native schema 18 retains at most 64 encrypted manifests when publication or import
commits. Starting from the trusted anchor, each manifest must authenticate the
exact previous identifier and generation before an older restored checkpoint can
authorize repair. The local anchor never moves backward and no old records are
imported. Missing, corrupt, forked or out-of-window proofs fail closed. Migration
preserves existing history but cannot invent proof of earlier publications.
The bounded manifest ledger is authenticated again during history-only handoff.

For a proven older server head, the sealed repair state separately remembers that
head as the server CAS base. The repair snapshot continues the newest local anchor
and publishes above its generation using `restore_generation`. Pending uploads
keep their original head/ciphertext and reset acknowledgements. Repair uploads also
queue the available bounded ancestor manifests, because restore may have lost the
proofs needed by other anchored readers. These are the original encrypted manifests,
not old plaintext or regenerated records. Proof objects participate in the same
bounded upload batches and must be acknowledged before publication. Handoff
authenticates these extra rooted objects as part of the frozen queue. Server publication
requires the exact restored CAS base and explicit restore acknowledgement; an
ordinary unflagged head cannot authorize a generation jump. Exact retries remain
valid after the restore flag clears, and a later competing publication makes the
request stale. All current local history and tombstones remain intact.

This is not general operator-restore recovery: an unproven older checkpoint,
a competing branch not linked to the trusted anchor or a lost local anchor still
requires a separate policy.
An initial pending upload without a previously trusted anchor remains blocked.
Actual server restore revokes device credentials.
Reauthorize into a fresh client store, then use the surviving archive's
`copy_recovery_history_to` to transfer committed authenticated records, its
independent recovery key and trusted anchor into that same account. The destination
must have no archive or live messaging/verification state. The handoff validates
every record, wraps archive state under the destination storage key and commits
atomically; the source remains unchanged. Pending imports are refused. A pending
upload transfers its exact frozen snapshot by traversing and authenticating the
manifest, each page and every referenced record. Missing/corrupt/extra staging
objects fail the entire handoff. All transferred object acknowledgements reset;
the source's acknowledgements remain unchanged.
No identity, ratchet session, prekey or verification state is copied, and prior
repair authorization is not carried over. The replacement then calls
`reconcile_restored_recovery`, `prepare_recovery_upload`, and bounded
`upload_recovery_step` until the successor commits. These are native APIs;
onboarding and user-facing recovery orchestration remain pending.
Automatic garbage collection also remains unfinished; no deletion request is
generated blindly.

## Opaque server API

All routes require a live native device credential and are scoped to its account.
Account reauthorization preserves archive access but supplies no decryption key.

| Route | Contract |
| --- | --- |
| `PUT /client/v0/recovery/objects/{id}` | `ciphertext` is lowercase hex; ID is its raw ciphertext SHA-256. Exact retries are immutable. |
| `GET /client/v0/recovery/objects/{id}` | Returns the caller's non-deleted ciphertext only. |
| `GET /client/v0/recovery/head` | Returns generation, manifest ID and restored-checkpoint flag; generation zero/null means no head. |
| `PUT /client/v0/recovery/head` | Compare-and-swap with `expected_generation`, `expected_manifest`, new `manifest`, and optional restored-checkpoint acknowledgement. Optional `restore_generation` advances beyond the normal successor only for an acknowledged restored head; exact retries are preserved. The manifest must already exist. |
| `POST /client/v0/recovery/objects/delete` | Up to 64 unique object IDs, guarded by the exact current generation/manifest. Clients must establish that encrypted child objects are unreferenced. |

Objects are 36–67,266 raw bytes; request bodies are capped at 140 KiB. The account
has at most 262,144 lifetime object IDs, including deletion tombstones. Deletion
frees ciphertext quota but prevents upload retries from resurrecting the same
object ID. The current manifest cannot be deleted. Missing objects or stale
heads roll back entire deletion batches. Mailbox and archive writes share the
configured account quota in one immediate transaction; archive ciphertext is
charged in raw bytes and existing mailbox hex payloads in stored encoded bytes.
Server schema 12 also charges retained session/device protocol-record reservations
against this quota; see [SessionsDevices.md](SessionsDevices.md). Archive-object
lifetime policy remains part of backend item 5, outside the completed item 1.

Server schema 8 preserves archives during operator restore and marks all heads
as restored checkpoints. Publication requires explicit acknowledgement of that
condition, and object deletion is blocked until a new head is committed. That
server mechanism does not replace the native reconciliation gate above.

A previously trusted manifest anchors rollback detection. A fresh device with
only the recovery secret cannot detect a malicious server replaying an older
valid archive without an independently trusted checkpoint. Authenticated
tombstones protect ordinary merges/imports; they do not solve this fundamental
fresh-device rollback limit. Availability and retained metadata remain visible
to, and controllable by, the server. Server backup restoration must not silently
present a stale recovery head as current.

When recovery is configured, typed ordinary-text sends and newly accepted
incoming text retain a revision-1 history record in the same transaction as the
message state. Archive failures roll back that transaction. The stable record ID
binds the conversation, authenticated author identity and logical message ID;
direction is relative to the owning account. Exact retries preserve later
revisions and tombstones. Pending upload snapshots remain immutable while new
text is retained for the next snapshot; pending imports block new retention.

Earlier messages are not automatically backfilled when recovery is enabled.
`recovery_records(after)` discovers at most 16 authenticated committed records
per call, including tombstones, using the last returned ID as its next cursor.
Staged imports remain invisible until their entire authenticated import commits.
Records are ordered by ID, not sender timestamp. Restart the scan after archive
changes; this cursor is not an incremental change feed or a cross-call snapshot.

Archival tombstones do not delete local inbox/outbox copies or cancel queued
delivery. Recovery-code UX, other platform key protection,
full history classification/deletion, media handling and independent review
remain integration gates.

## Typed retained media increment

History record content kind 2 contains canonical `SGFC` file metadata, its static encryption key/root descriptor, source and access capability. Kind 0 remains explicitly retained opaque/text bytes and kind 1 remains deletion. Parsing never upgrades ordinary bytes into media by guessing a prefix. Unknown/malformed file content fails; type changes under an existing record identity conflict, and deletion remains terminal. Native schema 46 prevents earlier native clients from opening a store whose records/events they cannot interpret. Existing recovery object encryption/framing and text encodings are unchanged; this is not a new ratchet export format.

Direct and group file capture commits with normal message/key state. `prepare_recovered_file` checks the connected/recovery/cache account scopes and reads the current authenticated local archive record. A fresh reauthorized device with a different local storage key successfully imports a typed media record over HTTPS and downloads/decrypts its file while its sessions, identities, prekeys and peer-verification tables remain empty. Existing recovery-checkpoint authority and rollback limitations still apply. Expired descriptors cannot start/read the media cache. Archive expiry cleanup follows below; deletion propagation into all existing media caches remains unfinished; no claim of complete media retention or forensic key erasure follows from this test.

## Automatic archive work and retained-file expiry (native schema 47)

`sync_recovery_due_online` runs a dedicated platform-owned archive worker. It
publishes changes under an already configured recovery key, resumes exact pending
uploads, and continues explicitly authorized imports/competition proofs. Its
independent encrypted scheduling slot uses the existing durable reservation,
completion-time Retry-After and backoff rules. It never accepts an unanchored
initial archive or authorizes a restored server checkpoint on its own. Callers
honor `next_at` and handle `scheduling_error`; lifecycle wakeups remain platform
work. One upload pass transfers at most 16 objects and may then publish the head;
snapshot preparation still visits the bounded complete archive transactionally.

Encrypted dirty state commits with retained edits/deletions and resets only when
a new immutable snapshot freezes. Changes made during that upload survive for
its successor. Finishing an import compares retained revisions and record
coverage with the imported archive: matching history becomes clean, while local
newer/additional records require a successor. This avoids unchanged import/upload
echoes. Schema migration preserves the messaging scheduler and conservatively
marks historical archive work dirty, including edits beside an older prepared
upload. The archive scheduler does not require a live encryption identity.

`delete_recovery_record(id, expected_revision)` creates a terminal authenticated
tombstone with optimistic revision checking and exact-repeat support.
`maintain_recovery(now)` works offline, examines at most 16 committed records per
pass with a durable fair cursor, and removes expired file keys from those records.
It defers changes during a staged import; the recovery worker resumes that import.
New snapshot preparation also normalizes every expired file record, so records
beyond the current maintenance batch do not enter a newly frozen snapshot with
expired file keys. An already prepared snapshot stays immutable; subsequent
changes require its successor. Original text encodings and retention defaults
are unchanged. This is logical key removal, not forensic erasure of SQLite/WAL,
old archives, or recipient copies.

Five integration tests cover bounded atomic expiry/restart, deletion during an
actual HTTPS upload, independent Retry-After/reservation persistence, schema-46
migration with pending edits, full snapshot expiry, and authorized fresh import
without restoring sessions/identities/prekeys/peers or echoing an unchanged head.
The migration/bulk-snapshot case uses low-level synthetic acknowledgements;
network scheduling has separate real HTTPS tests. Existing local message
journals, already queued delivery and previously prepared media caches still
need deletion propagation. Onboarding/backfill, full recovery UX, retention
settings, account quota warnings and independent review remain open.

## Archive references and cache deletion (native schema 48, cache schema 3)

The main store maintains a keyed content index for every committed file recovery
record. File capture, edits, tombstones and imports update it in the same
transaction; history handoff rekeys the index for the destination's local storage
key. Migration authenticates existing committed history before publishing the
new index/schema. A positive lookup reauthenticates the referenced record. The
index exposes equality between identical file bodies inside the local database,
but contains no file key, filename, source URL or access capability.

Authenticated received/group/recovery handoffs mark a cache entry as managed
when its current recovery record exists. The transfer worker and offline
`maintain_attachment_cache` use the existing bounded fair cursor to evict a
managed entry once no committed archive record references its exact file body.
A different retained reference preserves it even if that reference was never
separately opened in the cache. This is local eviction: no server delete request
is inferred from deleting a recovery entry. The ordinary cache cleanup limit
remains 16 chunks per pass. Legacy/unclassified cache entries stay unmanaged
until a new authenticated archive handoff; earlier history is not guessed.

`recovered_file_chunk` checks the specified current recovery record before
returning a reauthenticated complete-file chunk, so deletion blocks that read
before background cleanup. Cache/main operations consistently lock the cache
first and reserve the main store against concurrent archive edits; only cache
writes commit in these handoffs. Low-level `Cache::completed_chunk` checks cache
integrity and expiry, not archive authorization. Already returned plaintext,
original message journals, already submitted packets and recipient copies cannot
be recalled by this API. General message/queued-delivery deletion remains open.

Three additional tests cover shared references, immediate read denial, failed
index/key-cleanup writes, migration authentication/rollback, legacy-cache
classification, index rekeying on history handoff, and atomic imported tombstones.
Existing real HTTPS fresh-media import and event tests still pass.
