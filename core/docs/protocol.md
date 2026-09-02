# sigil-engine socket protocol (v1)

Transport: unix socket `$XDG_RUNTIME_DIR/sigil.sock`, UTF-8 JSON, one object per line.

* Request `{"req":"<name>","id":N, ...params}` → reply `{"reply":N,"ok":true,"result":{…}}` or `{"reply":N,"ok":false,"error":{"code","message"}}`.
* Pushes `{"event":"<name>", …}` go to every connected client.
* On connect: `hello{protocol,engine,pid}`, `status`, `recovery.status`, `rooms.list` (if any), `position`.
* `status` carries `backend: "sigil"` and `accountSaved`. On the `encrypt` branch the engine speaks only the Sigil backend (`docs/blind-backend.md`); requests marked *(later)* below answer `unsupported` until their phase lands.

Error codes: `bad_request not_logged_in login_in_progress oidc_unsupported sliding_sync_unsupported unknown_room unknown_event recovery_key_invalid permission_denied network no_livekit call_busy no_call device_error internal`.

## Requests

| Request | Params | Result / effect |
|---|---|---|
| `ping`, `status` | | `status` object |
| `account.create` | `username` (`@name:server`), `invite`, `password?`, `envoy?` (default `wss://<server>/envoy`) | registers the name, draws tokens, publishes key packages; with a password, sets up backup and recovery; `status` becomes `loggedIn` |
| `account.recover` | `username`, `password`, `code`, `envoy?` | fresh device: restores account, conversations and history from the encrypted backup; `{userId}` |
| `account.setPassword` | `password` | sets or changes the backup password |
| `recovery.code` | | `{code}`: the printed recovery code to show once and let the user save |
| `account.status` | | `{exists, active}` |
| `account.probe` | `server` (host, or a full URL for a test server) | asks the server for its card, straight from `/info`: `{hostname, registration: open\|invite\|oidc, tpm}`; the first screen decides which doors to show from this |
| `link.offer` | `username`, `envoy?` | new device: `{offer}` to show as a QR code; then `link.state` events: `offer`, `sas{sas}`, `joining{with}`, `done` (session starts) or `failed{error}` |
| `link.scan` | `offer` | existing device: `{sas}`, also pushed as `link.state{state:"sas"}`; nothing is sent yet |
| `link.confirm` | `ok` | existing device, after the user compared the emoji: `ok:true` transfers the account and adds the new device to every conversation; `ok:false` cancels |
| `logout` | `wipe` | `wipe` deletes the account, MLS store and history from this device |
| `recovery.status` | | `{recovery: enabled|disabled, backup: enabled|pending|disabled, verified}`; also pushed when it changes |
| `shape.settings` | `clockedSeconds?`, `socksProxy?` | the paranoid page: read or set the clocked tier (seconds between bags whether or not there is anything to say, 0 = off) and a SOCKS5 proxy such as a local Tor daemon (`127.0.0.1:9050`, empty = direct); returns both plus `appliesOn` |
| `login.*` | | *(removed)* answer `unsupported` |
| `rooms.list`, `spaces.tree` | | last snapshot |
| `room.members{roomId}` | | `{members:[{userId,displayName,avatarPath,powerLevel,membership}]}` |
| `dm.create{userId}` | | starts a conversation by username: takes a key package, creates the MLS group, sends the Welcome to their requests slot; `{roomId}` |
| `room.join{roomIdOrAlias}` | | accepts a request: the `id` of a `rooms.list` entry with `isInvite`, which look like `req:…` |
| `room.leave{roomId}` | | drops a request or forgets a conversation locally |
| `users.search{query}` | | exact username lookup; `{results:[{userId,displayName,avatarPath}]}` |
| `room.create{name, invite[]}` | | a group: creates the MLS group, sends a Welcome with the policy to each invitee's requests slot; `{roomId}` |
| `room.invite{roomId, userId}` | | adds a member: commit, Welcome, updated policy |
| `room.setSettings{roomId, name}` | | renames via a policy event; other fields are ignored for now |
| `attachment.send{roomId, path, caption?}` | | encrypts and uploads the file in chunks, sends the manifest; the item has `media.path` set locally |
| `media.get{roomId, eventId}` | | `{path, filename, mime}`; downloads on first call. Received media downloads in the background and the item's `media.path` (and `thumbnailPath` for images) is set by a `set` diff |
| `room.setFavourite{roomId,favourite}`, `room.setLowPriority{roomId,lowPriority}`, `room.setUnread{roomId,unread}`, `room.markRead{roomId}` | | |
| `space.hierarchy{spaceId,limit?}` | | `{rooms:[{id,name,topic,avatarPath,memberCount,isSpace,worldReadable,encrypted,joined}],nextBatch}` — the server's `/hierarchy`, so it includes children this account has NOT joined |
| `space.addRoom{spaceId,roomId}`, `space.removeRoom{spaceId,roomId}` | | `m.space.child` state on the space |
| `room.settings{roomId}` | | everything the settings pages read in one round trip: identity, `joinRule`, `historyVisibility`, `isEncrypted`, `notificationMode` (null = follow the account default), `myPowerLevel`, `can{…}`, `powerLevels{…}` |
| `room.setSettings{roomId, name?, topic?, joinRule?, restrictedTo?, historyVisibility?, encrypted?, notificationMode?}` | | writes only the fields present; absent means "leave alone". `encrypted` is one-way. `notificationMode` of `default`/null deletes the per-room rule |
| `room.setAvatar{roomId,path}` | | uploads and sets; empty `path` removes |
| `room.setPowerLevel{roomId, userId? \| key?, level}` | | `userId` moves a person between roles; `key` is one of `invite,kick,ban,redact,eventsDefault,stateDefault,usersDefault,name,avatar,topic,liveLocation` |
| `room.open{roomId,initialItems}` / `room.close{roomId}` | | `timeline.reset` then `timeline.diff` pushes; `room.typing` |
| `timeline.paginate{roomId,count}` | | `{hitStart}`; `timeline.paginationState` pushes |
| `message.send{roomId,body}`, `message.reply{roomId,eventId,body}` | | `body` is SigilText source; the engine composes it so every device renders the same |
| `message.react{roomId,eventId,key}`, `readReceipt{roomId,eventId}`, `typing{roomId,typing}` | | small events in the same slot; typing is rate-limited to one per 5 s |
| `message.edit`, `message.redact` | | *(later)* |
| `attachment.send{roomId,path,caption?}` | | |
| `typing{roomId,typing}`, `readReceipt{roomId,eventId}`, `ui.focus{roomId,visible}` | | |
| `media.get{roomId,eventId,thumbnail?{width,height}}` | | `{path,filename,mime}` |
| `media.saveAs{roomId,eventId,dest}` | | `{path}` |
| `notify.settings{enabled?,dms?,mentions?,calls?}` | | settings |
| `call.start{roomId}` | | announces a call in the conversation (kind-10 event) with a fresh random room; `{callId}` |
| `call.end{roomId, callId}` | | announces the end |
| `call.join{roomId, callId, offer}` | | joins the forwarding unit on the conversation's server with an SDP offer; `{answer, peer}` |
| `call.poll{roomId, callId, peer}` | | `{offer\|null, peers}`: a renegotiation offer from the unit when another participant's track was added, and the head count |
| `call.answer{roomId, callId, peer, answer}` | | completes a renegotiation |
| `call.leave{roomId, callId, peer}` | | leaves the unit's room |
| `call.state` (event) | | pushed as `{roomId, callId, state: started\|ended, sender}` when a call event arrives or is sent; the timeline also gets a `call` item. The media stack (capture, encoding, playback, the WebRTC peer) is the frontend's for now: it hands SDP in and gets SDP out |

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
