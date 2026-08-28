# sigil-engine socket protocol (v1)

Transport: unix socket `$XDG_RUNTIME_DIR/sigil.sock`, UTF-8 JSON, one object per line.

* Request `{"req":"<name>","id":N, ...params}` → reply `{"reply":N,"ok":true,"result":{…}}` or `{"reply":N,"ok":false,"error":{"code","message"}}`.
* Pushes `{"event":"<name>", …}` go to every connected client.
* On connect: `hello{protocol,engine,pid}`, `status`, `recovery.status`, `rooms.list` (if any), `spaces.tree` (if any), `call.state`.

Error codes: `bad_request not_logged_in login_in_progress oidc_unsupported sliding_sync_unsupported unknown_room unknown_event recovery_key_invalid permission_denied network no_livekit call_busy no_call device_error internal`.

## Requests

| Request | Params | Result / effect |
|---|---|---|
| `ping`, `status` | | `status` object |
| `login.start` | `homeserver`, `openBrowser` | `{url}`; later `login.finished` / `login.failed` events |
| `login.cancel`, `login.finish{query}` | | |
| `logout` | `wipe` | |
| `recovery.status`, `recovery.recover{key}` | | `recovery.status` object |
| `rooms.list`, `spaces.tree` | | last snapshot |
| `room.members{roomId}` | | `{members:[{userId,displayName,avatarPath,powerLevel,membership}]}` |
| `room.join{roomIdOrAlias}`, `room.leave{roomId}`, `room.invite{roomId,userId}`, `room.create{name,topic,private,encrypted,invite[]}`, `dm.create{userId}`, `users.search{query,limit}` | | |
| `room.setFavourite{roomId,favourite}`, `room.setLowPriority{roomId,lowPriority}`, `room.setUnread{roomId,unread}`, `room.markRead{roomId}` | | |
| `space.hierarchy{spaceId,limit?}` | | `{rooms:[{id,name,topic,avatarPath,memberCount,isSpace,worldReadable,encrypted,joined}],nextBatch}` — the server's `/hierarchy`, so it includes children this account has NOT joined |
| `space.addRoom{spaceId,roomId}`, `space.removeRoom{spaceId,roomId}` | | `m.space.child` state on the space |
| `room.settings{roomId}` | | everything the settings pages read in one round trip: identity, `joinRule`, `historyVisibility`, `isEncrypted`, `notificationMode` (null = follow the account default), `myPowerLevel`, `can{…}`, `powerLevels{…}` |
| `room.setSettings{roomId, name?, topic?, joinRule?, restrictedTo?, historyVisibility?, encrypted?, notificationMode?}` | | writes only the fields present; absent means "leave alone". `encrypted` is one-way. `notificationMode` of `default`/null deletes the per-room rule |
| `room.setAvatar{roomId,path}` | | uploads and sets; empty `path` removes |
| `room.setPowerLevel{roomId, userId? \| key?, level}` | | `userId` moves a person between roles; `key` is one of `invite,kick,ban,redact,eventsDefault,stateDefault,usersDefault,name,avatar,topic,liveLocation` |
| `room.open{roomId,initialItems}` / `room.close{roomId}` | | `timeline.reset` then `timeline.diff` pushes; `room.typing` |
| `timeline.paginate{roomId,count}` | | `{hitStart}`; `timeline.paginationState` pushes |
| `message.send/reply/edit{roomId,eventId?,body,markdown}`, `message.react{roomId,eventId,key}`, `message.redact{roomId,eventId,reason?}` | | |
| `attachment.send{roomId,path,caption?}` | | |
| `typing{roomId,typing}`, `readReceipt{roomId,eventId}`, `ui.focus{roomId,visible}` | | |
| `media.get{roomId,eventId,thumbnail?{width,height}}` | | `{path,filename,mime}` |
| `media.saveAs{roomId,eventId,dest}` | | `{path}` |
| `notify.settings{enabled?,dms?,mentions?,calls?}` | | settings |
| `call.devices`, `call.setDevice{kind,id_}`, `call.start/join{roomId,video}`, `call.decline{roomId}`, `call.leave`, `call.mute{muted}`, `call.camera{enabled}`, `call.screenshare{enabled}`, `call.state` | | `call.state` pushes |

## Timeline ops

`timeline.diff.ops[]` mirror `eyeball_im::VectorDiff` (oldest-first indices):
`append{items}`, `clear`, `pushFront{item}`, `pushBack{item}`, `popFront`, `popBack`,
`insert{index,item}`, `set{index,item}`, `remove{index}`, `truncate{len}`, `reset{items}`.

Item: `{id, kind, eventId, txnId, sender, senderName, senderAvatarPath, ts, isOwn, isHighlighted,
body, html?, isEdited, replyTo?{eventId,sender,senderName,kind,body}, threadRoot?,
reactions[{key,count,senders}], media?{mxc,encrypted,filename,mime,width,height,size,blurhash,duration,thumbnailPath,path},
sendState, sendError, readBy[{userId,ts}], utdReason?, stateText?, can{edit,reply,redact,react}}`.
Kinds: text notice emote image video audio voice file sticker poll redacted utd membership profile state call rtcNotification dayDivider readMarker timelineStart unsupported.

## Video frames

`call.state.participants[].tracks[]` / `local.tracks[]` carry `shmPath`; the file layout is `video/omv_shm.h`.
