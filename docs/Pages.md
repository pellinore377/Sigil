# Pages

Status: core, server, crypto and client engine built (steps 1, 3, 4, 5); conversion, grants and UI not yet. Replaces Notes (SigilText §14 `note::`, §16 Notes tab, `NoteCard.kt`, `client/src/notes.rs`, the home Notes tab). Experimental like the rest of Sigil; no security claim before review.

A page is a living document that belongs to one conversation: prose mixed with checklists, tables, charts and other SigilText blocks, edited live by the conversation's members. The timeline carries a compact card for it; the conversation's Pages list finds it again. There is no app-level Notes or Pages tab.

## 1. Data model

### Page

| Field | Where | Rule |
| --- | --- | --- |
| `id` | grant, server | 32 random bytes chosen by the creator; unrelated to the conversation ID |
| `kind` | body meta | `page` or `list`; drives the timeline card and the list filter, never behaviour |
| `title` | body meta | Plain text, at most 200 scalars; last writer by server order wins |
| `body` | op log | One attributed-text delta (below); at most 1 MiB of plaintext JSON |
| `epoch` | grant, server | Key and capability generation, starts at 1 |
| `archived` | body meta | Moves the page to Past; reversible |

A page is a single body: no tabs, page settings, running heads, styles, citations, footnotes, comments or tracked changes. Comments and suggestions are an open question.

### Body

The body is an attributed-text delta: `retain`, `insert` (text or embed) and `delete` components over Unicode scalars, every paragraph ending in `\n` that carries the paragraph attributes, an embed counting as one scalar (U+FFFC in flattened text). Offsets cross to Compose as UTF-16.

| Attribute | Kind | Values |
| --- | --- | --- |
| `bold`, `italic`, `underline`, `strike`, `code` | run | bool |
| `link` | run | URL; validated as SigilText links are |
| `color`, `background` | run | A SigilText named colour (`red3`, …), resolved against the theme; never a literal hex |
| `effect` | run | One resolved SigilText effect descriptor (§8), so inline animation and spoilers survive |
| `mention` | run | Serialized mention (§9) |
| `heading` | paragraph | 1–3 |
| `list` | paragraph | `bullet`, `number`, `check` |
| `checked` | paragraph | bool, only with `list: check` |
| `indent` | paragraph | 0–4 |
| `quote` | paragraph | bool |

Embeds:

| Embed | Shape | Edited by |
| --- | --- | --- |
| Block | `{ "block": { "id", "source" } }`: one SigilText construct as source text (`chart::`, `diagram::`, `recipe::`, `math::block`, `art::`, a fenced code block) | `block_set` op replaces `source` whole; later by server order wins |
| Table | A `table` embed: `{ "id", "rows": [[cell delta]], "widths" }`, cells one paragraph | Cell deltas transform per cell; plus row and column ops |
| Card reference | `{ "card": { "author", "message", "digest" } }`: a live conversation card (poll, reminder, timer, task list) | Not editable in the page; its state stays in the structured-card system and renders with the card's own composable |
| Rule | `{ "hr": true }` | — |

Images are left out of the first version; a later embed points at an attachment descriptor sealed under the page key.

### Ops

Implemented in `pages/src/ops.rs`, over one body:

```
{ "delta": [ ...components ] }
{ "cell": { "table", "row", "col" }, "delta": [ ...components ] }
{ "table_row": { "table", "at", "action": "insert" | "delete" } }
{ "table_col": { "table", "at", "action": "insert" | "delete" } }
{ "block_set": { "id", "source" } }
{ "body_set": [ ...components ] }          // restore and import; content-carrying, wins outright
{ "meta": { "title"?, "kind"?, "archived"? } }
{ "reset": { "period" } }                   // recurring lists, phase 7; idempotent per period
```

Content wins: an op that carries content (`body_set`, a delta that re-inserts a table) voids what it meets, which becomes the empty delta, never a delta of retains. A batch applies whole or not at all. An op that cannot be opened or replayed is stepped over by every device at the same point. A delta is read as `push` builds it (zero-length parts dropped, neighbours merged, one key per part); a loaded snapshot must already be canonical and valid. A cell edit accepted as a no-op on a missing cell is voided when its length cannot meet a concurrent edit of that cell.

### SigilText mapping

Conversion both ways lives in Rust (`pages/src/convert.rs`) on top of `sigil-text`'s parser, so a page can be made from any message and sent back as one.

| SigilText | Page |
| --- | --- |
| Paragraphs, Markdown emphasis, code spans, links | Text with run attributes |
| `#`–`###` headings | `heading` 1–3 |
| Markdown bullet and numbered lists | `list: bullet` / `number` |
| `checklist::Title` (standard) | Title from the checklist; items as `list: check` paragraphs, `-x-` as `checked` |
| `checklist::weekly::…` (recurring) | Phase 7: `list: check` plus recurrence in meta; `-r-` items carry `persistent`; resets are `reset` ops. Until then it stays a card |
| `checklist::task::…`, `poll::`, `remind::`, `timer::`, `countdown::`, `ago::`, services, locations, contacts, randomizers | Card reference embed; the card itself stays in the timeline |
| `table::` | Table embed |
| `chart::`, `diagram::`, `recipe::`, `math::block`, `art::`, fenced code | Block embed holding the source |
| Inline modifiers (`red::`, `shake::`, `spoiler::`) | `color` / `effect` runs |
| `note::text;` | A page titled with the first line of `text` (§2) |

Page → SigilText ("Send as message", Copy) writes the inverse; a card reference becomes a reply link to its message.

## 2. Promotion

The sender's client decides, once, at compose time. A receiver never re-decides: the message on the wire is either an ordinary post or a page card. `sigil_text::page_shape(&Document) -> Option<Kind>` is the single rule, shared by the composer preview and send.

| Source | Result | Why |
| --- | --- | --- |
| `page::Title` on the first line; the rest of the message is the body, no terminator | Page, automatic | Explicit |
| `note::text;` | Page, automatic; title is the first line | Replaces notes |
| `checklist::Title` (standard) alone | List page, automatic | Lists are what people keep editing |
| A message of at least 3 paragraphs that opens with a heading, or of at least 1,200 scalars with any heading or list | Page, automatic, with a composer chip "Posts as a page" that turns it off before sending | Long, structured writing is a document |
| A composition with at least two data blocks (`table::`, `chart::`, `diagram::`, `recipe::`) and prose | Page, automatic, same chip | A report, not a remark |
| Recurring and task checklists, polls, reminders, timers, countdowns, dice, contacts, locations, services, utilities | Card | Moments with their own state, schedule or attribution rules |
| A single table, chart or diagram, short text, code, emoji, files, voice | Ordinary message | Long-press offers Make a page |
| View-once or expiring messages | Never a page | A page outlives its message |

Examples:

```
page::Trip plan
## Saturday
Train at 9:10, back by 18:00.
table::Who|Pays
- Ana|Tickets
- Ben|Lunch;
```
→ a page titled "Trip plan" with a heading, a paragraph and a table embed.

```
checklist::Groceries
- Milk
-x- Eggs;
```
→ a list page "Groceries", 1 of 2 checked.

```
poll::Where should we eat?
- Noodles
- Tacos;
```
→ stays a poll card.

### Make a page

The message menu's "Add to notes" becomes "Make a page" on text, SigilText and captioned file posts (not view-once or expiring). It converts the message, opens the new page for its title, and posts the page card as a reply to the source, so the reply quote keeps authorship and context. Promoted text is copied: editing the page never edits the message.

### In the timeline

- Creation posts one page card (§5).
- Later edits post at most one quiet notice per editor per editing session: `Ana edited Groceries`, one line, quiet ink, no bubble, sent silently (no push, no unread count). Consecutive notices for the same page within an hour collapse: `Ana and Ben edited Groceries`.
- A notice carries the new title, the two-line preview and list progress, so every member's card updates without fetching the page.

### Migration from notes

- `note::` parses to a page from the release that ships pages; the help panel and SigilText §14/§16 are rewritten.
- Existing `Construct::Note` cards and `Action::Note` promotions stay decodable (stored history and the canonical codecs reject unknown variants) and render read-only as today. The conversation's Pages list shows them in an "Earlier notes" section under Past; each row offers Make a page. Nothing converts automatically, because a page is a server object someone must own and pay for.
- `Action::Note` is no longer emitted. `notes.rs`, `NotesGrid`, the "Notes" timeline category and `mobile_is_note` go once the list (§5) reads pages.

## 3. Keys and access

### Grant

The creator's device generates, per page and epoch:

| Secret | Size | Use |
| --- | --- | --- |
| Page key `K` | 32 bytes | Root of every page subkey |
| Access token | 32 bytes | Read and append; the server stores its SHA-256 only |
| Manage token | 32 bytes | Rotate and delete; the server stores its SHA-256 only |

A grant is `{ host, id, epoch, key, access, manage? }`, at most 256 bytes, where `host` is the canonical server holding the log (the creator's home server). It travels only inside authenticated end-to-end conversation events, as attachment descriptors do, and enters the recovery archive with them.

Subkeys: HKDF-SHA-256 over `K`, salt `id ‖ epoch`, info `Sigil/page/<purpose>/v0` for `op`, `snapshot`, `presence` and `label`. Sealing is AES-256-GCM-SIV with a random 96-bit nonce. Associated data binds purpose, `id`, `epoch` and, for ops, `base` and `request`; for snapshots, `seq`. The server cannot move an op to another base or a snapshot to another sequence.

Each op batch's plaintext is `{ device, created, ops, signature }`: the author's device binding fingerprint and an XEdDSA signature over `Sigil/page-op/v0 ‖ id ‖ epoch ‖ base ‖ request ‖ SHA-256(ops)`. Whether a batch applies depends only on AEAD success, which every holder of `K` computes alike. The signature only attributes it ("Updated by Ana"); an unknown or failing signature shows "Unverified editor" and never changes the document, since devices may know different device sets and must not diverge.

### Distribution

| Case | How the grant arrives |
| --- | --- |
| Creation, 1:1 | Inside the page card post: Triple Ratchet to the peer, own-device copies to linked devices |
| Creation, group | Inside the page card post under the current Sender Key epoch |
| Note to Self | Inside the post on the account-derived conversation; own devices only |
| Linked device, new | The post and any later `Page::Key` operations arrive with conversation history (device history copies or the recovery archive), like any other operation |
| Group member joins | After the Add transition commits, the device that ordered it sends `Page::Key` for every active page in the new epoch. An authorized history grant (`SGHG`) also carries earlier cards |
| Member leaves or is removed | The administrator device that ordered the transition rotates every page of the group (below) and sends `Page::Key` in the new Sender Key epoch |
| Account recovery | Grants restore from the archive with the conversation; page content is then fetched from the host |
| Missing or stale key | The card shows "Waiting for access" and the device sends `Page::KeyRequest`; any member device holding the current grant answers with `Page::Key` to that requester only (1:1 sealed, groups by an authorized direct channel) |

Manage tokens: both participants in 1:1 and Note to Self; in groups the creator and administrators by the signed membership roles. An ordinary member cannot delete a shared page or lock others out.

### Rotation

1. The managing device pushes its pending ops, generates `K'` and new tokens, and seals a snapshot of the head under `K'`.
2. `POST /pages/{id}/rotate` with the expected epoch, the new token hashes and that snapshot. The server, in one transaction, advances the epoch, replaces both hashes and records the snapshot; old tokens fail from then on and no op may be based before it. Two rotations race by compare-and-swap; the loser adopts the winner's epoch when its `Page::Key` arrives.
3. `Page::Key` reaches the remaining members. A device whose append failed for the old epoch keeps its pending ops in plaintext locally, re-seals them under `K'` on arrival and sends them again.

Devices keep earlier epoch keys to read version history. A removed member keeps everything up to rotation; that cannot be recalled.

### Conversation operations

New `SGCO` actions (a native schema bump; older readers reject them, which is the codec's rule):

```
Page { page: Id, change: Edited { seq, title, preview, done, total } }
Page { page: Id, change: Key { grant } }
Page { page: Id, change: KeyRequest { epoch } }
Page { page: Id, change: Removed }
```

The page card itself is `Construct::Page { grant, title, preview, kind, done, total }` in `sigil-text`, posted like any card. Forwarding a page card never forwards the grant: Forward becomes "Send a copy", which creates a new page in the target conversation from the current snapshot.

### What the server sees

Page IDs, the owning account, token hashes, epoch, op and snapshot sizes and times, which local devices fetch which page, and presence timing. It never sees titles, bodies, cursors, keys or which conversation a page belongs to (timing correlation with conversation traffic remains). Federated readers reach the host through their home server's lookup proxy without account or device identifiers, as attachment chunks do; the access token stays mandatory.

## 4. Sync

### Server

`server/src/pages.rs` and `page_routes.rs`, an op log keyed by capabilities. Every route needs a live device credential; tokens travel in `sigil-page-access` / `sigil-page-manage` headers, never in URLs.

| Route | Token | Rule |
| --- | --- | --- |
| `POST /client/v0/pages` | — | Creates `{ id, access_hash, manage_hash, expires_at?, snapshot }` owned by the caller's account; the initial snapshot is sequence 0 |
| `GET /client/v0/pages/{id}` | access | `{ epoch, head, snapshot_seq, bytes, expires_at }` |
| `POST /client/v0/pages/{id}/ops` | access | `{ epoch, base, request, body }` → `{ seq, missed }`. `base` ≤ head and not below the newest snapshot (`base_before_snapshot`); wrong epoch is 409; a repeated `request` answers what it produced the first time |
| `GET /client/v0/pages/{id}/ops?after=&wait=` | access | Ops after `after`, at most 512; `wait` holds up to 25 s for a new op |
| `PUT /client/v0/pages/{id}/snapshot` | access | `{ epoch, seq, body }`; ignored with 204 when a newer one exists; refused when a later op has an older base |
| `GET /client/v0/pages/{id}/snapshot` | access | Latest |
| `GET /client/v0/pages/{id}/snapshots` | access | Versions without bodies; `GET …/snapshots/{version}` for one body |
| `POST /client/v0/pages/{id}/snapshots/{version}/label` | access | Sealed version name |
| `PUT` / `GET /client/v0/pages/{id}/presence` | access | Sealed `{ cursor, selection_end }`, kept 30 s, keyed by a random per-session slot |
| `POST /client/v0/pages/{id}/rotate` | manage | §3 |
| `DELETE /client/v0/pages/{id}` | manage | Deletes ops, snapshots and presence; releases quota; keeps a charged tombstone so the ID cannot return |

```
CREATE TABLE pages(id TEXT PRIMARY KEY, owner TEXT NOT NULL REFERENCES accounts(id), access_hash BLOB NOT NULL,
  manage_hash BLOB NOT NULL, epoch INTEGER NOT NULL, head INTEGER NOT NULL, bytes INTEGER NOT NULL,
  created INTEGER NOT NULL, updated INTEGER NOT NULL, expires_at INTEGER, deleted INTEGER NOT NULL DEFAULT 0);
CREATE TABLE page_ops(page TEXT NOT NULL REFERENCES pages(id), seq INTEGER NOT NULL, base INTEGER NOT NULL,
  epoch INTEGER NOT NULL, created INTEGER NOT NULL, body BLOB NOT NULL, request TEXT NOT NULL, PRIMARY KEY(page, seq));
CREATE UNIQUE INDEX page_ops_request ON page_ops(page, request);
CREATE TABLE page_snapshots(page TEXT NOT NULL REFERENCES pages(id), version INTEGER NOT NULL, seq INTEGER NOT NULL,
  epoch INTEGER NOT NULL, created INTEGER NOT NULL, body BLOB NOT NULL, label BLOB NOT NULL DEFAULT x'', PRIMARY KEY(page, version));
CREATE TABLE page_presence(page TEXT NOT NULL REFERENCES pages(id), slot BLOB NOT NULL, body BLOB NOT NULL,
  updated INTEGER NOT NULL, PRIMARY KEY(page, slot));
```

No author column: authorship is inside the signed ciphertext.

| Limit | Value |
| --- | --- |
| Sealed op batch | 64 KiB |
| Sealed snapshot | 3 MiB (2 MiB plaintext body, the replay limit) |
| Live pages per account | 4,096 |
| Ops page / long-poll | 512 / 25 s; waiters share a bound of 512 with the mailbox |
| Appends | Burst 40, refill 10/s per page and device |
| Reservations | 2 KiB per page, 256 bytes per op or snapshot row, plus ciphertext; charged to the owner's account quota through `storage_budget` |
| Versions kept | The newest 50 snapshots plus every labelled one |
| Compaction | Ops more than 30 days behind the newest snapshot |
| Expiry | `expires_at` from a disappearing conversation; bounded cleanup removes the page |

Quota exhaustion rejects appends and snapshots; it never deletes history. Account deletion deletes owned pages (see open questions on hand-off).

### Client

`client/src/pages.rs` (grants, local index, conversation operations), `client/src/page_sync.rs` (engine), tests beside them; crypto in `crypto/src/page.rs`; wire types in `protocol/src/pages.rs`.

The engine (`client/src/page_sync.rs`) keeps a server-ordered `base` at `head`, a `window` of effective batches since the snapshot, one `inflight` batch with its request name, `pending` ops composed while typing, a speculative `current`, and per-device undo and redo stacks carried over every later op.

```
page_sync(id):
  push: send inflight, or move pending into inflight with base = head and a fresh request name;
        on base_before_snapshot move it back to pending, pull, send again;
        apply `missed` through receive() before folding our own batch in
  pull: GET ops?after=head (wait only when nothing is pending); receive each:
        transform it over the window entries in (base, seq), apply to base,
        transform inflight and pending over it, carry undo/redo, move cursors
  every 300 ops with nothing pending: seal and PUT a snapshot, then trim the window
  return the view (paragraphs, UTF-16 offsets, patch since the last view)
```

| Concern | Rule |
| --- | --- |
| Threads | A long-poll never holds the store lock. An open page is an in-memory session; its network call runs outside the store and only `receive` and persistence take the lock. Android drives it from a coroutine per open page; the web from the browser transport's async fetch |
| Closed pages | No polling. Notices keep cards current; opening fetches the snapshot and ops after it |
| Offline edits | Pending and inflight batches persist sealed under the storage key, debounced to 2 s and flushed on close and backgrounding (one fsync per keystroke is too costly). Sent on reconnect under their original request names |
| Long offline | Within 30 days the ops behind a newer snapshot are still there and transform normally. Past compaction the local edits cannot be transformed: the device saves them as a new page "Title (offline copy)" and says so |
| Concurrent text | Character-level OT; at the same position the earlier batch in server order goes first |
| Size | An author stops at 1 MiB of body or cell text (`apply_local`); replay allows 2 MiB, so concurrent batches each under the limit apply in either order. A batch past the replay limit fails at the same point in server order on every device and is stepped over; its author drops its own copy there and rebuilds the speculative page |
| Concurrent formatting, `checked` | Earlier batch in server order wins the attribute it set |
| Concurrent title, kind, archived | Later batch in server order wins |
| Blocks | `block_set` is whole; later in server order wins |
| Tables | Cell deltas merge; a deleted row or column empties edits inside it |
| Restore | `body_set` from the chosen version, then a fresh snapshot |
| Presence | Every 5 s while focused, sealed; shown as avatars in the page header and carets in the text |
| Notices | After pushing, an idle 60 s or closing the page sends one `Page::Edited` silently |

Command surface for the UI (`mobile.rs` and the browser bridge): `page_create`, `page_make` (from a message), `page_open`, `page_view`, `page_edit`, `page_format`, `page_paragraph`, `page_insert`, `page_block_set`, `page_table_*`, `page_meta`, `page_undo`, `page_redo`, `page_sync`, `page_history`, `page_restore`, `page_copy`, `page_delete`, `page_close`, `presence_set`, `presence`, `pages_list`.

#### Engine API (built)

`sigil_client::page_sync::PageSession` is the open page; no method takes both the store and the network, so a long-poll can never hold the store.

| Call | Takes | Does |
| --- | --- | --- |
| `ClientStore::page_signer()` | store | `Signer`: identity key and server device id that sign batches |
| `PageSession::create(net, host, page, signer, expires_at)` | network | New page, snapshot 0 at epoch 1; returns the `Grant` (with manage token) and the session |
| `PageSession::open(net, grant, signer)` | network | Newest snapshot plus every op after it |
| `ClientStore::page_resume(grant, signer)` | store | The session as last saved, offline; `None` when nothing is kept |
| `edit`, `format`, `paragraph`, `insert`, `block_set`, `table_edit`, `table_line`, `meta`, `undo`, `redo` | memory | Local command; UTF-16 offsets; returns the view |
| `view()`, `restart(patch)` | memory | The view (`head`, `meta`, `body`, `view`, `since` when patched, `can_undo`, `can_redo`, `pending`) |
| `sync(net, wait)` | network | Push, then pull (long-poll only with nothing local), snapshot every 300 ops; returns `Synced { view, offline, changed, copy, stale_key }` |
| `flush(net)` | network | Push only; on close and backgrounding |
| `snapshot(net)` | network | Seals the server-ordered head; trims the window once stored |
| `presence_set(net, cursor, end)`, `presence(net)` | network | Sealed caret per random session slot; others' carets in UTF-16 |
| `add_grant(grant)` | memory | Key of another epoch; a newer one replaces the access token |
| `summary()` | memory | Title, two-line preview, kind, checked and total for `Page::Edited` |
| `ClientStore::page_save(session)`, `page_forget(id)` | store | Persist or drop the local copy; the caller debounces saves (2 s) and saves on close |

- Persistence (client schema 87): `page_cache` holds the base page, head, snapshot sequence and window in 1 MiB sealed parts; `page_work` holds the inflight batch (sealed bytes and request name), pending ops and the batches received since the cache. The cache is rewritten after 64 received batches or 256 KiB of them.
- A batch is sealed once and resent byte for byte under its request name. A reply that went missing is recovered either by the retry or by finding the request name in the log, which folds it as our own.
- `copy` in `Synced` is local work that could not cross compaction, already titled "Title (offline copy)"; the caller creates it with `create` and posts its card.
- `wrong_epoch` or a revoked access token keeps the work and sets `stale_key`, and sync stops pulling until a newer grant arrives; after `add_grant` a batch sealed under the older epoch is looked for in the log and resealed only if absent. An op under an epoch the device holds no key for stops the log with `NeedsKey` instead of being stepped over.
- 429 answers leave the work in place for the next sync.

## 5. Interface

Tokens and components from [Design.md](Design.md); cards follow the SigilText card direction (the bubble is the card, no type word, ink and ground from the bubble).

### Timeline card

- Title in titleMedium, at most 2 lines; a two-line preview in bodyMedium at quiet ink (68% ink); the foot "Updated 2h ago by Ana" in labelMedium quiet ink. The leading glyph (`article`, or `checklist` for a list) is kept because it says the card opens into something.
- List pages show progress ("3 of 7") and the first three unchecked items as 48dp check rows that toggle straight from the timeline (a `delta` op setting `checked`, sent without opening the page), then "Open list".
- The whole card is one button: "Page, Trip plan, updated 2 hours ago by Ana. Open." Reply quotes show the glyph and title.
- "Waiting for access" and "Page removed" are quiet one-line states inside the same card.

### Opening a page

- **Phone**: a full-screen destination through the presentation host, sliding in like any page. Glass header: back, the title (tap to rename), editors' avatars, History, and an overflow with Send a copy, Send as message, Archive, Details and Delete (manage token only). The body is one column at message type (18/26), headings at titleLarge/titleMedium/titleSmall. The Format panel's compact toolbar rides above the keyboard with bold, italic, heading, bullet, check, indent and Insert (table, chart, diagram, code, math, card from this conversation).
- **Wide (≥ 1000dp, `LocalWideLayout`)**: the conversation surface splits; the chat keeps 400–560dp and stays live with its composer, and the page takes the rest as its own 28dp surface with the same glass header plus Close. When the navigation pane, chat and page do not fit, the pane collapses to its 72dp rail. Escape closes the panel; the chat keeps its draft and scroll.
- Embeds render with the existing composables (`TableCard`, `ChartCard`, `DiagramCard`, `MathCard`, `CodeBlock`, `RecipeCard`, card references with their own card). Selecting one gives it the accent edge with Edit (opens its builder or source) and Delete.
- History lists versions with who saved them and when, named versions first; Restore asks first.

### Pages list

- Reached from the conversation menu (Pages replaces Notes) and from the conversation details, which show the count. Note to Self has its own, as every conversation does.
- Chips: All, Pages, Lists, Reminders. Sections Active and Past (Past collapsed); Past holds archived pages, completed lists, fired reminders and Earlier notes. Reminders are the existing reminder cards, derived from the timeline as the Notes tab derives them now.
- Rows: glyph, title, one status line (`Updated 2h ago by Ana`, `3 of 7`, `Tomorrow · 9:30 AM`), 48dp minimum. A search field filters titles and previews from the local index. New page is the primary key at the top.

### Removing the Notes tab

The main navigation becomes Messages, Calls, Settings (`Navigation.kt` tab list, `NavigationPane.kt` both rails, `NotesGrid` and `NotesHeader` in `Inbox.kt`, the `"notes"` destination). The composer's `notes` mode, which wraps the draft in `note::`, goes with it. Design.md's main navigation line and "Inbox, notes and settings" section change with the code.

### Accessibility

- Headings carry heading semantics; check paragraphs are toggleable with the Checkbox role and read "checked" or "not checked"; embeds read with their card's own description.
- Every hover affordance (embed chips, version rows) has a tap and long-press route; every shortcut (Ctrl+B, Ctrl+Z, Escape) has a visible control.
- Remote edits are not announced per keystroke; a polite announcement says "Ana is editing" once when presence starts.
- Quiet ink stays at 4.5:1 on all four grounds; carets and selection of other editors use ink ramps with their initials, never colour alone. Reduced motion drops caret and panel motion.

## 6. Plan

Each step is one agent's work with its own tests; later steps depend on earlier ones only where noted.

| Step | Scope | Files | Tests |
| --- | --- | --- | --- |
| 1 | Core crate: copy and trim the delta core, page ops, apply, invert, transform, view | `pages/src/{delta,ops,view,page}.rs`, workspace member | Ported unit tests; a randomized convergence test (two to four sites, random concurrent batches, server order, all converge); a `fuzz/` target |
| 2 | SigilText conversion and promotion rule | `pages/src/convert.rs`, `text/src/` `page_shape`, `page::` parse, `note::` alias, help | Round trips for every construct; promotion table as fixtures |
| 3 | Server store and routes, quotas, compaction, long-poll, presence, expiry, deletion | `server/src/pages.rs`, `page_routes.rs`, `page_tests.rs`, schema bump | Idempotent append, base-before-snapshot, orphan refusal, rotation CAS, token revocation, quota, compaction, restore behaviour |
| 4 | Crypto and wire types | `crypto/src/page.rs`, `protocol/src/pages.rs`, `SGCO` actions, `Construct::Page` | Independent vectors for HKDF and AEAD with associated data; tamper and wrong-epoch failures |
| 5 | Client engine and persistence | `client/src/page_sync.rs`, `pages.rs`, schema bump | Engine against an in-process server: concurrent edits, lost replies, offline then reconnect, compaction fallback to an offline copy, undo across remote ops |
| 6 | Grants in conversations: create, make, key, request, rotate on removal, join, recovery | `client/src/pages.rs`, group work hooks | 1:1, group add and remove, linked device, archive restore, key request |
| 7 | UI: card, notice row, list, phone editor, wide panel, Make a page, Notes tab removal | `PageCard.kt`, `PageEditor.kt`, `PagePanel.kt`, `PagesList.kt`, `Timeline.kt`, `Navigation.kt`, `Inbox.kt` | Desktop tests for card semantics, list filters and sections, panel layout at 1000/1400dp, menu entries; seeds renders in light and dark, incoming and outgoing, narrow |
| 8 | Migration and removal of notes code; docs | `notes.rs`, `mobile_views.rs`, `NoteCard.kt`, SigilText §14/§16, Design.md, Status.md | Legacy note decoding and the Earlier notes section |
| 9 | Federation access through the lookup proxy | `federation_lookup.rs`, client routing by `host` | Foreign reader and writer without identifiers in the envelope |
| 10 | Recurring lists as pages; card references | `pages/src/ops.rs` `reset`, `structured_recurring.rs` reuse | Reset idempotence across devices and time zones |

Riskiest parts:

- **Transform correctness.** One missing mirror arm diverges documents silently. The convergence test and fuzzing come before any UI; the content-carrying cases are where it breaks.
- **Store lock and long-polls.** A page sync holding the client store blocks messaging; the session split must land with step 5, not after.
- **Rotation races** with members sending under the old epoch, and administrators offline when a removal commits.
- **Groups and the server.** The host cannot check group membership (private groups), so capability tokens are the only gate; a removed member keeps write access until rotation.
- **Editor on the web and Android IME.** Fragmented text fields, composition and caret mapping are the hardest part; use a fragment model rather than one big field.
- **Compatibility.** New `SGCO` actions and `Construct::Page` are rejected by older clients by design; ship them behind one schema bump with the server.

## Open questions

1. Long structured messages: promote automatically with the chip to opt out (proposed), or only suggest?
2. Should group members who join later get existing pages (proposed: every active page), none, or the administrator's choice?
3. In groups, may only administrators and the creator rotate and delete (proposed), or every member?
4. Task checklists: stay cards (proposed) or become pages that lose single-completer undo? Recurring lists in step 10 or never?
5. When the creator leaves a group or deletes the account, should a remaining member's device copy the page to its own server automatically, or does the page go?
6. In disappearing-message conversations: pages expire with the conversation's timer (proposed) or are disabled?
7. Edit notices silent with no unread count (proposed), or notifying?
8. Comments and suggested edits in pages: later, or never?
9. Federated members poll their home server's proxy while a page is open (seconds of latency): acceptable for the first version?
10. Existing notes: an Earlier notes section with per-row Make a page (proposed), or convert Note to Self notes automatically?
11. Timeline checkboxes on list cards: toggle in place (proposed), or open the list first?
12. Image embeds in the first version?
