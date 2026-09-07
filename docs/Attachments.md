# Encrypted attachment transfer contract

Status: implementation in progress. This organizes item 7 and supplies a future bounded transport for authorized history bundles; it does not close either item. UI remains paused.

## Content and cryptographic boundary

Use immutable files split into 1 MiB plaintext chunks, including one authenticated empty chunk for a zero-byte file. The default server file limit remains 1 GiB; this experimental profile has an explicit 1 TiB maximum. The server receives ciphertext, file references, lengths and upload state. Filenames, media types, encryption keys and access capabilities belong inside authenticated encrypted application content.

Generate a fresh random 32-byte file identifier and master key for every new file/version. Derive each chunk key with HKDF-SHA-256, the file identifier as salt, and a versioned context binding total plaintext length and chunk index. Reuse the existing AES-256-GCM-SIV implementation with a fresh random 96-bit nonce. Authenticate the complete chunk header: version, file ID, index, total length, chunk plaintext length and nonce. Strictly validate the expected index and exact final-chunk size. Existing message and storage primitive size limits stay unchanged.

The primitive is [AES-GCM-SIV, RFC 8452](https://www.rfc-editor.org/rfc/rfc8452.html); its misuse resistance does not authorize intentional nonce reuse. Segmented authentication follows the general requirement that chunks stay bound to their file and position, also described in [Tink's streaming-AEAD overview](https://developers.google.com/tink/streaming-aead). This is a Sigil framing profile, not Tink interoperability or an independently reviewed streaming construction.

Freeze exact ciphertext before upload. A changed source chunk under an existing local job is a conflict, never an upload retry. The descriptor contains the file key and a SHA-256 commitment to the ordered list of ciphertext-chunk hashes, bound to file ID, length and chunk count. It must travel only in authenticated encryption and may be retained for authorized media recovery. It is not live messaging-ratchet state. A receiver authenticates every chunk before using it, and verifies complete ordered coverage and the descriptor commitment before publishing a completed file. Chunk authentication alone is not proof of complete-file receipt.

## Durable lifecycle

The server must authenticate upload ownership, reserve quota before accepting chunks, support exact retries without extending expiry, refuse conflicting chunk replacement, and publish only a complete immutable file. Download authority must be separate from the encryption key; do not expose a key in an HTTP path, query, log or server record. Cancellation/deletion/expiry revoke future access and preserve retry tombstones without resurrecting removed data. Physical cleanup must be bounded and quota accounting must reflect both reservations and retained bytes.

The client must persist encrypted descriptors, frozen chunk jobs, progress and cancellation across restart. Network failure must retain exact work; accepting a response and failing a local write must reconcile safely. Downloads remain private staging artifacts until authentication and completeness checks pass. No preview worker receives messaging keys, file keys, arbitrary networking, macros or document scripts.

## Acceptance slices

1. Bounded cryptographic framing, ordered-file commitment, strict parsing, independent vectors and substitution tests.
2. Durable client staging, changed-input detection, exact retries, cancellation and restart.
3. Authenticated resumable server storage, publication, quotas, deletion/expiry and bounded cleanup.
4. Authenticated download, completeness, interrupted transfer and normal worker integration.
5. Format capability registry and isolated preview processing, with explicit full/simplified/download-only distinctions.
6. Conversation authorization, encrypted media recovery, retention and deletion propagation.

Do not label the attachment feature complete from its cipher or upload endpoint alone. General history sharing still needs group authorization and explicit provenance; transporting a file does not establish either.

## Cryptographic increment evidence

`sigil_crypto::attachment` implements the bounded shape, chunk key derivation/encryption, sensitive 116-byte descriptor and ordered ciphertext-hash accumulator. Four focused tests pass: complete small-frame byte tampering/truncation, file/key/index substitution, empty/full/final chunks, maximum shape without whole-file allocation, strict descriptors, incomplete/reordered commitment rejection and independent OpenSSL vectors. Existing workspace Clippy passes with warnings denied. This increment adds no dependencies.

The C reference generator's `--attachments` mode independently derives chunk keys and encrypts empty, short and multi-chunk synthetic files. Fixtures include complete small ciphertexts and the hash of a full 1 MiB chunk, keeping test data compact. Rust matches the keys, ciphertexts/hashes, complete-file commitments and descriptors. The C generator passes ASan/UBSan. Fixture: `crypto/tests/vectors/attachments.json`, SHA-256 `0b78c216890d393bf49770ab9e11906e9b0284790f8bfe139cb22ecd226a01fc`. Durable client/server lifecycle and preview work remain open.

## Server storage increment

Use the existing authenticated same-server client API. Upload ownership is account-scoped; it establishes storage authority, never the E2EE content author. Download needs a currently authorized device credential and a separate file access capability in a header. Store only its domain-separated hash. Never put that capability or a file key into a URL. Cross-server retrieval remains a federation dependency.

Reserve declared ciphertext plus bounded chunk/row overhead before upload, with a separate 24-hour unfinished-upload deadline. Published files may be retained indefinitely or carry a frozen explicit expiry. Exact creation/chunk/publication retries never extend either deadline or replace bytes. A complete ordered list of chunk hashes must match the uploader's expected root before publication. Status and bounded chunk-index pages support diagnostics and reconciliation.

Cancellation, deletion, expiry and account disable stop further reads/writes immediately. Release unused reservations immediately; continue charging physically retained chunks until bounded cleanup deletes them. Retain one charged file-ID tombstone to prevent resurrection. Server restore cancels unfinished uploads and flags completed files as inaccessible restored checkpoints. Re-enabling a completed file requires explicit owner reconciliation against the expected root; the future client must gate that decision on trusted retained-history state. A server snapshot by itself cannot establish deletion freshness.

Cancellation also creates a charged, owner-scoped tombstone when the file does not yet exist. This covers cancellation arriving before a delayed creation request. Later creation conflicts instead of resurrecting the upload; another account cannot take over the tombstone. Failed tombstone insertion rolls back its quota charge.

Schema 13 implements this same-server storage contract and advertises `attachment_storage: [0]` separately from the still-disabled complete encrypted-event capability. Binary chunk requests/responses are capped at 1 MiB plus 84 framing bytes. Metadata uses the existing smaller request limit. Chunk-index pages contain at most 64 entries. Cleanup transitions at most 64 expired/disabled files and deletes at most 64 chunks from one terminal file per pass; actual retained chunks stay charged until deletion commits. Each file-ID tombstone costs a retained 512-byte reservation; live chunk accounting additionally reserves 256 bytes per chunk for row/index overhead.

Seven server integration tests pass, including real encrypted chunk/descriptor agreement, HTTP authorization/origin/body/header boundaries, competing writers, failed quota/cleanup writes, partial upload restart, immutable publication, expiry/disable, schema 12→13 migration, bounded 65-chunk cleanup, and restore reconciliation. All 94 server tests pass. Docker acceptance additionally checks encrypted-file persistence after kill/restart, inaccessible restored files, exact owner repair, and immediate read denial after deletion under the existing non-root/read-only resource limits. The server accepts opaque ciphertext; only the client can establish AEAD/content validity. Durable client upload/download and conversation/recovery integration remain unfinished.

## Native upload increment

The native HTTPS adapter transfers exact bounded binary chunks over the existing fixed authenticated origin. It rejects redirects, unexpected content types/compression, truncated/oversized responses and malformed progress claims; file capabilities use sensitive headers. Three focused tests cover real TLS transfer and response boundaries.

`ClientStore::open_attachment_cache` derives an account-bound cache wrapping key and opens a separate private SQLite database with an explicit caller-persisted page budget. A two-GiB cache budget accommodates the default one-GiB file plus database overhead; WAL files require additional disk space. This budget does not reserve free filesystem space. Messaging database and small-record limits remain unchanged. Ciphertext chunks and their sealed hashes/progress commit together. Sensitive file descriptors, names, media types and capabilities stay sealed; no plaintext files are staged.

Upload preparation freezes file identity, key, metadata and expiry. Staging rejects changed source bytes under an existing chunk index. Finalization verifies ordered complete coverage from authenticated chunk records without rereading whole files. The upload worker performs one network operation per step and freezes the possibility of in-flight creation before sending. Receipt-write failure retains exact work; cancellation after possible submission requires the server tombstone. Cancellation logically removes the local descriptor/key and bounded cleanup removes at most 16 ciphertext chunks per pass. This is not forensic erasure.

Only confirmed publication exposes a descriptor for future encrypted-message creation. Manual reconciliation can observe expiry/removal but never automatically authorizes a restored server checkpoint. Five cache tests pass, including competing stagers, restart, wrong key/account, malformed metadata, corruption, exhausted storage, failed upload/publication receipts and cancellation-before-creation. An eighth server test covers the cancellation ordering and quota contract. Download staging and normal scheduling/event/recovery integration remain unfinished.

## Native download, scheduling and cache retention

Download preparation explicitly requires an already authenticated, conversation-authorized descriptor. The low-level API does not establish that origin, and cross-server retrieval is not enabled. Each accepted ciphertext chunk passes AEAD before its progress commits. Reordered exact chunks can resume across restart; changed descriptors or chunks conflict. No plaintext is exposed until complete ordered coverage matches the descriptor commitment. Each later chunk read rechecks its retained ciphertext hash and AEAD. Expiry and local cancellation stop reads; cancelling a received download does not delete the sender's server file.

Cache schema 2 adds a durable worker schedule and bounded scan cursor. `sync_attachments_due_online` runs on a dedicated platform-owned blocking transfer worker, leaving chat polling independent. A pass scans at most 16 file records and performs one selected transfer operation. It reserves a retry window before I/O, measures Retry-After from completion, backs failures off to 300 seconds, and preserves newer reservations against stale completions using the same scheduling rules as messaging. Successful active work is eligible one second later; idle scans wait five seconds. Platform adapters still own wakeups and must handle the returned scheduling error. Actual network throughput/rate limits have not been benchmarked or tuned.

Upload conflicts schedule a status reconciliation. Removed/expired files become terminal locally. A restored publication becomes `Restored` and leaves automatic network work; the client never grants the restore-acknowledgement flag automatically. Previously authenticated local ciphertext remains readable, while new descriptor handoff stays blocked until the remote-access decision is resolved. This increment does not implement that authorization decision.

Expiry logically clears cached keys and schedules bounded ciphertext cleanup. Explicit eviction of completed/terminal cache entries removes at most 16 chunks per pass and then removes the cache row, allowing storage reuse without retaining a permanent client transfer tombstone. An evicted file may be downloaded again only from a separately retained authenticated descriptor. The caller must retain needed descriptors in history before evicting; that conversation/recovery handoff remains unfinished. Pending upload jobs require cancellation first, so eviction never silently abandons possible remote creation.

Four download tests and four scheduling/eviction tests pass, in addition to the earlier five upload-cache and three HTTPS tests. Tests cover partial/corrupted data, wrong complete roots despite valid individual chunks, restart, failed writes, expiry, account separation, Retry-After, bounded progress and restored-checkpoint refusal. The server/container suite covers actual restore revocation; the client scheduler test isolates the post-restore marker while retaining test credentials.

The explicitly invoked optimized `one_gib_file_stages_restarts_and_releases_its_independent_cache_budget` test stages a full 1 GiB using a reused 1 MiB input buffer, finalizes/reopens it, checks retained chunks and root, and releases all 1,024 chunks through bounded cleanup. In one local run: 2.50 s staging, 2.9 ms finalization, 0.26 s cleanup, 1,075,228,672 database bytes. This is storage-path acceptance, not a production throughput or platform-memory claim. The test is skipped in ordinary workspace runs because it writes a large temporary file. Encrypted conversation events, media recovery, formats and isolated previews remain unfinished.

## Authenticated conversation and recovery handoff

Native schema 46 connects the cache to direct and group file events, documented in [Events.md](Events.md). Publication, account scope, verified peers/current group authorization and exact message retry identity gate outgoing handoff. Incoming handoff resolves retained authenticated journal content; recovered handoff resolves typed archive records. Both reject wrong-source or deleted/superseded descriptors. Direct/group message transactions capture static media keys in typed recovery records, with no live ratchet export. Five integration tests cover normal HTTPS delivery, group Sender Keys, failed atomic writes, lost-session resend/deduplication and actual fresh-device media download without restored identities/sessions.

Transfer storage, transport, scheduling and these handoffs are implemented increments. General message/archive retention and cache deletion propagation, authorized restore re-publication decisions, cross-server retrieval, the required format/viewer matrix and isolated preview processing remain open. Item 7 is not complete.

### Archive-managed cache retention

Native schema 48 indexes all committed file recovery references under the local
storage key. Cache schema 3 distinguishes entries prepared from current archive
records. The regular transfer worker or offline `maintain_attachment_cache`
evicts these entries after their last matching reference disappears, preserving
other references and leaving server ciphertext untouched. New archive-aware
viewers use `recovered_file_chunk` to reject a deleted record immediately;
low-level cache reads alone do not establish that authority. Existing legacy or
unclassified transfers require a new authenticated handoff before this retention
policy applies. See [the recovery contract and evidence](Recovery.md).
