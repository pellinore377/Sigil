# Encrypted attachment contract

Files are immutable, split into 1 MiB plaintext chunks, including one authenticated empty chunk for a zero-byte file. Default server limit: 1 GiB; profile maximum: 1 TiB.

Each new file/version gets a random 32-byte ID and master key. HKDF-SHA-256 derives chunk keys using file ID, length and index. AES-256-GCM-SIV uses a fresh random 96-bit nonce; associated data binds the complete versioned header, file, index and lengths. Exact ciphertext is frozen before upload; changed source bytes conflict.

The sensitive 116-byte `SGAD` descriptor carries the file key and SHA-256 commitment to ordered ciphertext-chunk hashes, bound to ID/length/count. It travels inside authenticated messaging and may enter authorized media recovery. Authenticate each chunk and complete ordered coverage before publishing plaintext. Codecs: `protocol/src/attachment.rs`, `crypto/src/attachment.rs`.

Server ownership is account-scoped. Downloads require a live credential and separate access capability in a header; only its hash is stored. Foreign downloads use the home server’s authenticated federation proxy and the same capability/publication checks; the file host receives the capability in the signed request body. Keys/capabilities never enter URLs. Unfinished uploads have a 24-hour deadline; exact retries cannot extend deadlines or replace chunks. Quota reserves ciphertext and row overhead before upload.

Cancellation/deletion/expiry stop future access, release unused reservations and retain charged ID tombstones. Bounded cleanup releases retained bytes only when deletion commits. Cancellation before creation also reserves a tombstone. Restore cancels unfinished uploads and gates published files on owner reconciliation against a trusted root.

Native staging, exact retries, progress and cancellation persist across restart. The authenticated file event freezes its source server; foreign chunk downloads retain that route across cache/client restart. New download handoffs require authenticated retained content and current tombstone/expiry checks. Archive-managed cache references cannot resurrect deleted versions. Deletion cannot recall keys/plaintext already received.

Preview capabilities and fidelity limits live in `media/src/formats.rs`. The shared worker returns bounded text/cells, RGBA frames or interleaved little-endian f32 audio. Office pages/slides, spreadsheet cached values, vCard text and solid mesh previews have explicit simplifications. Interactive platform viewers remain client work.

Linux preview handoffs reauthenticate complete files, stage plaintext in anonymous `/dev/shm` files and recheck deletion/expiry before returning results. View-once access requires an already-consumed local viewing handle. Workers receive no encryption keys, host home directories or network access. Macro/Python/active-content execution is disabled in the isolated Office profile; PDFium has no V8/XFA. No scanning upload occurs and no file is labeled guaranteed safe.

Preview limits: two staged inputs and two workers, 128 MiB per input, 16 MiB output plus 1 MiB metadata, 2 GiB address space, 20 CPU seconds and 25 seconds elapsed. ZIPs have entry/expanded-size limits; images have pixel bounds. Larger files retain ordinary transfer/export support. Cancellation kills the process namespace; anonymous staging and private worker files disappear when handles/processes close. These limits do not claim forensic erasure or protection against host compromise.

Runtime setup and acceptance are in the README; current work is in [Status.md](Status.md). Attachment encryption uses [RFC 8452](https://www.rfc-editor.org/rfc/rfc8452.html); no independent cryptographic audit is claimed.
