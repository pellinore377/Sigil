# Encrypted history recovery

Archives contain retained content and conversation structure, never identity secrets, prekeys, live ratchets, credentials, verification decisions or group master keys. A replacement device needs account reauthorization and the recovery key; it establishes fresh messaging sessions. Recovery-key compromise exposes retained backups. Recipient copies cannot be recalled.

A separate random 256-bit secret derives the archive key through HKDF-SHA-256. Account scope binds the canonical server name and stable account ID. AES-256-GCM-SIV objects use random nonces and purpose-specific AAD; ciphertext SHA-256 identifies each object. Archive ownership does not prove another participant's authorship or current verification.

## Publication and restoration

`enable_history_recovery` returns the new secret for offline safekeeping. `sync_backend_due_online` integrates bounded history backfill, messaging, recovery publication and attachment work. Network lanes preserve independent durable deadlines/backoff. Platform callers schedule this blocking worker; onboarding and progress screens remain UI work.

Freeze records, pages and manifest before upload. Upload children first, then compare-and-swap the account head. Exact retries preserve ciphertext/generation; later edits enter a successor. A manifest holds at most 512 ordered pages of 256 record references. Upload/import steps process a page or at most 16 records. Imports authenticate every dependency before atomic finalization. Conversation structure/text become available before progressive media downloads.

`history_recovery_progress` reports backfill, staged/committed records, pending objects, unprotected records and last checkpoint time. `recovery_media_checkpoints` separately reports whether an independently owned media descriptor reached a confirmed checkpoint. Successful file upload alone does not establish archive protection.

Media backup downloads and verifies the original, then re-encrypts under an independent file key and republishes to the recovering account's server. Durable transfer state resumes across failures. Recovery descriptors bind their parent and original descriptor digest. Budget local cache space for both ciphertext copies plus SQLite overhead while republishing.

Trusted anchors accept an identical head, direct successor or at most 64 authenticated predecessor links. Restored server heads require reconciliation against a trusted checkpoint; repair preserves local edits/tombstones and republishes above the trusted generation. Ambiguous checkpoints remain blocked. A fresh device with only the recovery secret cannot detect replay of an older valid archive without an independent checkpoint.

## Retention and deletion

Account-private retention settings synchronize through encrypted conversation operations. Default remote retention is indefinite; a positive day count expires historical message/media records. Expiry produces permanent archive omissions while retaining existing local history under the device storage key. Increasing retention does not revive omissions. Disappearing, view-once and transient content are excluded.

Explicit deletion is permanent. Compaction removes obsolete message/edit bodies while preserving authenticated operation identities and reply structure; legacy history links propagate deletion to older archive records and media copies. Deleted content cannot return through ordinary archive merge or operation replay. File expiry also removes retained file keys logically.

After a confirmed successor, cleanup deletes superseded ciphertext under the exact server head. Authenticated references protect current records, pages and checkpoint manifests. Independently owned backup media is removed only after its omission/deletion reaches a confirmed checkpoint. Failed local completion writes leave cleanup retryable. Server deletion releases quota; it cannot erase copies already held elsewhere.

Bounded maintenance also erases obsolete transport/action bodies. SQLite clears superseded live pages and migration removes old free-page remnants. External copies and physical storage remain outside these guarantees; see [erasure boundaries](Security.md#erasure).

## Server API and limits

All routes require a live account-scoped device credential.

| Route | Contract |
| --- | --- |
| `GET /client/v0/storage` | Account usage/quota and recovery object capacity; warning at 90% |
| `PUT /client/v0/recovery/objects/{id}` | Immutable ciphertext; ID hashes raw bytes |
| `GET /client/v0/recovery/objects/{id}` | Caller-owned, non-deleted ciphertext |
| `GET /client/v0/recovery/head` | Generation, manifest and restored flag |
| `PUT /client/v0/recovery/head` | Exact-head CAS; explicit acknowledgement for restored-head repair |
| `POST /client/v0/recovery/objects/delete` | At most 64 IDs under the exact current head |

Objects are 36–67,266 bytes; request bodies are capped at 140 KiB. Archives hold at most 131,072 records. Accounts have 262,144 lifetime object IDs including tombstones. Ciphertext shares the configurable account quota (default 10 GiB); exhaustion rejects new storage without silently deleting history. Native schema 63 and attachment cache schema 4 reject older readers.
