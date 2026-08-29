# Wiring: spaces & discovery group

Per page: `in` properties the bridge fills, callbacks → engine requests, and
model.slint additions still needed. Protocol names are from core/docs/protocol.md
and Service.qml.

## SpacePage (pages/space.slint)
Fill on `nav-opened("space")` for `space-id`:
- `space-name/space-avatar/space-initials/space-tint` — from spaces.tree entry, falling back to rooms.list.
- `is-public` — `room.settings{roomId: spaceId}` → joinRule == "public"; `member-count` — same reply's memberCount, else rooms.list joinedMembers.
- `children: [HierarchyRow]` — `space.hierarchy{spaceId}` → rooms[]; set `loading` around the call, `loaded` after first reply. Reload debounced 700ms on every spaces.tree push (Service.qml reload timer).
- `note` (in-out): bridge may set "Could not load this space" / "Could not join …" / "Link copied"; page auto-clears after 2.6s.
Callbacks:
- `open-room(id)` → selectRoom flow (room.open etc.), nav→chat.
- `join-room(id)` → `room.join{roomIdOrAlias}`; on error set note; on success re-request space.hierarchy.
- `create-room` → nav to start-page create mode with spaceForNewRoom = spaceId (Panel: room created from a space lands in it via space.addRoom after create).
- `add-existing` → SpaceRoomsPage mode="add"; `manage-rooms` → mode="manage"; both nav "spacerooms".
- `view-members` → members page with settingsRoomId = spaceId, members-return = "space".
- `open-settings` → nav "spacesettings".
- `share-space` → clipboard copy of https://matrix.to/#/<canonicalAlias || spaceId>; then set note = "Link copied". (Clipboard: slint has no clipboard API on Android yet — JNI ClipboardManager, platform.rs.)
- `leave-space` → `room.leave{roomId: spaceId}`; on success nav home (Panel leftSpace → goHome).
Gap noted: HierarchyRow has no `world-readable`; page shows Public/Private from `!encrypted` (the same fallback SpaceRoomsPage.qml uses). Add `world-readable: bool` to HierarchyRow and switch both pages when convenient.

## SpaceRoomsPage (pages/spacerooms.slint)
- `mode` — "manage" | "add" (set by SpacePage callbacks).
- `rows: [HierarchyRow]` — manage: space.hierarchy children; add: rooms.list entries with !isSpace && not already a child && id != spaceId, mapped into HierarchyRow. **The `in-space` field doubles as the row's "picked" mark** — bridge holds the selected set and flips it per row on rebuild.
- `selected-count`, `busy`, `loading`, `note` as in QML.
Callbacks: `toggle(id)` — bridge mutates selection, rebuilds rows; `apply` — fan out `space.addRoom{spaceId, roomId}` / `space.removeRoom` per selected id, count failures, note "N could not be changed", clear selection, reload.

## NewSpacePage (pages/newspace.slint)
- `avatar-preview`/`has-avatar` — set after `pick-avatar` resolves via platform file picker (Android: SAF intent — platform.rs addition needed; desktop: xdg portal or omarchy-file-select).
- `busy`, `error`.
Callbacks: `create(name, topic, private)` → `room.create{name, topic, private, encrypted:false, space:true}`; avatar is a second call on purpose (`room.setAvatar`) — create_room has no avatar field and uploading first leaves an orphan upload when creation fails. On success: nav to the new space (Panel: created → openSpace). Call page's `reset()` when navigating in.

## StartPage (pages/start.slint)
- `people: [UserRow]` — query ≥ 2 chars: `users.search{query, limit:12}` **debounced 300ms in the bridge** (QML used a Timer); else suggestions = rooms.list DMs mapped to UserRow.
- `busy`, `error`.
Callbacks: `start-dm(userId)` → `dm.create{userId}` → open resulting roomId, nav chat; `submit-extra(mode, value)` — create: `room.create{name, topic:"", private:true, encrypted:true}` → open room; space: `room.create{…, space:true, encrypted:false}` → close page (opening a space shows an empty timeline); join: `room.join{roomIdOrAlias}` → open room. Call `reset()` on nav-in.

## ForwardPage (pages/forward.slint)
- `mode` — "forward" | "attach"; `chats: [RoomRow]` — rooms.list minus spaces/invites, recency-sorted, filtered by the search text (bridge holds the query from `search-edited`).
Callbacks: `picked(roomId)` — forward mode: bridge holds the payload item; image w/ media path → `attachment.send{roomId, path}`, else `message.send{roomId, body}`; then nav chat on that room. Attach mode: stage pendingShare files into the chosen room's composer flow.

## SearchPage (pages/search.slint)
- Bridge ports Service's collect(): over the open room's shadow items — `results` (body contains query, kind != image; cap 40), `images` (kind == image with thumbnailPath loaded into `thumb`; cap 12), `links` (first http(s) URL in body, carried in `body`; cap 10). Recompute on `search-edited` (bridge keeps query; `searching` = len ≥ 2) and on timeline.diff for the room.
Callbacks: `jump-to(eventId)` → nav chat + scroll-to-event (chat parity task); `open-image(eventId)` → image viewer (media task); `open-link(url)` → platform::open_url.

## ThreadsPage (pages/threads.slint)
- `room-name`; `threads: [ThreadRow]` — `threads.list{roomId}` → threads[] mapped {rootId, senderName, sender→tint/initials, avatarPath→avatar, body, count→reply-count, ts→last-ts-label ("HH:mm"/"Yesterday"/ddd/d MMM — the room-list clock)}. `loading`/`loaded`.
- Reload debounced 700ms on thread activity: any diff op item with threadRoot/threadSummary on the room key, or any diff on a `roomId|thread:*` key (Service.noteThreadActivity).
Callbacks: `thread-picked(rootId)` → `thread.open{roomId, rootId, initialItems:60}` → returned key becomes thread-key; nav "thread" renders the ordinary chat page on that key.
- `accent`: pass the room's chat-theme accent when chat themes land; Theme.accent until then.

## PinsPage (pages/pins.slint)
- `room-name`, `room-is-dm`; `items: [TimelineRow]` — `pins.items{roomId}` mapped with **pins stamp format** in `stamp` ("Today · h:mm AP" / "Yesterday · …" / "d MMM · …") and the kind chip in `media-icon` + words in `media-filename` (image→Photo, video→Video, audio→Audio, file→File, location→Location, sticker→Sticker; empty for text). `loading`/`loaded`.
- Reload on room.pinned push for the room.
Callbacks: `jump-requested(eventId)` → nav chat + scroll-to-event; `unpin(eventId)` → `message.unpin{roomId, eventId}`.
- `accent` as ThreadsPage.

## model.slint additions wanted (do not block)
- HierarchyRow: `world-readable: bool` (Public/Private now approximated by !encrypted).
- A dedicated SearchResult struct would read better than TimelineRow reuse; optional.

## Shared notes
- Pages with `reset()` need it invoked on nav-in (nav-opened handler).
- Note toasts auto-clear via an in-page Timer; the bridge only ever *sets* `note`.
- Clipboard (SpacePage share) and file picker (NewSpacePage avatar) are new platform.rs seams.
