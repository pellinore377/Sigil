# Application event contract

`protocol/src/event.rs` defines canonical `SGEV 00 01 00 00` frames. Integers are big endian; unknown versions/kinds/reserved flags and trailing bytes fail. Existing text/file/SigilText codecs remain readable for stored history and compatibility.

| Kind | Content | Header |
| --- | --- | --- |
| 1 / 2 | Direct / group text | 150 / 118 bytes |
| 3 / 4 | Direct / group file | 150 / 118 bytes |
| 5 / 6 | Direct / group SigilText | 150 / 118 bytes |
| 7 / 8 | Direct / group conversation operation | 150 / 118 bytes |

Headers bind transport message ID, conversation/group, sender fingerprint, direct recipient fingerprint, positive sender timestamp and exact body length. Direct bodies are capped at 65,224 bytes. Sender timestamps are presentation metadata. Expiry decisions use trusted local time with a persisted rollback floor.

Account references hash `Sigil/account-reference/v0`, u16 canonical server-name length, server bytes and stable account ID. Direct conversation references hash `Sigil/direct-conversation/v0` and sorted account references. Device fingerprints bind exact endpoints; federation preserves these checks. Validate content and authorization before committing ratchets, prekey consumption, history or acknowledgements. Authorized lost-session resends preserve application content and follow at most three signed request hops.

`protocol/src/conversation.rs` defines `SGCO 00 01` followed by canonical JSON. Unknown fields, noncanonical encodings and oversized operations fail. Logical operation IDs remain stable across device fan-out; each author/device counter identifies one operation. Lookup indexes are keyed and content is sealed locally. Private operations and chunked snapshot synchronization require a verified same-account channel; they are rejected in groups.

Posts carry text, canonical file metadata or SigilText, optional reply/thread references, expiry and view-once flags. References include author account and message ID. Edits require the original author; structured cards use their existing authorized actions. Author deletion wins over every edit. Pins and each account's single reaction resolve by counter, device fingerprint and operation ID. Delivery/read receipts accumulate by account. Delivery receipts are durably queued on receipt; read receipts and typing start enabled, presence disabled. Typing/presence are contact-limited and expire within 30/120 seconds of local receipt.

Captionless files retain `SGFC 00 01 00 00`. Captioned files use `SGFC 00 02 00 00`, the same file header/metadata, then a u16 UTF-8 caption length and 1–8192 caption bytes. Both remain inside one authenticated encrypted post. Receivers must support v2; captions are never silently stripped for older clients. File-cache identity excludes the caption but still compares source, name, MIME, expiry, access credential and complete key descriptor; message/recovery authentication retains the caption. Native schema 77 prevents older clients from opening captioned history or deferred upload drafts.

Private pins, unread/snooze/hidden settings, optional collections and drafts synchronize across linked devices. Settings resolve deterministically; draft versions retain concurrent alternatives until an explicit observed-version context resolves them. Note to Self has a separate account-derived conversation. Timeline/thread and local conversation/global search APIs scan bounded pages without a plaintext search index.

Application reads use conversation projections and guarded file APIs. Deleted/expired bodies are hidden, obsolete queued sends are cancelled, and view-once consumption commits before plaintext is returned. Consumption markers synchronize; offline devices do not share an atomic global lock. Expiring/view-once posts and their edits are excluded from device-history copies and recovery. Physical retention cleanup and key erasure belong to roadmap items 5 and 12. Previously disclosed plaintext cannot be recalled.

Recovery uses canonical `SGCS` snapshots: compact binary posts preserve maximum legacy bodies without JSON expansion; other operations retain canonical `SGCO`. Restore derives conversation state without restoring live encryption keys. Device synchronization fragments snapshots, authenticates complete assembly and rejects conflicting replays; stale worker completions cannot rewind progress. Authorized earlier group history retains supplying-device provenance instead of claiming direct original-sender authentication.

Files use bounded `SGFC` metadata and an `SGAD` key descriptor. Source, access capability and descriptor must match retained publication/download state; filenames/MIME never authorize filesystem paths or parsers. [SigilText](SigilText.md#canonical-backend-representation) creators/actors and origin IDs/timestamps must match authenticated events. Emoji animation is a [client presentation rule](Design.md), with Unicode preserved as text.
