# Porting the whole QML UI to Slint, for Sigil

This is the plan of action for rebuilding every part of the QML user
interface under `omarchy/` in Slint, on top of the Sigil backend on the
`encrypt` branch. It exists so that whoever does the work, person or Claude
session, has one definition of done and one list of what to build.

"1:1" here means two things and only two things:

1. **Same look.** Every page, sheet, bubble, control and animation renders
   the way the QML one does: same sizes, spacing, radii, colours, fonts,
   grouping rules and motion. The design tokens are the same fourteen
   colours and six type sizes (`docs/portability.md`), so this is mostly a
   matter of copying numbers faithfully and reproducing the few derivations
   that generate the rest.
2. **Same features.** Everything a person can do in the QML app can be done
   in the Slint app, except things that only exist because of Matrix. Those
   are replaced by their Sigil equivalents, listed in part 3, or dropped
   where Sigil deliberately has no equivalent, also listed.

It does not mean the same files or the same code. QML has JavaScript inside
the UI; Slint has none. Anything the QML computed inline moves to the Rust
bridge. Where Slint cannot express a QML effect, this document says what to
do instead, so nobody has to improvise.

The `slint` branch already holds a first attempt (39 `.slint` files, an
in-process engine bridge, tokens, icons, fonts, and on-device fixes). It is
Matrix-shaped and incomplete, but its foundations are sound and are reused
in phase 0 rather than rewritten. Everything else is measured against the
QML, not against that branch.

---

## Part 1. Inventory: 77 QML files, 19,250 lines

Every file, what it is, and what it becomes. **Target** is the Slint file,
mirroring the QML path under `slint/ui/` so the two trees can be audited
side by side. **Sigil change** says what differs from the QML because the
backend is Sigil. **Now** is the state on the `slint` branch (from its own
QA and wiring notes, not verified on screen).

### 1.1 Shell and bridge (`omarchy/*.qml`)

| QML | Lines | What it is | Target | Sigil change | Now |
|---|---|---|---|---|---|
| `Service.qml` | 1098 | The engine bridge: socket, request/reply, every event, all state, timeline models, grouping rules, side caches | `slint/src/bridge.rs` + `models.rs` (Rust, not Slint) | In-process engine, no socket. Drops Matrix-only state (spaces, presence, power levels). Keeps timeline semantics exactly: newest at index 0, `decorate()` whitelist, diff ops, 5-minute/same-day grouping, session stamps, drift detection | bridge exists (1,395 lines), Matrix-shaped |
| `Panel.qml` | 1773 | The card, page stack, slide-in holders, `goBack()` unwind ladder, call minimise/maximise transform, drafts, theme persistence | `app.slint` + `slint/src/nav.rs` | A normal window instead of a Wayland layer panel. Same 400×820 card proportions on desktop, full screen on phone. The 90-function `sigilui` test IPC becomes a Rust test harness (part 5) | app.slint exists; not all pages mounted |
| `BarWidget.qml` | 94 | Omarchy bar icon with unread and call badges | none in Slint; a 30-line omarchy plugin that toggles the Slint window (phase 8) | — | not needed |
| `FrostPopup.qml` | 106 | Real xdg-popup so Hyprland can blur it | `furniture.slint` popup with an opaque tinted surface | Slint cannot blur what is behind it; use an opaque surface at popup-background | done (opaque) |
| `TextContextMenu.qml` | 208 | Themed Cut/Copy/Paste/Select-all menu for text fields | `furniture.slint` `TextMenu` | Same rows, same icons | missing |

### 1.2 Components (`omarchy/components/`)

| QML | Lines | What it is | Target | Sigil change | Now |
|---|---|---|---|---|---|
| `Avatar.qml` | 98 | Round or square avatar, initials on a hashed hue, presence dot | `components.slint` `Avatar` | **No presence dot** (Sigil has no presence, part 3). Initials from `@name:server` the same way | exists with presence; remove dot |
| `ChoiceSheet.qml` | 71 | Bottom sheet, one question, radio answers | `sheet.slint` `ChoiceSheet` | none | exists |
| `Dialogs.qml` | 118 | Modal host: dm / join / create / invite / leave | `furniture.slint` `Dialogs` | `join` by `#alias` is gone. `create` loses the Private/Encrypted toggles: every Sigil conversation is private and encrypted | partial |
| `EmojiPicker.qml` | 115 | Search, 44 px grid, category strip | `emoji.slint` | Emoji table bundled in `shared/` instead of borrowed from another omarchy plugin | exists |
| `Fonts.qml` | 38 | Bundled Roboto, Roboto Mono, Material Symbols (outlined and filled) | `style.slint` font imports | none | done |
| `Icons.qml` | 139 | Generated from `shared/icons.json` | `icons.slint`, third emitter of `shared/icongen` | none | done |
| `IconLabel.qml` | 47 | One glyph in a known box, filled or outlined family, ink-fit option | `components.slint` `IconLabel` | none | exists |
| `ImageViewer.qml` | 701 | see 1.5 | | | |
| `LoginPage.qml` | 121 | SSO sign-in | `login.slint` **rewritten** as the Sigil welcome flow (part 3.1) | Replaced by three screens: Create account, Recover, Link this device | Matrix SSO shape; rewrite |
| `OverflowMenu.qml` | 77 | The ⋮ drop with its own scrim | `furniture.slint` `OverflowMenu` | none | exists |
| `PageHolder.qml` | 48 | Slide-in page layer, opaque ground, event sink | `furniture.slint` `PageHolder` | Slint animates on a clock, not on frames. Use a 220 ms ease-out and **build the page before sliding** so a stall cannot eat the animation (the QML went frame-driven for that reason) | exists |
| `RecoveryPage.qml` | 76 | Enter the Element security key | `recovery.slint` **rewritten** (part 3.1) | Becomes: show the recovery code once; set or change the backup password | Matrix shape; rewrite |
| `ScrollBarStyle.qml` | 30 | 4 px bar, hidden unless in use | `components.slint` scrollbar styling | Slint's ListView bar; style to 4 px rounded | exists |
| `SettingsGroup/Header/Row.qml` | 250 | The settings design system: back + title + one action; titled band; row with icon at 22, text at 56, trailing at 20, seven trailing kinds | `components.slint` `SettingsHeader`, `SettingsGroup`, `SettingsRow` | none; the three numbers are the point | exists |
| `Spinner.qml` | 12 | Rotating glyph, 900 ms | `components.slint` | none | exists |
| `TipLayer.qml` | 55 | In-card hover tips | `furniture.slint` `TipLayer` | Desktop only; hover reveals, never gates | missing |

### 1.3 Home, lists and settings (`omarchy/mobile/`)

| QML | Lines | What it is | Target | Sigil change | Now |
|---|---|---|---|---|---|
| `HomePage.qml` | 473 | Header, search pill, Chats/Spaces segmented control, chat rows with kind-icon previews, badges, FAB, scroll-to-top, in-card account menu | `home.slint` | Segmented control becomes **Chats / Requests**. Requests are the Sigil request screen: strangers who wrote to you, with accept and decline. Sort rule kept: favourite, highlights, unread, last activity. No presence dots. Lock badge always on, so it is dropped as noise. Call badge stays | exists (Chats/Spaces); rework |
| `StartChatPage.qml` | 150 | Search people, quick actions, DM suggestions | `pages/start.slint` | Search is an **exact username lookup** (`users.search` returns one result or none). Quick actions: New group, and nothing else. No join-by-address, no spaces | exists; rework |
| `SearchPage.qml` | 138 | Client-side search of the loaded timeline; images and links when idle | `pages/search.slint` | none | exists |
| `ForwardPage.qml` | 94 | Pick a chat, resend | `pages/forward.slint` | Filter out requests instead of spaces and invites | exists |
| `RoomSettingsPage.qml` | 305 | Avatar, name, subtitle, topic, Add people, Pin, Low priority, members with Admin/Mod tags, settings groups, Spaces section, Leave | `pages/roomsettings.slint` | Admin tag comes from the **policy's admins list**, not power levels. No Spaces section. Notifications row stays (local, part 3.4). Security row becomes a **Privacy** page (3.4). Leave stays; a leave is a real MLS removal now | exists; rework |
| `MembersPage.qml` | 169 | Member rows, role tag, invite button | `pages/members.slint` | Tag = Admin from policy. No name disambiguation (usernames are unique) | exists |
| `AddPeoplePage.qml` | 75 | Debounced people search, invite on tap | `pages/addpeople.slint` | Exact username lookup; invite = `room.invite` (MLS add) | exists |
| `NotificationsPage.qml` | 132 | Per-room mode with the "custom setting" toggle and the default band | `pages/notifications.slint` | **Local** per-conversation mode (all / mentions / mute) stored on the device; the engine gains `room.setNotify` (part 4). The E2EE caveat text goes away: matching is always on-device in Sigil | exists; rework |
| `SecurityPage.qml` | 195 | Join rule, encryption toggle, history visibility | `pages/privacy.slint` **new content** | Becomes an information page: "End-to-end encrypted, always"; members and their devices; conversation epoch and last key rotation; Leave and forget. No toggles, nothing to save | exists as Matrix page; replace |
| `RolesPage.qml` | 265 | Admins / Moderators counts, change my role, reset permissions | `pages/admins.slint` | Becomes **Admins**: list admins, make admin, remove admin (policy change, admins only). No moderators, no reset | exists as Matrix page; replace |
| `PermissionsPage.qml` | 159 | Per-key power levels | none | Dropped: Sigil groups have admins and members, nothing finer. Part 6 lists it as a deliberate omission | delete |
| `ChatThemePage.qml` | 408 | Live mini preview, 7 swatches plus custom HSV wheel, 3×3 gradients, photo wallpaper | `pages/chattheme.slint` | none; purely local. The hover-only swatch affordances get a tap twin | exists; verify wheel |
| `ThreadsPage.qml` | 242 | Thread list with theme-matched chrome | `pages/threads.slint` | Kept **if** threads are adopted (decision D4 in part 7). Backed by a Sigil event reference, part 4 | exists; needs engine |
| `PinsPage.qml` | 324 | Pinned messages as mini bubbles with the exact bubble fill | `pages/pins.slint` | Pins become a policy field (part 4) | exists; needs engine |
| `SpacePage.qml` | 443 | A space's hierarchy | none | Dropped with spaces (decision D1) | delete |
| `SpaceRoomsPage.qml` | 268 | Add or remove space children | none | Dropped | delete |
| `SpaceSettingsPage.qml` | 229 | Space identity and groups | none | Dropped | delete |
| `NewSpacePage.qml` | 204 | Create a space | none | Dropped. `room.create` with a name and invitees is the group flow, reached from Start chat | delete |

### 1.4 Chat (`omarchy/mobile/`)

| QML | Lines | What it is | Target | Sigil change | Now |
|---|---|---|---|---|---|
| `ChatPage.qml` | 2047 | Header, timeline, typing row, composer, autocomplete, wheel chase model, pin-to-latest, pagination, jump-to-event, keyboard shortcuts, toast, invite banner | `chat.slint` + `slint/src/chat.rs` | Header subtitle: typing text, else "N members", else nothing (no presence). `#room` autocomplete gone; `@` autocomplete over members stays; `@::` contacts stays if contacts are adopted (D6). Invite banner becomes the **request banner** with Accept and Decline | exists; large gaps (see QA) |
| `BubbleDelegate.qml` | 1477 | One row: day chip, unread divider, state caption, sender header, bubble with group corners and fills, reactions straddling the top corner, pin marker, detail row with reader avatars, reply quote, thread chip, entry animation, jump flash | `bubble.slint` | `can.*` comes from the item as before; the engine fills it from ownership and admin status. Read receipts and reactions unchanged. Pin marker and thread chip depend on D4/D5 | exists; spacing wrong, several bodies missing |
| `RichText.qml` | 427 | Per-glyph SigilText renderer: shake, wave, pulse, glow, typewriter, sparkle, glitch, blur, flip, barrel, spoiler, mark, rainbow and gradient colour, size scale | `richtext.slint` + `slint/src/glyphs.rs` | none. Rust lays the text into glyph runs; Slint draws one `Text` per glyph for animated runs and one `Text` per word otherwise, exactly the QML split | "not drawn"; build |
| `CodeBlock.qml` | 94 | Fenced code as a picture inside the bubble, pre-highlighted by the engine, `#242428` ground, language badge | `bubble.slint` `CodeBlock` | none | plain text; build |
| `MessageSheet.qml` | 418 | Long-press sheet: frosted page, lifted bubble copy, reaction pill, action menu, delete confirm, emoji drawer | `sheet.slint` | Actions: reply, forward, copy, edit, delete, pin, thread (D4/D5), end poll (D3). The frost is a **pre-blurred snapshot** rendered in Rust (part 5.3) | exists; deep-dim scrim instead of blur |
| `AttachMenu.qml` | 444 | Grid of Files, Emojis, Stickers, Poll, Current location, Live location, Drop a pin; the poll builder; sticker flow | `attach.slint` | Tiles present for what is adopted: Files, Emojis always; Poll (D3), Stickers (D7), the three location tiles (D8) | exists, not mounted |
| `TypingIndicator.qml` | 71 | Avatar plus three bouncing dots, height animates 0↔38 | `chat.slint` `Typing` | none | exists |
| `PollBody.qml` | 201 | Poll card with animated shares, retract, multi-select | `bodies.slint` `PollBody` | D3 | exists; needs engine |
| `AudioBody.qml` | 154 | Music-file card with cover art and engine-picked accent | `bodies.slint` `AudioBody` | none; `audio.info` is engine work (part 4) | missing |
| `ContactBody.qml` | 241 | Shared contact card, stacked vcards, Message/Save/Share | `bodies.slint` `ContactBody` | D6 | missing |
| `LocationBody.qml` | 183 | Map card, live countdown, ended desaturation | `bodies.slint` `LocationBody` | D8; without a map renderer it is the pin-on-tone fallback the QML already has | pin fallback |
| `DocThumb.qml` | 183 | Document glimpse drawn as a page: prose, sheet grid, or image | `bodies.slint` `DocThumb` | `doc.thumb` re-wired in the engine (part 4) | missing |
| `VoiceRecorder.qml` | 221 | Idle, recording, ready; level bars from `voice.level`; restart, stop, attach | `voicerec.slint` | none; engine has `voice.*` | exists |

### 1.5 Media pages and viewer

| QML | Lines | What it is | Target | Sigil change | Now |
|---|---|---|---|---|---|
| `components/ImageViewer.qml` | 701 | Morph out of the thumbnail, blurred backdrop, pager, pinch and double-tap zoom, passive-grab pan, quick reactions, download, delete, video scrubber | `viewer.slint` + `slint/src/zoom.rs` | none. Zoom maths (`zoomAbout`, `clampOffsets`) ports verbatim to Rust. Video frames arrive through the frame channel (part 5.4) | exists, not mounted; pinch missing |
| `mobile/DocumentPage.qml` | 502 | Markdown, sheet grid, PDF pages, text blocks; paper tone; shelf with Download | `pages/doc.slint` | `doc.preview` and `doc.page` re-wired (part 4) | stub replaced; needs engine |
| `mobile/AudioPage.qml` | 342 | Full-bleed stage with accent gradient, cover, scrubber, play/pause | `pages/audio.slint` | none; playback clock in the bridge | stub replaced |

### 1.6 Calls

| QML | Lines | What it is | Target | Sigil change | Now |
|---|---|---|---|---|---|
| `mobile/CallPage.qml` | 576 | Voice, 1:1 and group layouts; featured track rule; controls; reaction tray and floaters; draggable self PiP with spring physics; settings sheet | `callpage.slint` + `slint/src/call/` | The whole media stack moves into the app in Rust (part 5.5). `call.state` keeps the **same shape** the QML reads so this page ports faithfully | shell only |
| `mobile/CallGrid.qml` | 161 | Two-column grid, spotlight for shares, thumb strip | `callpage.slint` `CallGrid` | none | shell |
| `mobile/CallPiP.qml` | 203 | Minimised call card with throw physics | `callpip.slint` | none | shell |
| `calls/CallBanner.qml` | 50 | Incoming call banner, outlives the panel | `callbanner.slint` | Rings from the kind-10 call event; Accept joins, Decline sends nothing (there is no decline event; D9) | shell |
| `calls/CallPill.qml` | 47 | Persistent pill while the panel is hidden | none on phone; desktop tray later | — | not needed |
| `calls/CallBar.qml` | 26 | "Return / Hang up" strip when the call is in another room | `chat.slint` `CallBar` | none | missing |
| `calls/CallControls.qml`, `DevicePicker.qml`, `CallView.qml`, `ParticipantGrid.qml` | 176 | Wide-window call column | `callpage.slint` wide layout | Desktop only | missing |
| `calls/ParticipantTile.qml` | 98 | Tile with the `trackLive` rule, speaking border, name pill, quality glyph, mirror | `callpage.slint` `CallTile` | `trackLive` rule kept: a muted camera keeps its track | shell |
| `calls/SpeakingRipple.qml` | 56 | Two rings, amplitude from level | `callpage.slint` | none | shell |
| `calls/VideoTile.qml` | 13 | The native shm surface | Slint `Image` fed from Rust | Frame channel, part 5.4 | missing |
| `video/` (C++ plugin) | | shm reader and texture upload | `slint/src/frames.rs` | Rust reader of the same `omv_shm.h` layout when out of process; direct handoff in process | missing |

### 1.7 Maps

| QML | Lines | What it is | Target | Sigil change | Now |
|---|---|---|---|---|---|
| `mobile/MapView.qml` | 438 | MapLibre view with hand-rolled passive-grab pan, pinch, wheel, markers, live waves | `pages/map.slint` | Slint has no map renderer. Two options in D8: raster tiles drawn in Rust into an `Image`, or the pin fallback only | pin fallback |
| `mobile/MapPage.qml` | 498 | Full-page shared location, countdown chip, Stop or Open | `pages/map.slint` | D8 | fallback |
| `mobile/LocationPicker.qml` | 325 | Current, live, drop-a-pin modes | `pages/map.slint` `LocationPicker` | D8 | fallback |

---

## Part 2. What the Slint app is

- **One Rust crate, `slint/`**, linking `sigil-engine` in process. The JSON
  contract in `core/docs/protocol.md` is the boundary; `bridge.rs` replaces
  the socket, not the protocol. This is already how the `slint` branch is
  built and it is the right shape: the engine is the one piece that never
  needs porting.
- **Rust does the thinking, Slint does the drawing.** Every `function` and
  every `property: expr` with logic in the QML becomes Rust in the bridge,
  which hands Slint flat structs and pre-computed strings. Slint expressions
  cannot scan arrays, format dates, or derive colours from strings.
- **A window, not a panel.** On the desktop the app is a normal window
  sized like the card (400×820 logical, resizable); on Android it fills the
  screen. The Omarchy bar widget, layer-shell panel, and `FrostPopup` are
  the shell's business and are not ported. A tiny omarchy plugin that shows
  and hides the window comes last.
- **Tokens.** `style.slint` `Theme` carries the same names as `qs.Commons`:
  `accent background foreground muted urgent`, `menu-*`, `popup-*`, the six
  type sizes, `alpha()`. `Style.space(n)` is `n px`, since Slint scales
  logical pixels itself. Sigil owns its theme now: ship the Frosted Glass
  values as the default and let the user pick an accent in Settings.
- **Icons and fonts** stay generated and bundled as they are.
- **Navigation.** A page stack in Rust (`nav.rs`) mirroring Panel's `nav`
  states and the single `goBack()` ladder: viewer, dialogs, sheet confirm,
  sheet drawer, account menu, call tray, call settings, minimise call, chat
  menu, page stack, close. One `PageHolder` per page in `app.slint`, each
  with an opaque ground and an event sink.

---

## Part 3. Matrix to Sigil: what every page becomes

### 3.1 Getting in

The QML has SSO login and a security-key page. Sigil has three doors, all
already in the engine:

| Screen | Requests | Notes |
|---|---|---|
| **Welcome** | `account.status` | App glyph, "Sigil", three buttons: Create account, I have an account (recover), Link this device. Same layout as `LoginPage` with the SSO button row replaced |
| **Create account** | `account.create{username, invite, password?, envoy?}` | Username as `@name:server`, invite code from the server admin, optional password with the explanation "lets you recover everything with your password and a printed code". Advanced: Envoy address |
| **Recover** | `account.recover{username, password, code}` | Username, password, the printed code. Error copy for a wrong secret and for the server's back-off |
| **Link this device** | `link.offer` then `link.state` events | Shows a QR (the offer string) and the seven emoji when they arrive; `done` opens Home. On the **existing** device: Settings → Link a device → `link.scan{offer}` (camera on phone, paste on desktop) → seven emoji → `link.confirm{ok}` |
| **Server first** | `account.probe{server}` (new) | The first screen asks only for the server, then reads its card: open registration, invite codes, SSO, TPM. The next screen shows just the doors that server offers |
| **SSO** | `account.sso{server}` (new), then `account.create{gate}` | When the server gates names with OIDC: one button, "Continue with <provider>". The browser round-trip gives the engine an ID token. A first-time `sub` lands on Create account with the username prefilled and becomes an account at once; a returning `sub` on a new device lands on Restore (link, or password and code, or password alone when the server has a TPM), every path gated by that token |
| **Recovery code** | `recovery.code` | Shown once after account creation with a password, and from Settings; "write this down" page in the `RecoveryPage` layout |
| **Backup password** | `account.setPassword`, `recovery.status` | Set or change; shows backup state (enabled, pending, disabled) |

### 3.2 Home

- **Chats** tab: every conversation, sorted favourite → highlights → unread →
  last activity. Rows exactly as QML minus presence dots and the always-on
  lock badge. Requests never appear here.
- **Requests** tab (replaces Spaces): every `rooms.list` entry with
  `isInvite`. Row shows the sender's username and their first message; tap
  opens a request page with the message, "Accept" (`room.join`) and
  "Decline" (`room.leave`). Badge count on the tab. This is the request
  screen the design promised strangers would land in.
- **Start chat**: a username field with exact lookup, DM suggestions from
  existing conversations, and "New group" (name plus usernames) →
  `room.create`.
- **Account menu**: name, username, Link a device, Recovery, Settings, Sign
  out (`logout{wipe}` with a confirmation that explains a wipe).

### 3.3 Conversation

Everything in `ChatPage` and `BubbleDelegate` ports as is, with these
substitutions:

- Header subtitle: typing names, else "N members" for a group, else the
  username for a DM. No presence.
- Request banner instead of invite banner.
- The `can` object comes from the item: `edit` and `redact` for own
  messages, `redact` also for admins, `react` and `reply` always.
- Autocomplete: `@` over members. `#` removed.
- Message kinds rendered: text, image, video, audio, voice, file, sticker
  (D7), poll (D3), location (D8), contact (D6), redacted, membership, policy
  ("X renamed the group"), call ("X started a call"), day divider, read
  marker. `utd` (waiting for keys) has a Sigil twin: an envelope for an
  epoch this device never had, shown as "Sent before you joined".

### 3.4 Settings pages

| QML page | Sigil page | Content |
|---|---|---|
| RoomSettingsPage | Conversation settings | Avatar (local only, D10), name (`room.setSettings`), members with Admin tags, Add people, Pin and Low priority (local), Notifications, Privacy, Admins, Leave |
| NotificationsPage | Notifications | All / Mentions / Mute for this conversation, stored locally; the "custom setting" toggle keeps its shape with the default band explaining the account default from `notify.settings` |
| SecurityPage | Privacy | Read-only: always encrypted; members and device counts; current epoch number and when it last rotated; "Sigil's server cannot read this conversation" with a link to the plain-English design |
| RolesPage | Admins | Admins list; make admin / remove admin (`room.setAdmins`, part 4); only admins see the actions; the last admin cannot demote themself |
| PermissionsPage | dropped | |
| Space pages | dropped | |
| (new) App settings | Settings | Accent colour, notifications (`notify.settings`), **Privacy shape** (`shape.settings`: clocked tier seconds, SOCKS proxy for Tor, with the plain-English explanation from the design doc), Link a device, Recovery, About |

### 3.5 What Sigil deliberately does not have

These are gone, not pending: spaces and the space hierarchy, join rules,
history visibility, an encryption toggle, power levels and permissions,
presence, user directory search, join-by-address, room aliases, per-room
server push rules, device verification and cross-signing, secret storage,
SSO. The Privacy page explains why the first few are absent.

---

## Part 4. Engine work the port needs

The engine on `encrypt` already covers accounts, linking, recovery,
conversations, groups, text, reactions, receipts, typing, files, voice
recording, audio and video playback, position, map style config, and call
signalling. The QML also uses the following, which the engine must gain, in
the order the UI phases need them. Each is small; together they are the
feature half of "1:1".

| Feature | Sigil design | Engine surface |
|---|---|---|
| **Edit** | kind 3 event, `reference` = original id, body = new SigilText | `message.edit{roomId, eventId, body}`; item gains `isEdited`; the timeline replaces the body |
| **Delete** | kind 4 event, `reference` = target; receivers blank the item; only the sender or an admin | `message.redact{roomId, eventId}`; kind `redacted` |
| **Retry and cancel** | local send queue | `message.retry`, `message.cancelSend`; `sendState` `sending / sent / failed` on items |
| **Notification mode** | local per-conversation setting | `room.setNotify{roomId, mode}`; `notificationMode` on rooms |
| **Admins** | policy `admins[]` change, admins only | `room.setAdmins{roomId, add[], remove[]}` |
| **Pins (D5)** | a `pinned[]` list in the policy, changed by anyone (or admins; decide) | `message.pin`, `message.unpin`, `pins.list`; `room.pinned` event |
| **Polls (D3)** | new kind 15 poll `{question, options, closed, max}`; kind 16 vote `{reference, ids}`; kind 17 end | `poll.create`, `poll.vote`, `poll.end`; item `poll{…}` in the QML shape |
| **Threads (D4)** | text events carry `reference` plus a `thread` flag; thread summary counted locally | `thread.open{roomId, rootId}` (a timeline keyed `roomId\|thread:rootId` exactly like the QML), `threads.list` |
| **Stickers (D7)** | an image attachment with `sticker: true` in the manifest; packs are local folders | `stickers.list`, `sticker.send` |
| **Contacts (D6)** | a vCard file attachment with `contact: true`; "Sigil contact" carries a username | `contact.send`, `vcard.read`, `contacts.list/save/remove` |
| **Location (D8)** | new kind 18 location `{lat, lon, description, self}`; live shares as repeated kind 18 with `until`, ended by a final one | `location.send`, `location.startLive`, `location.stopLive`; `location.live` event; item `location{…}`, `liveShare{…}` |
| **Link previews** | fetched **on the device**, never by a server, off by default, with an obvious switch | `link.preview{url}` behind the setting |
| **Documents** | the `docs` module exists in `core/src/docs/` but is not dispatched on this branch | re-wire `doc.preview`, `doc.page`, `doc.thumb` |
| **Audio info** | cover art, accent, duration, waveform | re-wire `audio.info` |
| **Media ready** | already delivered as a `set` diff; the QML also listened for `media.ready` | keep the diff; the bridge derives the signal |
| **Calls, app side** | see 5.5 | `call.mute`, `call.camera`, `call.screenshare`, `call.devices`, `call.setDevice`, `call.react`, `call.decline` are **app-side** in Slint (the media stack lives in the app), so the bridge answers them itself and the engine keeps signalling only |
| **SSO gate** | design B24: the server validates an OIDC ID token (issuer, audience, expiry, JWKS) as the registration gate and for recovery; config `registration = "oidc"`, `oidc_issuer`, `oidc_client_id`; the card already flags it | server: token validation; engine: `account.probe`, `account.sso` (PKCE, loopback redirect), `gate` on `account.create` and `account.recover` |
| **Housekeeping** | | `engine.rs` still answers `call.*` with "calls arrive with Phase 7" when the Sigil session is absent; reword to "not signed in" |

Everything here is a Sigil event inside the conversation's encrypted
envelopes, so none of it changes what the server can see.

---

## Part 5. Where Slint differs from QML, and what to do

These are the places the first attempt either skipped or approximated. Each
has a decided approach so the port does not stall on them again.

### 5.1 No scripting

All computation lives in Rust. Specifically port these QML derivations
verbatim into `slint/src/`:

- Theme derivation (`docs/ui-conventions.md`): `accC`, `tintAmt 0.35`,
  `surfaceC`, `chromeC`, `convoC`, `chipC`, `deepChipC`, `themedSend`. One
  function, used by chat, threads, pins, and the sheet.
- Bubble fill: own = accent mixed 0.42 over popup background; highlighted =
  0.24; other = theme surface or foreground at 0.22.
- Group corners: radius 16, `rSmall 5` on the sender side between
  consecutive same-sender bubbles; max width 78 %.
- Grouping and stamps: 5-minute and same-day sender grouping, session
  day labels, `showHeader`, `groupEnd`.
- Home sort order and relative-time formatting.
- Call layout choice (`featured`, `groupMode`), the 2-column grid rules,
  and the PiP spring (`k 120, c 15`, throw lead `0.14 s`, restitution 0.3).
- Viewer zoom (`zoomAbout`, `clampOffsets`) and the wheel accumulator.
- Chat theme gradients (`gradPair`, 3 hues × 3 depths).

### 5.2 Animations

Slint animates properties on a clock with easing; that covers every
`Behavior` and `NumberAnimation` in the QML (slide-ins, OutBack scale
pops, opacity fades, the segmented control thumb, typing dots, reaction
lift, detail-row open). Two QML animations were frame-driven to survive
UI-thread stalls: page slide-in and jump-to-latest. In Slint, build the
page content off screen first, then slide; and drive jump-to-latest by
animating the list's viewport offset. Entry animations for new bubbles
(rise from the composer, 54 px, scale 0.84, OutBack) need a per-row
`entered` flag set by the bridge when the row is appended; Slint rows
animate from that flag.

### 5.3 Blur and frost

Slint has no backdrop blur. The message sheet and the image viewer frost
the page behind them. Do it the way the QML actually does, which is also
a snapshot: when the sheet opens, the bridge renders the current page to
an image (Slint's software renderer can render any component to a pixel
buffer), blurs it in Rust (a box blur at radius 24, twice, matches
`MultiEffect` blur 0.6 closely enough to be indistinguishable at 55 %
black), and hands it to the sheet as its background `Image`. Cost is a
few milliseconds on a phone. The lifted bubble copy is the same trick on
the bubble's rectangle.

### 5.4 Video frames

Slint draws an `Image` from a `SharedPixelBuffer`. The app owns a frame
channel: in process, the engine's call and video code writes RGBA frames
straight into a buffer the bridge swaps into the `Image` at up to 60 Hz,
only while the tile is visible. Out of process (the daemon case on Linux),
`frames.rs` reads the `omv_shm.h` layout with the same seqlock the C++
plugin uses. Fit, crop and mirror are `Image` properties. Rounded corners
come from `clip: true` on a rounded `Rectangle`, which Slint supports for
plain images.

### 5.5 The call media stack

The engine hands SDP in and out; everything else is the app's. Build it as
a Rust module `slint/src/call/` in this order, each step usable on its own:

1. **Audio only.** `str0m` as the WebRTC peer (the same crate the server
   uses), `cpal` for microphone and speaker, `opus` for the codec, a simple
   jitter buffer. `call.state` produced locally in the QML shape:
   `participants[]` with `micMuted`, `speaking`, `level` (from the decoded
   audio), `local{…}`. Devices from `cpal` for `call.devices`.
2. **Video receive.** VP8 decode (`vpx` bindings or a pure-Rust decoder)
   into the frame channel; `tracks[{kind: "camera"}]`.
3. **Camera send.** `nokhwa` capture on desktop, `camera2` through the
   Android activity, VP8 encode.
4. **Screen share.** PipeWire portal on Linux; later elsewhere.
5. **Reactions** as tiny data-channel messages between peers, exactly as
   LiveKit did it, so they never touch the conversation.
6. **SFrame** on frames with a key from the conversation epoch, so the
   forwarding unit relays what it cannot decode. This closes the last gap
   in the server-blind design and belongs here, not in the engine.

The call pages port against the QML shape from step 1, so voice calls work
before any video exists.

### 5.6 Text

Slint text is plain: no HTML, no per-span styling. That is the biggest
fidelity gap on the `slint` branch and it is closed by doing what the QML
already does for effects, for all text:

- The engine already produces `parts` and `effects` per message. Rust lays
  each message out into **runs**: plain runs, styled runs (bold, italic,
  mono, colour, link, spoiler), and animated glyph runs.
- `richtext.slint` draws runs in a flow: one `Text` per run, one per glyph
  for animated runs, wrapping decided in Rust with the same measurement the
  QML did off-layout.
- Links get a tap area per run. Code blocks are their own component with
  the highlighter's spans as runs.
- GIFs: decode frames in Rust and step an `Image`; show the GIF badge.

### 5.7 Input

The portability rules hold: every hover has a tap twin, every right-click
a long-press twin, tap targets 44 px. Slint's `TouchArea` gives press,
release, long-press (`pointer-event`), and scroll; pinch is two pointers
tracked in the bridge. The passive-grab lesson from the map does not
arise: Slint has no grab contention.

---

## Part 6. Phases

Each phase ends with a **side-by-side sheet**: a screenshot of every page in
the phase from the Slint app next to the same page in QML, with the same
fixture data, plus the gap table (part 8). A phase is not done until the
sheet exists and every difference on it is either fixed or listed as a
decision.

**Phase 0. Foundations.** *Done.* Start from the `slint` branch's crate. Reconcile
`bridge.rs` with the Sigil protocol: delete Matrix state, add account,
link, recovery, requests, shape. Tokens, icons, fonts, `PageHolder`,
settings furniture, `OverflowMenu`, `ChoiceSheet`, `Dialogs`, `Avatar`,
`IconLabel`, `Spinner`, scrollbar, toast, `TextMenu`. Fixture data for the
screenshot harness (`slint/tests/fixtures/`), and the harness itself: a
test that mounts each page with fixtures under the software renderer and
writes a PNG. Files: 1.1, 1.2.

**Phase 1. Doors.** *Done; `slint/tests/e2e-doors.sh` walks the real flow against a real server.* Welcome, Create account, Recover, Link this device (both
sides), Recovery code, Backup password, App settings with Privacy shape,
Sign out. Files: `login.slint`, `recovery.slint`, `pages/settings.slint`.

**Phase 2. Home.** *Done; `slint/tests/e2e-home.sh` has the app receive, accept and answer a request from the command-line client.* Chats and Requests, search, sort, badges, FAB, account
menu, Start chat, New group, request page. Files: `home.slint`,
`pages/start.slint`, `pages/request.slint`.

**Phase 3. Conversation.** *In progress: edit and delete are in the engine; the frosted sheet, Enter to send, and `slint/tests/e2e-chat.sh` (reply with quote, reaction, edit, delete, each seen by the command-line client) are done.* The whole of `ChatPage` and `BubbleDelegate` for
text, image, video, file, audio and voice; composer with reply and edit
staging; reactions; receipts with reader avatars; typing; day labels;
grouping; pagination; jump-to-event with the flash; keyboard shortcuts;
message sheet with the frosted snapshot; emoji picker; attach menu (Files,
Emojis); forward; search; chat theme; SigilText runs and all effects; code
blocks; GIFs. Engine: edit, delete, retry, cancel. Files: 1.4 minus polls,
contacts, location, plus `viewer.slint`.

**Phase 4. Groups.** *Done; `slint/tests/e2e-groups-app.sh` has the app make a group, add the command-line client, make it an admin, rename and leave, with every policy change heard on the other side.* Conversation settings, Members, Add people, Admins,
Notifications, Privacy, Leave, rename, request banner. Engine: admins,
notification mode. Files: 1.3 rows that survive.

**Phase 5. Media pages.** *Done for what a machine without a microphone or
ffmpeg can show; `slint/tests/e2e-chat.sh` now also sends a Markdown
document and a WAV track, opens the document page and the audio page, and
sends a voice message, each seen by the command-line client.* Image viewer
complete (zoom, pan, pager, react, delete, download), Document page and
thumbs, Audio page and body, voice recorder. Engine: re-wire docs and audio
info; frame channel for video playback. Files: 1.5, `DocThumb`,
`AudioBody`, `VoiceRecorder`.
Engine work done here: `doc.preview`, `doc.thumb`, `doc.page` and
`audio.info` on the Sigil backend (`core/src/sigil/docs.rs`), `voice.send`
with the clip's length and waveform carried in the file manifest, own files
deletable, a size label on every file. The recorder itself, track length
and cover art, and video frames all go through ffmpeg exactly as they did
under QML, so they are exercised on a desktop with ffmpeg installed rather
than in the headless suite; the recorder page is on the fixture sheet
(`recorder.png`) with a clip ready to send. Gap kept: a video bubble shows
its first frame only where ffmpeg is present.

**Phase 6. More kinds.** *Done; `slint/tests/e2e-kinds.sh` has the app pin
a message, ask a poll that both sides vote on and the asker ends, answer in
a thread that the other side answers too, send a sticker from a local pack,
share a contact card from the member sheet, share a place through the
picker and open it on the map page, receive a place, and draw a link card
for a page served on loopback once the switch is on.* Whichever of pins,
polls, threads, stickers, contacts, location and link previews the
decisions in part 7 keep, each as one engine change plus its bodies and
pages. Files: `PollBody`, `ContactBody`, `LocationBody`, `PinsPage`,
`ThreadsPage`, map pages.
Engine work done here (`core/src/sigil/kinds.rs`, wire spec 16): pins as a
policy field anyone may change; polls as kinds 15 to 17; threads as a text
reference, with the main view hiding replies and a thread view per root;
stickers and contacts as flagged files; places as kind 18 with live shares
updated every half minute; link previews fetched on the device, off until
switched on in Settings. Gaps kept, as decided: no map tiles (the pin card
and the coordinates stand in, and `location.map` says so), and "drop a
pin" waits for a map to tap on. A `sigil-cli event` command sends any raw
event, which is how the tests play the other side.

**Phase 7. Calls.** The media stack in the order of 5.5, then `CallPage`,
`CallGrid`, `CallPiP`, `CallBanner`, `CallBar`, the wide layout, devices,
reactions. Voice calls ship at step 1; the pages are complete from the
start because `call.state` has its final shape from the start.

**Phase 8. Platforms.** Android activity, file pickers (SAF, portal),
system notifications from the engine's `notify` event, the omarchy toggle
plugin, desktop tray for the call pill, packaging.

Rough size, measured against the QML line counts and what the first attempt
took: phases 0 to 2 are small, 3 is the largest UI phase, 6 depends on the
decisions, 7 is the largest engineering phase because of the media stack.

---

## Part 7. Decisions for the owner

The plan proceeds with the recommendation unless told otherwise.

| # | Question | Recommendation |
|---|---|---|
| D1 | Spaces | Drop. Sigil has no server-side hierarchy and no public rooms. If grouping is wanted later, local folders on the device, nothing on the wire |
| D2 | Presence | Drop. It would put "online now" on the server's path, which is exactly the metadata Sigil refuses to give it |
| D3 | Polls | Keep, as Sigil events (part 4) |
| D4 | Threads | Keep, as references. Cheap on the wire, and the pages exist |
| D5 | Pins | Keep, in the policy, changeable by anyone in the conversation |
| D6 | Contacts and vCards | Keep. Sharing a contact is a file plus a username |
| D7 | Stickers | Keep as flagged images with local packs; no `im.ponies` |
| D8 | Location and maps | Keep the events. Ship the pin-card fallback first; a raster tile map drawn in Rust is a phase-6 add-on, and the tile source is the user's own choice in settings since fetching tiles reveals where you are looking |
| D9 | Decline for calls | Add a kind-10 `{action: "decline"}` so the caller's ringing stops; otherwise the banner just closes locally |
| D10 | Group avatars | Local only for now; a shared avatar is a small policy field later |
| D11 | Link previews | On-device fetch only, off by default |
| D12 | Desktop form factor | A normal window sized like the card; the omarchy toggle plugin comes last |

---

## Part 8. How to run this with a Claude session, and how to check its work

Give the session this document and one phase at a time. Require, at the
end of each phase:

1. **The side-by-side sheet**: one PNG per page from the screenshot harness,
   next to the QML page rendered with the same fixtures (the QML can be
   rendered with `qml` on the desktop, or screenshotted once and kept under
   `docs/port/qml/`).
2. **The gap table**, one row per QML file in the phase:
   `QML file | Slint file | what differs | why | fixed or decision #`.
   A file with no row is not done. "Looks the same" is not a value for the
   third column; the value is a measurement or "none".
3. **The feature list for the phase**, each line with the request it sends
   and the test that exercises it in `core/tests/e2e-sigil.sh` or the Slint
   harness.

Do not accept "the full port" as a status. Accept a sheet, a table, and a
list, or nothing.
