# Experimental direct-text events

The automatic native mailbox receiver now requires an authenticated `SGEV` text event. Opaque crypto/handshake APIs remain evaluation primitives; raw plaintext is not silently upgraded into an application event. The complete `encrypted_event` capability remains disabled while group, control, edit, media and compatibility contracts are unfinished.

The version-1 encoding is fixed and big-endian:

| Field | Bytes | Meaning |
| --- | --- | --- |
| Prefix | 8 | `SGEV 00 01 00 00` |
| Kind | 1 | `1`, direct text |
| Retention mode | 1 | `0`, ordinary retained text only |
| Logical message ID | 32 | Matches the mailbox ID except for an authorized lost-session resend |
| Conversation reference | 32 | Canonical pair of stable account references |
| Sender binding fingerprint | 32 | SHA-256 of the sender's complete device signing statement |
| Recipient binding fingerprint | 32 | SHA-256 of this receiving device's signing statement |
| Sender timestamp | 8 | Positive, at most signed-64-bit maximum; presentation metadata, not an expiry authority |
| Body length | 4 | Exact UTF-8 byte count |
| Body | 1–65,224 | UTF-8 text |

The 150-byte header and body fit the native initial envelope's 65,374-byte plaintext ceiling. Initial and subsequent text events share the same body limit. Unsupported versions/kinds/retention modes, invalid UTF-8, empty/oversized bodies, inconsistent lengths and trailing bytes fail. Rich SigilText semantics are not inferred from a string's contents.

An account reference is SHA-256 of `Sigil/account-reference/v0`, the server-name byte length (u16), canonical server bytes and stable account ID (32 bytes). A conversation reference hashes `Sigil/direct-conversation/v0` followed by the two account references in lexicographic order. It is independent of local session IDs and device replacement. Device fingerprints still bind the exact endpoints, including their encryption keys, in each event. This profile currently accepts only the same homeserver on both endpoints; federation remains separate work.

`start_claimed_text` creates this content using the verified frozen claim. `send_text` validates the peer and commits ciphertext, ratchet state, retained outgoing content and frozen delivery metadata together. Callers preserve the original timestamp and message ID on retry; changing either is a conflict. The lower-level byte-oriented methods do not supply these application guarantees.

Incoming acceptance checks the inner IDs, conversation and both fingerprints before committing the ratchet, prekey consumption, inbox or acknowledgement journal. A valid AEAD tag alone is insufficient. Cached incoming retries are checked again, and acknowledgement validates the committed event before requesting server deletion. Blocking after acceptance can stop new traffic while still permitting acknowledgement of already accepted content. Unknown peers and identity changes never gain verification from a valid event.

`Incoming::text()` borrows the decoded event from zeroizing retained plaintext. Local byte-oriented history APIs return the canonical event bytes for these messages. With recovery configured for the connected account, new ordinary text is archived in the same transaction as its initial/send/receive operation. Raw byte-oriented APIs do not infer recovery classification. Earlier-history backfill and structured history presentation remain to be connected; recovery never restores verification or live session state.

Native schema 17 adds encrypted logical-event deduplication, scoped to the own device, peer and message ID. A new session cannot redefine an already accepted logical event, even if the ciphertext authenticates. Ordinary retries on the original session preserve the same event and exact ciphertext. The ledger retains at most 4,096 entries. Earlier valid journal entries establish their ledger record before acknowledgement; a failed write cannot authorize early deletion.

Schema 26 permits byte-identical logical content across sessions only when the recipient's authenticated retry ledger authorizes it. The signed retry request ID becomes the response transport ID; the inner event bytes stay unchanged. The recipient validates a chain of at most three requests back to the logical ID and caps response expiry at the signed request deadline. `Incoming::duplicate` marks previously accepted logical content, including delayed originals and cached journal entries. Callers use `Incoming::text().message` for timeline identity and the outer message ID for transport bookkeeping. Recovery keeps one logical record. Configured recovery deletions or changed bodies block resends before preparation, again in the HTTPS send worker, and before incoming acceptance. General deletion/cancellation and automatic discard acknowledgement remain incomplete.

Recovery records use the authenticated author encryption identity, account-pair conversation, sender timestamp, account-relative direction and UTF-8 body. Their IDs hash `Sigil/direct-text-history/v0 || conversation || author_identity || message_id`; recipient-specific delivery fields are excluded so future device fan-out can share one logical history entry. An existing archival revision or deletion marker is not overwritten by a send retry. This does not implement local message deletion or cancellation of already queued delivery. Enabling recovery later does not automatically backfill earlier messages; upload scheduling and the onboarding flow remain work.

Tests exercise every fixed framing field, Unicode and size limits, maximum-size text through actual HTTPS initial/reply delivery, exact retry timestamps, atomic outgoing delivery failure, substituted outer IDs, and authenticated events with wrong conversations/endpoints. Every rejected incoming application event leaves the ratchet and inbox uncommitted. Legacy raw initial content remains available through its existing history APIs and is rejected by automatic application acceptance.

## Ordinary retained file events (native schema 46)

The shared event decoder preserves text kinds 1/2 and adds direct-file kind 3 and group-file kind 4. Direct and group headers remain 150 and 118 bytes respectively. All use ordinary retention mode 0; this does not implement disappearing/view-once or rich SigilText events. Text-specific APIs reject file content rather than reinterpret it as text. The complete `encrypted_event` capability remains disabled.

The canonical `SGFC` body binds source homeserver, display filename, media type, optional file expiry, access capability and the existing `SGAD` file-key/root descriptor. It is at most 805 bytes. All of it travels inside authenticated message encryption. One shared bounded descriptor/metadata parser supplies protocol, crypto and client validation. The native path currently requires the source to be the same homeserver as the authenticated conversation endpoints; it does not follow a descriptor to an arbitrary origin. Display names/MIME do not grant parser or filesystem authority.

`queue_peer_file` and `queue_group_file` require an account-bound, confirmed published cache entry and keep the cache locked against cancellation while the messaging transaction freezes the event. They reuse ordinary peer verification, session selection, durable send intents, Sender Keys and receipt journals. Generic `resend_event` replaces the text-only name and preserves the same signed-request authorization and exact inner bytes for both types. Earlier text encodings and history IDs are unchanged. Typed direct/group file history IDs use separate domains.

Downloads can be prepared from an authenticated retained inbox/group journal or a typed local recovery record. A caller-constructed `Incoming` object is not sufficient. Recovery deletion/edit checks block using superseded retained descriptors for a new download. Media recovery restores static file keys and metadata without restoring live messaging/group keys or peer verification. The cache still checks current expiry, authenticates every chunk and requires complete-file commitment verification before plaintext use.

Five integration tests cover direct initial/session delivery, injected sender/receiver archive failures, exact retries, delayed-original deduplication, explicit lost-session recovery, same-server source enforcement, publication/cancellation handoff, group key/history atomicity and actual fresh-device media retrieval through HTTPS. General conversation deletion propagation, expiry cleanup of retained message/archive records, view-once/disappearing semantics, richer event types and platform viewers remain unfinished. Deleting a retained recovery descriptor blocks new handoffs; it does not yet erase every already-cached media copy or recall a key previously shared with another device.

## Canonical SigilText events (native schema 50)

Rich direct/group kinds 5/6 retain the existing 150/118-byte header and ordinary
retention mode 0. Their body is bounded canonical SigilText JSON, validated by the
same shared Rust model used before send. Typed mailbox variants are `SigilText`
and `GroupSigilText`; legacy text accessors reject them. Recovery content kind 3
preserves the exact canonical body. Direct/group history IDs use separate
`Sigil/direct-sigiltext-history/v0` and `Sigil/group-sigiltext-history/v0` domains.
Neither literal text containing JSON nor old history is silently reinterpreted.

`queue_peer_sigiltext`, `send_peer_sigiltext` and `queue_group_sigiltext` reuse
existing verification, session/claim selection, frozen delivery, Sender Keys,
recovery transactions and resend machinery. Archive tombstones block new rich
handoffs, including a pending intent before dispatch. Existing queued ciphertext
and recipient copies are not recalled by this increment. Incoming malformed rich
content is rejected before ratchet/inbox commitment.

See the [SigilText implementation contract](SigilText.md#backend-increment-canonical-inline-text-native-schema-50)
for supported syntax and open work. Complete event/compatibility capabilities
remain disabled. Structured actions, broader conversation controls and platform
rendering are not implied by this text-delivery increment.

Native schema 51 extends rich bodies to standalone canonical structured cards.
`queue_peer_card`/`queue_group_card` bind creator account, message ID and timestamp
to the authenticated direct/group event. Inline `rich()` and structured `card()`
accessors stay distinct; no new event/recovery kind or server schema is needed.
See [structured-card acceptance](SigilText.md#backend-increment-standalone-structured-cards-native-schema-51).

Native schema 52 also accepts canonical structured actions in rich kinds 5/6.
Their message ID hashes the exact action; actor and timestamp must match the
verified envelope. `Content::action()` exposes the typed body, while
`queue_peer_action` and `queue_group_action` use the existing encrypted delivery
paths. Card references include a digest of the original creation. Pending and
rejected semantic actions are journaled separately from malformed/forged
application frames. See [durable action acceptance](SigilText.md#backend-increment-durable-checklist-changes-and-ballots-native-schema-52).
