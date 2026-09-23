# Account recovery and encrypted history

## Account key

Each account has one XEdDSA account key, separate from device identities. It signs an endorsement of each device binding fingerprint (`Sigil/account-endorsement/v1`). The server stores the public key once, verifies endorsements before a device joins, and lists them in the contact directory. Contacts pin the account key when they accept a contact and trust exactly the devices it endorsed; a different key pauses the contact until the user approves a review digest over the new key (`Sigil/contact-identity-review/v2`). Verifying one endorsed device by QR verifies the account.

## Recovery secret and passkeys

A random 256-bit recovery secret derives the backup key and seals the account key into a server-stored bundle (HKDF-SHA-256 salted with the account scope, AES-256-GCM-SIV). A passkey wraps the secret under its WebAuthn PRF output, bound to the scope and credential ID; the server stores only wraps (at most 16). The optional recovery code is the secret itself in groups of four and is shown only in Settings. The server never sees the secret, a PRF output or the account key.

## Signing in on a new device

Password, SSO or an administrator's recovery invitation on an account that has a key or devices creates a pending device: it can read only its pending state, the account key bundle and passkey wraps (`GET /client/v0/pending`). Unlocking the bundle, endorsing its own binding and `POST /client/v0/pending/activate` make it a device; nothing else changes. `POST /client/v0/pending/reset` is the explicit last resort when the secret is lost: a new account key, every other device signed out, wraps and backups cleared, and contacts asked to approve the new key. Pending devices expire after an hour.

Backups carry the contact catalog (contact address and pinned account key) so a recovered device lists every conversation; the recovery secret authenticates it like the account key itself.

A stolen trusted device holds the account key and secret. Account-key rotation after removing a device is not implemented; replace passkeys and the code by resetting if a device is lost to someone else.

## Publication and restoration

Signing in starts backups under the account's recovery secret. `sync_backend_due_online` integrates bounded history backfill, messaging, recovery publication and attachment work. Network lanes preserve independent durable deadlines/backoff. Platform callers schedule this blocking worker; onboarding and progress screens remain UI work.

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

Backup routes require a live account-scoped device credential. Account-key routes: `GET/PUT /client/v0/account-key`, `GET /client/v0/recovery-wraps`, `PUT/DELETE /client/v0/recovery-wraps/{id}`; pending routes are above.

| Route | Contract |
| --- | --- |
| `GET /client/v0/storage` | Account usage/quota and recovery object capacity; warning at 90% |
| `PUT /client/v0/recovery/objects/{id}` | Immutable ciphertext; ID hashes raw bytes |
| `GET /client/v0/recovery/objects/{id}` | Caller-owned, non-deleted ciphertext |
| `GET /client/v0/recovery/head` | Generation, manifest and restored flag |
| `PUT /client/v0/recovery/head` | Exact-head CAS; explicit acknowledgement for restored-head repair |
| `POST /client/v0/recovery/objects/delete` | At most 64 IDs under the exact current head |

Objects are 36–67,266 bytes; request bodies are capped at 140 KiB. Archives hold at most 131,072 records. Accounts have 262,144 lifetime object IDs including tombstones. Ciphertext shares the configurable account quota (default 10 GiB); exhaustion rejects new storage without silently deleting history. Native schema 77 and attachment cache schema 5 reject older readers.
