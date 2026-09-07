# Experimental Triple Ratchet profile

This is an experimental implementation, not an enabled encrypted-event suite.
The native adapter now persists both ratchets and establishes them through PQXDH
profile 2. Independent review, protocol interoperability vectors and complete
application/transport integration remain required. No wire format is a production contract.

## Sources and interpretation

The algorithms follow the public-domain [Double Ratchet revision 4](https://signal.org/docs/specifications/doubleratchet/doubleratchet.pdf),
2025-11-04, sections 5–7, and [ML-KEM Braid revision 1](https://signal.org/docs/specifications/mlkembraid/mlkembraid.pdf),
2025-09-26. Pinned PDF hashes are in `plan.md`. Protocol code is written from the
specifications; no Signal implementation source is incorporated.

Four details require explicit interpretation rather than literal pseudocode:

* Braid describes `H(seed || vector)`. Standard ML-KEM encodes the public key as
  `vector || seed`; its Fujisaki–Okamoto transform hashes that standard encoding.
  Sigil uses `header = seed || SHA3-256(vector || seed)`, matching FIPS 203,
  libcrux and RustCrypto. Both implementations agree with the pinned NIST vectors.
* Delayed Braid fragments report `receiving_epoch = message.epoch - 1`, including
  fragments from an earlier negotiation. Returning the current local epoch for
  these messages would violate the specification's SCKA epoch-agreement property.
  Old fragments do not advance Braid. A future epoch can advance it only by one,
  from `Ct2Sampled`, as specified.
* SPQR section 5.6's receive pseudocode skips through the target counter and then
  advances once again. Sigil caches counters strictly before the target, then
  derives the target key. Previous-chain sealing includes its final counter.
* Section 7.2's chain KDF references an undefined `sk` and omits `ctr`, although
  section 5.2 requires the current chain key and counter. Sigil's concrete KDF
  below binds both, under its own experimental protocol identifier.

These choices need independent protocol review. Passing same-implementation
round trips is not evidence that every specification ambiguity is resolved.

## Cryptographic profile

All HKDF and HMAC operations use SHA-256. Integers use big-endian encoding.
Epochs are u64; message counters are u32 and start at one in SPQR, zero in the
classical component. Overflow fails without committing a candidate.

* `B = Sigil/experimental/braid/v0_MLKEM1024_SHA-256_RaptorQ64`
* `P = Sigil/experimental/spqr/v0_MLKEM1024_SHA-256_RaptorQ64`
* `T = Sigil/experimental/triple-ratchet/v0_X25519_MLKEM1024_SHA-256_AES256GCMSIV_RaptorQ64`

`HKDF(salt, ikm, info, length)` denotes extract-and-expand. `Z` is 32 zero bytes.

| Operation | Salt | Input key material | Info | Output |
|---|---|---|---|---|
| Split authenticated handshake secret | Z | SK | T + `:Initialize` | 64: EC seed, SPQR seed |
| Braid output key | Z | raw KEM secret | B + `:SCKA Key` + epoch | 32 |
| Braid authenticator update | prior authenticator root | output key | B + `:Authenticator Update` + epoch | 64: root, MAC key |
| SPQR initialization | Z | SPQR seed | P + `Chain Start` | 96: root, A→B chain, B→A chain |
| SPQR new epoch | prior root | Braid output key | P + `Chain Add Epoch` | 96: root, A→B chain, B→A chain |
| SPQR chain step | Z | current chain key | P + `Chain Step` + counter | 64: next chain, message key |
| Hybrid message key | SPQR message key | EC message key | T | 32 |

Braid authenticator initialization updates a zero root at epoch 1 with the SPQR
seed. Header MACs bind `B + :ekheader + epoch + header`; ciphertext MACs bind
`B + :ciphertext + epoch + ct1 + ct2`. HMAC verification is constant-time through
the library API. Internal authentication is retained in addition to outer AEAD.

Incremental ML-KEM-1024 uses libcrux-ml-kem 0.0.10 with only its ML-KEM-1024 and
incremental features. RustCrypto ML-KEM remains used by PQXDH and provides an
independent comparison implementation. No ML-KEM arithmetic is reimplemented.
The incremental API is explicitly experimental upstream. Owned private key and
encapsulation-state byte arrays are zeroized on drop; this does not establish
erasure of compiler-generated copies or library stack temporaries.

## Chunking and state bounds

RaptorQ 2.0.1 encodes only four fixed object sizes: header plus MAC (96), public
vector (1536), ciphertext part one (1408), ciphertext part two plus MAC (192).
Configuration is one source block, one sub-block, alignment one, 64-byte symbols.
Each fragment is a four-byte RFC 6330 payload ID plus a 64-byte symbol. Systematic
symbols are sent first, followed by fresh repair symbols. The 24-bit symbol ID
must never wrap. Encoders stop at exhaustion; a new session is then required.

Each decoder admits at most 64 distinct symbols, rejects another source block
and conflicting duplicates, and ignores identical duplicates. Transmission
parameters never come from the network. This bounds both storage and the size
of decoding work, although adversarial authenticated traffic still needs review.
Fountain codes are permitted by Braid section 3.6. Their probabilistic recovery
can require extra symbols compared with the suggested Reed–Solomon construction;
Sigil does not claim the same vulnerable-message-set bound.

SPQR uses section 5.7's previous-chain counter to erase old sending and receiving
chains when their respective epochs advance. It retains at most 128 skipped
message keys across epochs, and prunes keys more than two epochs behind the
latest sending/receiving epoch. At most three nonempty chain entries are admitted.
The classical component separately retains at most 128 skipped keys. Each
component derives at most 128 skipped keys per packet across old/new chains.
Larger gaps reject the candidate. At capacity, each cache evicts its oldest
retained insertion before adding a new skipped key. This prevents permanent
packet loss from exhausting the cache indefinitely. Eviction is committed only
after outer authentication; very late evicted messages can no longer decrypt.
This deterministic policy follows the bounded-deletion guidance in sections
8.4/8.7. Session replacement and additional age-based expiry remain work.

## Framing and commit boundary

Braid messages encode epoch (8), kind (1), and either no payload or one fragment
(68). SPQR adds previous-chain count (4) and message count (4). Triple Ratchet
headers contain `SGTR 00 01 00 00`, classical DH (32), classical previous count
(4), classical count (4), SPQR header length (1), and the SPQR header (17 or 85).
Total headers are 66 or 134 bytes. Plaintext is limited to 65,536 bytes.

AES-256-GCM-SIV uses the hybrid key once, with a zero nonce. Associated data is
`T || context[32] || complete_header`. The context must bind the authenticated
identities, negotiated protocol and handshake instance. Every Braid fragment,
counter, length, and classical header field is authenticated by the outer AEAD.

Sending stages both ratchets before encryption. Receiving stages both ratchets,
including skipped-key removal and Braid decoding/authenticator changes, before
verification. Any error discards both candidates. Successful in-memory commit
does not authorize network use. The native adapter atomically saves both states
and the exact outgoing packet, or the incoming message, before release.

A standalone Braid object is not a secure transport: its internal MACs do not
authenticate every fragment before buffering. Never commit a Braid candidate
based solely on partial decoding. An authenticated internal inconsistency requires
a fresh session; it must not trigger a downgrade to the classical ratchet.

## Handshake selection and encrypted checkpoints

Public prekey bundles retain the PQXDH-only profile-1 framing. Initial messages
select profile 2 in byte 5 (`SGPQ 00 02 02 00`) for Triple Ratchet. Its initial
AEAD key uses HKDF info `Sigil/experimental/pqxdh/initial/v1_TripleRatchet`, also
appended to the initial associated data. Reframing profile 1 as 2 or vice versa
fails authentication. Native acceptance requires profile 2, including retry paths;
it never falls back to classical messaging. Raw profile 1 remains a primitive
evaluation interface and an independent reference fixture, not a native session path.

Native initiating deliveries use `SGHI 00 02 00 00`: a four-byte big-endian
initial length, a fixed 1,694-byte profile-2 handshake encrypting empty content,
and the current Triple Ratchet packet carrying the actual message. The ratchet
context hashes the complete raw handshake, binding both ciphertexts. Both must
authenticate before consuming a prekey or committing session/history state.

Native schema 23 retains the exact handshake header, sealed under the storage
key and session binding. Until an authenticated peer packet commits, each new
send attaches that same header. Server acceptance never confirms the peer. The
receiver derives its stable local session reference from the handshake header
and peer/device context, so later initiating packets can establish the session
when earlier packets are lost. Delayed packets reuse the existing session and
bounded skipped-key caches, never the consumed prekey or a restored checkpoint.
The receiver's sealed header additionally binds its slot and expected sender.

Native schema 24 extends the sealed sender header with the first prepared
delivery's immutable expiry. Peer-aware initiation freezes it in the same
transaction as the initial packet and transport metadata. Later unconfirmed
deliveries cannot extend that window: default expiry is capped, explicit
extensions fail, and new text after the deadline rolls back without advancing
the ratchet or retaining an unsendable message. Authenticated peer traffic
removes this restriction for newly prepared deliveries; exact retries retain
their original deadlines. The caller still supplies a trusted clock.

Earlier sender headers with existing deliveries have no authenticated deadline;
migration does not infer one from queue order or silently reset it. New
unconfirmed deliveries on those sessions require a fresh handshake or an
authenticated reply. Existing frozen requests remain exact retries.

Confirmation derives from the authenticated ratchet checkpoint, without a
separate mutable status flag. New messages omit the header after confirmation;
already queued envelopes remain byte-identical retries. Fresh repeated-envelope
traffic participates in active-session selection; cached/replayed deliveries do
not. Retiring a session removes its retained header transactionally.

Initiating text is capped at 65,374 bytes, preserving the 67,230-byte mailbox
limit with the 12-byte frame, handshake and at most 150 bytes of ratchet overhead.
Confirmed bare ratchet messages retain the 65,536-byte primitive plaintext limit.
The existing 128-key derivation/cache budgets bound loss and reordering; losing
more than that before establishment can still require a fresh session. General
lost-session retry requests remain unfinished.

This replaces the previous empty-bootstrap envelope. Version 1 and unwrapped
initial packets are rejected on acceptance and outgoing retry; retained history
remains readable. Migration does not reconstruct missing handshake headers.
Unconfirmed older initiators without one require a fresh handshake before new
messages; no stored packet is rewritten or re-encrypted.

`SGTS 00 02 00 00` checkpoints encrypt the classical component, SPQR roots/chains,
bounded skipped keys and Braid state in one storage envelope. Raw state stays
inside the crypto crate in a preallocated, zeroizing buffer capped at 32 KiB.
Restore validates counts, epoch/role consistency, key-part relationships, framing,
and trailing bytes. It reconstructs KEM state from retained seeds/randomness and
RaptorQ state from fixed objects/cursors and bounded received fragments, avoiding
untrusted library-internal memory layouts. The largest sealed checkpoint admitted
by the native store is 32,804 bytes. This is not a rollback-resistant backup format.
Version 2 records skipped-key insertion order, including the embedded `SGRS`
version-2 record. Version-1 checkpoints preserve all existing keys on import;
their encoded order supplies the initial eviction order because chronological
age was not recorded. New writes use version 2, with unchanged size bounds.

Client schema 6 marks earlier sessions as legacy and preserves their stored
records. Their send/retry/delivery paths return `UnsupportedSession`; no classical
state is reinterpreted as hybrid state. Previously committed history remains
readable. New sessions require fresh handshakes. Incoming retry commitments hash
the complete packet before applying the existing keyed commitment, accommodating
the larger hybrid header without truncation.

## Validation and outstanding review

Tests cover all eleven Braid states and their checkpoint restoration; repeated
epochs with loss, reordering and asymmetric sends; malformed key parts; strict
framing; maximum skipped-key backlogs; restart around every send/receive; every-byte
message tampering across observed Braid kinds in both roles and epoch parities,
with complete checkpoint equality after rejection; downgrade rejection; and storage failures at PQ root creation
and epoch completion. These checks do not constitute an independent protocol audit.

`crypto/tests/reference.c --ratchet` independently generates the HKDF/HMAC vectors
using OpenSSL. Saved `ratchet.json` SHA-256 is
`677ae42490a251b4b4f249566218e71875da3d14aa858cf3677f823bc2ca1ace`.
The same C receiver verifies both PQXDH profile-2 fixtures in `pqxdh-triple.json`,
SHA-256 `3fd1531a72edbaea8f1445f3ece96486f81c7869458acb60fc332e77b6136bfc`.
OpenSSL/libsodium checks also pass with AddressSanitizer and UndefinedBehaviorSanitizer.
These are Sigil-profile fixtures, not upstream Signal interoperability vectors.

The current dependency scan reports no known vulnerabilities and one informational
unmaintained warning: `proc-macro-error2` 2.0.1, RUSTSEC-2026-0173. It enters through
the optional `cfg(hax)` dependencies of `hax-lib-macros`; the normal native build
does not compile it. No advisory is globally suppressed. Dependency maintenance,
constant-time behavior, secret erasure, protocol ambiguities and platform performance
remain review items before a secure-messaging claim.

## Identity signing and memory review

Identity signatures follow [XEdDSA revision 1](https://signal.org/docs/specifications/xeddsa/),
2016-10-20, sections 2.3 and 3. The pinned PDF SHA-256 is
`a65684d87d747934e5b05698ed20bec72cd3c30e3f7ff2f6376555d65a5104b7`.
Self-review found that the previous signing dependency used unreduced clamped
scalar bytes in the positive-orientation nonce hash and left local scalar/nonce
copies without an erasure contract. It has been removed. Signing now uses the
existing curve25519-dalek primitives, canonical reduced scalars in both
orientations, branchless orientation selection, 64 fresh OS-random nonce bytes,
and zeroizing owned secret scalar/encoding buffers. Verification keeps strict
canonical-S and weak-point rejection; it does not adopt the original document's
more permissive scalar encodings. Existing signed handshake fixtures still verify.

`crypto/tests/reference.c --xeddsa` generates 32 independent OpenSSL/libsodium
fixtures covering both orientations. `xeddsa.json` SHA-256 is
`8103cb1354bd79c64acd5e45f9f100088b9ac86c06ec539ef2a53452e74202b1`.
The C checks pass with AddressSanitizer and UndefinedBehaviorSanitizer. These
checks establish agreement with this specified signing profile, not a proof of
nonce security or a constant-time audit.

SHA-2 0.11.0 and HMAC/HKDF 0.13.0 replace the older crypto-crate hash/KDF versions.
SHA-2 and HMAC's digest-buffer zeroize features are enabled. Existing independent
KDF/MAC vectors are unchanged. SHA-256/SHA-512 hash cores and their buffered
input have erasure contracts; owned secret outputs use zeroizing buffers.
Temporary PRK/output arrays inside HKDF, HMAC key-pad temporaries, lower-level
primitive copies, compiler/register spills and physical storage remanence still
require review. This is deliberately not a claim of complete secret erasure.

Native peer-bound checkpoints additionally authenticate their peer reference.
The server's signed-device-binding directory does not establish trust: native
clients verify signatures and require independent comparison of the complete
binding fingerprint. Blocked or changed identities stop peer-bound live-session
operations. Bounded inbound routing trials candidates without committing failures;
ratchet/inbox/acknowledgement changes share one transaction. Session selection,
retirement, device linking and complete application events remain profile gates.

## Durable key-erasure boundary

A synthetic native-store probe confirmed that, after a session advances from
revision 0 to 1, the earlier sealed checkpoint can still occur in the live WAL.
Extracting those bytes and supplying the current storage key and original AAD
successfully opens the retired checkpoint. Authenticated revision bindings stop
accidental substitution; they do not erase earlier ciphertext or revoke the
static wrapping key's ability to decrypt it.

The probe also held a snapshot reader while attempting WAL truncation: SQLite
reported the checkpoint as busy and retained the WAL. Truncation succeeded after
the reader was released. That observation establishes logical file cleanup only,
not removal of filesystem snapshots or physical media remnants.

This is an endpoint-compromise concern: the attacker needs the storage key and
residual database data. It does not give a homeserver access to client keys.
Retained local messages and intentionally retained recovery history have their
own exposure semantics. Nevertheless, the in-memory ratchet's skipped-key
deletion and private-prekey retirement must not be presented as durable forensic
erasure or full forward secrecy for deleted content.

SQLite appends versions to its WAL, and readers can delay checkpoint progress;
ordinary checkpoints can recycle the file without truncating it. See SQLite's
[WAL documentation](https://www.sqlite.org/wal.html). Checkpoint/truncation and
[secure deletion](https://www.sqlite.org/pragma.html#pragma_secure_delete) need a
reviewed policy, but neither is by itself a complete platform key-erasure design.
The next persistence review must evaluate crash-safe wrapping-key retirement,
concurrent readers, filesystem snapshots, Android key capabilities and failure
recovery without silently reverting to old live keys. No such solution is claimed
implemented, and durability/WAL behavior has not been weakened to hide this gap.

Simply rewrapping the same database key under a new platform alias is not a
solution: compromise of the current database key still opens old ciphertext.
A candidate design would separate live-state encryption from retained-history
encryption, advance the actual live-state key, migrate only current live records,
and retire earlier key material after a verified durable cutover. This is a design
candidate, not an implemented guarantee. It needs a measured migration budget,
a precise exposure interval, fault tests around every cutover step, and evidence
that the platform prevents recovery or rollback of retired key material. Deleting
a keystore alias must not be assumed to prove those platform properties.

## Explicit logical session retirement

Native schema 20 adds an explicit retired-state marker. `retire_session` requires
all outgoing packets for that session to have authenticated server receipts or
explicit local expiry under schema 21 (no queued packet may remain). It authenticates the current Triple Ratchet checkpoint,
then atomically replaces it with a domain-separated, revision- and peer-bound
storage-key-sealed tombstone. Retrying retirement authenticates that tombstone.
A failed write leaves the prior checkpoint intact. Retirement also works for
blocked or quarantined peers; it does not grant them renewed verification.

Retired sessions reject new send/decrypt operations and are excluded from incoming
ratchet trials and the eight-active-sessions-per-peer count. Stored messages,
receipts and session/initialization ID tombstones remain, preventing ID reuse and
preserving history. Previously accepted delivery bookkeeping can still finish
without using ratchet keys. The 1,024 lifetime-session bound remains in force.

This is an explicit caller decision: delayed new packets can no longer decrypt
under the retired session. Complete session convergence, automatic retirement and
replacement policy remain unfinished. Logical retirement does not establish physical key
erasure: old WAL pages, filesystem snapshots or flash remnants may still contain
checkpoints decryptable after storage-key compromise, as described above.

Native schema 21 adds `expire_delivery(session, id, now)` for explicitly abandoning
retries after a prepared request's frozen deadline. The caller must supply a trusted
clock; unprepared packets cannot expire through this API. An authenticated expiry
marker and packet removal commit together, retaining history, message IDs and the
unchanged ratchet checkpoint. Repeated expiry is idempotent. This records cessation
of retries, not proof of non-delivery: a matching authenticated server receipt may
still supersede local expiry, including after session retirement.

`send_pending_online` now resolves expiry while processing the first 16 queued
packets, using the supplied trusted clock. It validates the entire selected batch
before committing any expiry; unprepared or damaged entries and failed writes
leave batch preparation unchanged. Remaining requests are transmitted only after
that transaction commits. `SendProgress` separates accepted and expired counts;
zero accepted alone does not mean the queue is empty. A transport error can follow
committed expiry or earlier acceptance, so retries resume durable state. The
read-only `pending_deliveries` API still reports expired entries without mutating
them. Background scheduling and platform clock policy remain pending.

## Durable active-session selection

Native schema 22 stores an authenticated active-session reference per verified
peer device, bound to that device's fingerprint. Newly bound sessions become
active. A newly accepted established message activates its receiving session in
the same transaction as ratchet advancement, event validation, history retention
and the incoming journal. Cached deliveries and replayed initial packets do not
change selection. Failed validation or writes leave the previous selection intact.

`send_peer_text` selects this session transactionally for a new message. An exact
retry instead follows its original delivery record and authenticates the original
session, recipient and content; it never re-encrypts on a newly active session.
Blocked/quarantined peers remain rejected. Retiring the active session clears its
selection in the retirement transaction, including for blocked peers, without
silently choosing an older session. Migration leaves selection empty until new
binding or newly accepted established traffic; recovery never imports it.

This implements the active-session insertion/receive-selection rule described in
[Sesame revision 2, sections 3.2–3.4](https://signal.org/docs/specifications/sesame/).
It is not full Sesame conformance: lost-session retry requests, approved device-list reconciliation,
multi-device fan-out and automatic stale-session retirement remain unfinished.
Existing eight-live-session and 1,024-lifetime-session bounds remain enforced.

## Signed lost-session retry requests

Native schema 25 adds a separate durable retry-control outbox and accepted-request
ledger. `prepare_retry_request` is an explicit caller report of an undecryptable
delivery. It signs a versioned 176-byte control with the existing device identity:
`SGRR 00 01 00 00`, original message ID, requester and target device fingerprints,
absolute expiry, and a 64-byte XEdDSA signature over the preceding 112 bytes.
The request ID hashes the two fingerprints and original message ID under a
separate domain, so requesting the same packet cannot silently renew its lifetime.
The randomized signature and expiry are frozen before network transmission.

This control is signed, not encrypted: message references, fingerprints, expiry
and request timing are observable to the server. It contains no conversation
plaintext. This is a constrained Sigil profile of the retry concept in
[Sesame section 4.1](https://signal.org/docs/specifications/sesame/), not a new
cryptographic primitive or a claim of full Sesame implementation.

`send_retry_request_online` sends one stored control through the existing HTTPS
mailbox and records its server receipt. Failed local receipt storage retries the
same bytes. `accept_retry_request` verifies the signature against the currently
verified device, outer routing/expiry, original delivery recipient and retained
text's two fingerprints. A valid request stages a deterministic fresh peer-bound
prekey claim and its ledger entry in one transaction. Duplicate requests cannot
create more claims; a differently signed/expired replacement cannot overwrite the
frozen request. `retry_request` reloads staged work and rechecks trust and expiry.
Retiring the original session does not remove its retained delivery evidence.

Bounds are 1,024 outgoing retry records, 4,096 accepted records and the existing
64 pending-claim budget. Requests expire within seven days using the trusted
caller clock. Unknown messages, missing retained text, changed/blocked peers,
malformed signatures and failed writes do not authorize a fresh claim.

Schema 26 adds `resend_event`: after the staged claim is filled, one transaction
creates a fresh session, selects it, and queues the original canonical text under
the request ID as its new transport ID. The logical ID, body and timestamp remain
unchanged. Restart retries reuse the committed ciphertext. Recipient acceptance
requires its authenticated outgoing request chain; an unrelated outer ID cannot
authorize a different logical ID. At most three resend hops are allowed. Exact
logical duplicates, including delayed originals, are flagged without creating a
second recovery record; changed plaintext is rejected.

Configured recovery deletion markers and changed retained bodies prevent stale
resending at preparation and in the HTTPS send worker, and prevent recipient
acceptance before committing prekey/session state. Already in-flight packets cannot
be recalled. Missing recovery records do not imply deletion; general retention and
local deletion lifecycle remain separate work. Schema 26 prevents older clients
from reopening this newly authorized cross-session history, without new tables.

Schema 27 adds a bounded, sealed retry-control acknowledgement journal. The
mailbox receiver takes a trusted caller clock and returns either a text event or
an authenticated, durably staged retry request. Retry-prefixed payloads are routed
to the strict signed-control parser; malformed controls never become text events
or acknowledgement candidates. Individual failures retain the existing cyclic
scan behavior.

The claim, accepted request and sequence-bound acknowledgement journal commit in
one transaction. The existing acknowledgement worker handles up to 16 text/control
entries combined in sequence order. Server success followed by a local write
failure safely retries after restart. The sealed journal authenticates the
sequence, request ID, owning device and acknowledgement state, and references the
retained accepted control. Subsequent expiry, deletion or blocking does not revoke
permission to acknowledge an already durably accepted control; it still blocks
resending where applicable. Text and control journals cannot accept the same
sequence. The new journal retains at most 4,096 entries; migration does not invent
acknowledgements for previously staged requests.

Schema 28 adds `resume_retries_online(now)`, which scans at most 16 accepted
requests using a sealed device-bound cyclic cursor. It fills a pending prekey
claim, prepares a fresh-session response when absent, and sends only that
response. Prepared responses reuse their ciphertext without repeating the claim
or changing active-session selection. Existing authenticated delivery receipts
report server acceptance without further transmission, even after expiry or
blocking. Completion is proved by those receipts. Acknowledging a control does
not mean its response was delivered.

Each item returns its receipt or error. Local failures do not starve later records;
network errors stop the batch and expose the error so the caller can honor
Retry-After. Cursor updates compare the previously sealed value, and a failed
cursor write cannot roll back already committed claims or server receipts. Empty
end-of-scan batches reset the cursor. The caller must schedule invocations and supply the
trusted clock; this is not an autonomous background runtime.

Schema 29 removes completed and explicitly cancelled requests from the active
scheduler using an indexed `finished` field authenticated inside the sealed
request record. Delivery receipt storage marks the request completed in the same
transaction. Earlier completed requests are classified when next scanned. A
caller can still inspect server acceptance through the delivery receipt API.

`cancel_retry_request` atomically abandons the accepted request's prekey claim,
clears only its queued response ciphertext, and seals the cancellation. This
works before claiming or after preparation, survives restart, and prevents request
replay from reactivating the work. It does not block or retire the peer/session,
alter the ratchet, or remove unrelated queued messages. Already durable server
acceptance wins over cancellation; an authenticated late receipt can likewise
record acceptance and supersede the cancellation. Neither cancellation nor local
expiry proves non-delivery, and in-flight packets cannot be recalled.

Active-queue cleanup retains the signed request, acknowledgement journal and
logical history evidence needed for replay detection, delayed messages and retry
chain limits. The existing lifetime ledger caps still apply; physical compaction
and safe reclamation of those proofs remain unfinished. This is not secure disk
erasure.

The scheduler automatically cancels already accepted work when its authenticated
signed deadline has passed, or authenticated retained history proves deletion or
a changed body. `Error::Obsolete` distinguishes those content changes from general
conflicts. The decision, claim abandonment, queued-packet removal and finished
marker share one transaction, using the existing schema-29 cancellation state.
It checks again after claim I/O before sending; known server acceptance takes
precedence. A failure during cancellation preserves the original claim and packet.

Invalid clocks, clock rollback beyond the request window, blocked/changed peers,
missing or corrupt evidence and pending recovery imports are not cancellation
proof. They leave work available for a later attempt. The trusted caller clock
still governs deadline decisions.

Mailbox dispatch now resolves controls first encountered after deletion,
supersession or signed expiry through an authenticated discard path. It verifies
the signature, current peer trust, both device fingerprints, outer routing and
expiry, original delivery metadata/content and bounded retry chain before allowing
discard. Expiry alone is not authentication. Unreadable history, missing evidence,
clock rollback and trust failures remain errors without acknowledgement authority.

For a new obsolete control, the existing sealed request ledger stores a cancelled
record and the acknowledgement journal commits in the same transaction; no claim
or session is created. `MailboxEvent::DiscardedRetry` tells the caller that this
control resolved without scheduling another response. If previously accepted work
became obsolete, shared cancellation atomically abandons its claim and clears its
queued response, preserving known server acceptance. Finished-control replays
cannot reopen work. The existing acknowledgement worker handles these records and
survives restart using the same authenticated journal. The strict
`accept_retry_request` API still rejects obsolete input rather than silently
changing its return contract. No schema or wire-format change was needed.

Schema 30 adds `reclaim_retry_journals(now)`, a local maintenance operation that
scans at most 16 journal entries per call. It deletes only authenticated server
acknowledgements whose retained control is terminal and whose signed deadline has
passed. Completed controls additionally require their authenticated delivery
receipt with matching expiry. Active, unacknowledged and still-live records remain.

A sealed device-bound cyclic cursor prevents an ineligible prefix from starving
later entries and survives restart. Cursor progress and deletions share one
transaction; corruption or a failed write rolls the batch back. The caller supplies
the trusted clock and schedules maintenance. Reclaimed sequence journals no longer
provide a cached acknowledgement: replayed controls must pass authentication again,
can only resolve against their retained terminal request, and receive a fresh
acknowledgement journal. They cannot recreate claims or sessions. This does not
advance a mailbox high-water mark or discard unseen lower-sequence deliveries.

Request/chain proofs, delivery receipts, text deduplication and recovery records
remain untouched. This reclaims journal capacity, not guaranteed physical disk
erasure.

Schema 31 adds `reclaim_outgoing_retry_controls(now)`. Dependency discovery
authenticates the bounded outgoing ledger (at most 1,024 full records), and one
transaction replaces at most 16 expired, unreferenced controls with compact expiry
tombstones. A retained inbox response or another outgoing request referencing a
control pins its full proof. Response lookups use an inbox-message index. A child
retired in one batch may release its parent for a later call.

Tombstones seal the owning fingerprint and original expiry under the request ID.
They prevent a repeated request for the same failed transport ID from acquiring
a fresh deadline after its signed control is reclaimed. Retirement works for sent
or unsent expired controls. Full outgoing capacity is freed, while tombstones are
capped at 4,096; reaching that cap stops further retirement without deleting proof.
Corrupt dependency records or tombstones fail closed, and write failures roll back
both retirement and deletion. Maintenance uses the caller's trusted clock.

Already retained responses, cached acknowledgement and retry-chain validation
keep their proofs. An unseen response arriving after its authorization deadline
and reclamation can no longer use the removed control to authorize its transport
ID; it is rejected. Ordinary original-message acceptance remains independent.
Tombstones are not guaranteed physical erasure and do not provide unlimited
retention.

Schema 32 adds `reclaim_accepted_retry_controls(now)` for expired cancelled or
discarded controls that never prepared a response. It authenticates at most 4,096
full controls and 4,096 journal references, and retires at most 16 candidates in a
transaction. Active controls, response outbox/delivery records, child requests and
remaining acknowledgement journals pin their full proofs. Reclaim eligible journals
first. Prepared-response proofs remain intact for retained history and late receipts.

Compact accepted-control tombstones seal the owner, peer, original packet digest
and signed expiry under the request ID. Exact replays are freshly authenticated
against the peer and original message, then checked against the tombstone before
being discarded and journaled. A new randomized signature or renewed expiry cannot
replace the original packet. The acknowledgement worker and journal reclamation
also authenticate compact proofs, without recreating a full control or claim.
Tombstones are capped at 4,096; reaching the cap stops further retirement safely.
Write failures roll back tombstone creation and full-record deletion together.

This frees eligible full-control capacity but deliberately retains compact replay
evidence and proofs referenced by messages. Remaining lifetime retention limits
and end-to-end recovery acceptance under those limits remain open; this is not
unlimited retention or guaranteed physical erasure.

Cross-feature acceptance now exercises actual loss of the original private prekey
through the real HTTPS fixture: the old packet fails decryption and is not
acknowledged; a signed retry is durably dispatched/acknowledged; an unavailable
fresh prekey returns a network error without creating a session; both clients
restart; a new publication lets the scheduler recover the original logical text.
The recipient retains one history entry, replies on the fresh session, and only
that authenticated reply confirms the sender's session. A further restart and
ordinary message succeed. Local cleanup at the signed deadline reclaims the
control journal while retaining response-dependent proofs. Maintenance clocks are
advanced separately after the network checks; this is not a week-long live test.

Quota acceptance fills all 1,024 outgoing-control slots, verifies refusal of a new
request, reclaims 16 expired controls and verifies that a new request fits. It
then fills the 4,096 compact-tombstone ceiling with authenticated synthetic records
and verifies that retirement stops without deleting remaining full controls.
These checks do not remove the documented limits or constitute an independent
security audit.

Schema 33 resolves the leftover undecryptable original after logical recovery.
When ordinary mailbox acceptance fails, `resolve_failed_delivery` follows at most
three deterministic outgoing retry IDs and uses indexed inbox lookup to find a
stored replacement. It authenticates the request chain, replacement content,
conversation/device bindings and an already committed logical-event ledger entry.
Missing replacement content or prior logical acceptance never authorizes deletion.

A separate sealed journal binds the failed sequence, peer, original transport ID,
packet digest, expiry, replacement/session and acknowledgement state. Its write
commits before server acknowledgement. Mailbox callers receive
`RecoveredDelivery`, not another text event. The existing acknowledgement worker
handles text, controls and recovered originals within one shared batch of 16,
revalidating proof before network I/O. Failed receipt persistence retries the same
sequence after restart. Conflicting packet/expiry substitutions, corrupt journals
and cross-journal sequence reuse fail closed. The failed original is never
decrypted or inserted as a second logical message by this operation.

The lost-prekey HTTPS acceptance test now verifies that the next cyclic scan
resolves the original and empties the mailbox after acknowledgement. Additional
checks cover missing proof, journal-write rollback, substitution, corruption and
lost acknowledgement receipt; the three-hop retry test also resolves its original.
The new journal is capped at 4,096 entries.

Schema 34 adds `reclaim_recovered_journals(now)`. It shares the existing journal
maintenance implementation and inspects at most 16 entries per call. Only sealed,
server-acknowledged recovered deliveries past their recorded expiry are reclaimed;
pending or still-live entries remain. Acknowledgement is authenticated from the
sealed journal, not trusted from the SQL flag. Replacement messages and request
proofs are untouched. Reclamation does not require the peer to remain unblocked
after an already recorded server acknowledgement.

Retry-control and recovered-delivery maintenance have separate device-bound cursor
rows and authentication labels. Migration preserves the existing retry cursor and
adds no inferred recovered cursor. Both use cyclic scans, with progress and deletion
in one transaction. Failed writes roll back the batch; a pending prefix cannot
starve later eligible entries. Replay after reclamation must prove recovery again
and create a fresh acknowledgement journal before deletion from the server.

Regressions cover pending/live preservation, forged acknowledgement flags,
cursor-write rollback, restart, replay without duplicate history, a 16-entry pending
prefix followed by two reclaimable records, and cross-cursor substitution. The
batch-boundary fixture uses synthetic authenticated local acknowledgements; the
separate HTTPS test covers actual server acknowledgement and lost receipt recovery.
Broader lifetime retention limits remain; this does not establish physical erasure
or completion of the full backend.
