# Wiring: settings group

Eight pages in `slint/ui/pages/`. All engine names below are from
`core/docs/protocol.md` / Service.qml. Every page keeps `closed` (shell-wired)
and `in fg`.

## Shared

- `RoomSettingsModel` fills from `room.settings {roomId}` + the room's
  `rooms.list` summary (avatar/name/topic/isFavourite/isLowPriority/joinedMembers).
- Writes are optimistic where noted; a `room.settings` re-read lands stale
  (push rules / state propagation), so re-read ~1.2 s later, not immediately.
- model.slint additions wanted (not made): per-capability booleans on
  RoomSettingsModel (`can-set-name/topic/avatar/join-rule/history/encryption`)
  — currently approximated with `can-edit-info`/`can-edit-permissions` plus
  page-local ins on SecurityPage.

## RoomSettingsPage (`roomsettings.slint`)
IN: `model: RoomSettingsModel`; `members: [MemberRow]` from `room.members {roomId}`
(fetch on open, group rooms only); `spaces: [SpaceMembershipRow]` (exported from
this file) from spaces.tree × room membership (`in-space` = roomId ∈ space.children);
`pinned-count` = count of isFavourite rooms; `dm-user-id`.
CALLBACKS: `add-people` → nav addpeople (add-return per Panel); `open-notifications`
/`open-security`/`open-roles` → nav with settings-return="roomsettings";
`set-favourite(bool)` → `room.setFavourite`; `set-low-priority(bool)` →
`room.setLowPriority`; `toggle-space(spaceId, add)` → `space.addRoom`/`space.removeRoom`;
`leave` → `room.leave {roomId}` then nav home.

## SpaceSettingsPage (`spacesettings.slint`)
IN: `model`, `can-edit-info`, `busy`, `error`, `new-avatar-path`+`new-avatar`
(file picker result; picker itself is platform work), seeds `name-text`/`topic-text`
(in-out) from settings on open.
CALLBACKS: `pick-avatar` → platform file chooser; `save(name, topic)` →
`room.setSettings` with only changed fields, THEN `room.setAvatar` if
new-avatar-path set (avatar second: doing it first orphans the upload if the
name change fails — QML comment); `open-members` (members-return="spacesettings"),
`open-notifications`/`open-security`/`open-roles` (settings-return="spacesettings").

## NotificationsPage (`notifications.slint`)
IN: `mode` ("" = no per-room rule), `encrypted`, `busy`, `error`.
CALLBACK: `set-mode(m)` → `room.setSettings {notificationMode: m}` ("default"
deletes the rule). Bridge shows the write optimistically, re-reads at 1.2 s.

## SecurityPage (`security.slint`)
IN: `model`, `parent-space-id`/`parent-space-name` (first space containing the
room), `can-set-join-rule`/`can-set-history`/`can-set-encryption`, `busy`,
`error`; seeds `join-rule-edit`/`history-edit`/`want-encrypted` (in-out) on open.
CALLBACK: `save(joinRule, history, encrypted)` → one `room.setSettings` with
only the changed fields; add `restrictedTo: parent-space-id` when joinRule
becomes "restricted"; `encrypted` only ever sent as true.

## RolesPage (`roles.slint`)
IN: `admins`/`moderators` counted from settings.powerLevels.users (>=100 /
50..100), `my-level` = myPowerLevel, `can-set-power-levels`, `busy`;
`note` (in-out) for bridge error toasts.
CALLBACKS: `open-permissions` → nav permissions; `open-role(level)` → nav
members with filter-level set; `set-my-role(level)` →
`room.setPowerLevel {userId: me, level}`; `reset-permissions` → the Matrix
defaults {invite:0, kick:50, ban:50, redact:50, eventsDefault:0,
stateDefault:50, name:50, avatar:50, topic:50, liveLocation:50} sent ONE KEY
AT A TIME via `room.setPowerLevel {key, level}` (never batch).

## PermissionsPage (`permissions.slint`)
IN: three `[PowerRow]` bands with labels:
member-perms [invite "Invite people", kick "Remove people", ban "Ban people"];
detail-perms [name "Change name", avatar "Change avatar", topic "Change topic"];
content-perms [eventsDefault "Send messages", redact "Remove messages",
liveLocation "Share live location"]. `may-edit` = can.setPowerLevels; `busy`;
`note` in-out.
CALLBACK: `set-level(key, level)` → `room.setPowerLevel {key, level}`, then
re-read settings.

## MembersPage (`members.slint`)
IN: `members: [MemberRow]` — bridge filters by `filter-level` (-1 all, 100
admins, 50 mods 50..100) and refills on `room.members` / filter change;
`all-count` = unfiltered length (0 → "Loading…"); `filter-level`.
CALLBACKS: `invite` → nav addpeople (add-return = members-return context).
MemberRow.role = "Admin"/"Moderator"/"" from powerLevel.

## AddPeoplePage (`addpeople.slint`)
IN: `results: [UserRow]`, `note` (in-out; bridge writes invite outcome:
"Invited X" / "Invite failed: …").
CALLBACKS: `search(q)` (page debounces 300 ms; ""→clear results; bridge:
`users.search {query, limit:12}`, only when len ≥ 2); `invite-user(userId)` →
`room.invite {roomId, userId}`.
Page functions `reset()` / `focus-search()` exist for the shell.

## Not ported (noted, out of scope)
Avatar presence dots (Avatar component lacks presence ring — affects
RoomSettings member preview + AddPeople rows).
