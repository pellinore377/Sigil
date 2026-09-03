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
| `account.probe` | `server` (host, or a full URL for a test server) | resolves the name through its pointer (`/.well-known/sigil`) and asks the server for its card from `/info`: `{hostname, registration: open\|invite\|oidc, tpm, base, envoy}` (`base` and `envoy` are where the name resolved to; the app connects there); with `oidc`, also `oidc: {issuer, clientId, name}` from the server's `/oidc`; the first screen decides which doors to show from this |
| `account.oidcStart` | `server` (hostname), `issuer`, `clientId` | starts the sign-in at the server's identity provider: PKCE, a loopback listener for the redirect; returns `{url}` for the app to open in the browser. Ends in an `oidc.state` event: `done` with `name` (the provider's username, a suggestion) or `failed` with `error`. The ID token stays in memory and `account.create` with an empty `invite` presents it as the gate |
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
| `room.settings{roomId}` | | `{id, name, isDm, memberCount, notificationMode, admins[], isAdmin, slotServer, epochs, can{name, invite, admins}}` |
| `room.setSettings{roomId, name?, notificationMode?}` | | `name` renames via a policy event (admins); `notificationMode` (`all\|mentions\|mute\|default`) is kept on this device |
| `room.setAdmins{roomId, add[], remove[]}` | | usernames; admins only; a conversation keeps at least one admin |
| `attachment.send{roomId, path, caption?}` | | encrypts and uploads the file in chunks, sends the manifest; the item has `media.path` set locally, and `media.sizeLabel` for the pages that show a size. Own files can be deleted like own messages |
| `voice.send{roomId, path, duration, waveform[], caption?}` | | a voice message: the clip is sent like any file, with its length (`duration` in seconds from the recorder, kept in milliseconds) and its bars (`waveform`, 0 to 1) in the manifest, so every device draws the same bubble. The item's kind is `voice` and `media` carries `duration` (ms) and `waveform` |
| `doc.preview{roomId, eventId}` | | reads the document behind a file event (downloading it once): `{kind, title, blocks[], sheets[], html, pages, truncated, note}` plus, for a PDF, `rasterisable`, `pageCount`, `pageW`, `pageH` |
| `doc.thumb{roomId, eventId}` | | the bubble's preview: `{kind, title, pages, lines[{t:"p", text, level} \| {t:"row", cells[]}], imagePath}`; a PDF's first page is drawn to `imagePath` instead |
| `doc.page{roomId, eventId, index, width}` | | one PDF page drawn at `width` pixels: `{path, width, height}`; cached under the cache directory |
| `audio.info{roomId, eventId}` | | a track's `{artPath, accent, duration}` (ms); needs ffmpeg and ffprobe on the machine, and comes back empty without them |
| `message.pin{roomId, eventId}`, `message.unpin{roomId, eventId}` | | pins live in the conversation's policy (`pinned[]`) and anyone in it may change them; `pins.list{roomId}` → `{events[]}`, `pins.items{roomId}` → `{items[]}` (newest pin first); a `room.pinned{roomId, events[]}` event follows every change |
| `poll.create{roomId, question, options[], closed?, maxSelections?}` | | a kind-15 event; the item is kind `poll` with `poll{question, answers[{id,text,votes,mine}], voters, ended, maxSelections, disclosed}`. `closed` hides the numbers until the poll ends |
| `poll.vote{roomId, eventId, answers[]}` | | a kind-16 event referencing the poll; a person's latest vote replaces their earlier one; `poll.end{roomId, eventId}` is a kind-17 event only the asker may send |
| `thread.open{roomId, rootId}` | | `{key}` = `roomId\|thread:rootId`; a `timeline.reset` on that key follows with the root and its replies. Any `message.*` request may name the key as its `roomId`; a reply in a thread is a text event whose reference is `thread:<root>` (or `thread:<root>\|<replyTo>`), it carries `threadRoot`, stays out of the main timeline, and the root gains `threadSummary{count, body, sender, ts}`. `threads.list{roomId}` → `{threads[{rootId, sender, senderName, body, count, ts, latestBody}]}` |
| `stickers.list` | | `{stickers[{path, url, body, pack, width, height}], dir}`: local packs, one folder of images per pack under `dir`; `sticker.send{roomId, url, body, width, height}` sends the image as a file flagged `sticker`, shown as kind `sticker` |
| `contact.send{roomId, userId, displayName?}` or `{roomId, path}` | | a vCard file flagged `contact`; the item is kind `contact` with `contact{displayName, userId, cards}` once the file is here. `vcard.read{roomId, eventId}` → `{cards[]}`; `contacts.list`, `contacts.save{userId, displayName?}`, `contacts.remove{userId}` keep an address book on this device |
| `location.send{roomId, lat?, lon?, description?}` | | a kind-18 event (the device's fix when no coordinates are given): kind `location` with `location{geoUri, lat, lon, description, self}`. `location.startLive{roomId, durationMs?, lat?, lon?}` sends one with `until` and keeps updating it every 30 s while the share runs (`liveShare{live, expiresAt, lat, lon, updatedTs, ended}`, kind `liveLocation`); `location.stopLive{roomId}` ends it. `location.live{roomId, eventId, …}` events follow updates and ends. `location.map` is `unavailable`: no tile server learns where anyone is looking |
| `link.preview{url}` | | `{url, title, description, siteName, imagePath, imageWidth, imageHeight, isVideo}`, fetched by this device (direct, or through the SOCKS proxy set in `shape.settings`, never a proxy from the environment); `disabled` unless `shape.settings{linkPreviews: true}` |
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
| `message.send`, `message.reply`, `attachment.send` | | the item appears at once with `sendState: "sending"` and a `local:…` id, then is `set` to `sent` with its real id, or to `failed` with `sendError`. A failed item keeps `src` (the text as typed) or `media.path`, so `message.retry{roomId, eventId}` sends it again and `message.cancel{roomId, eventId}` drops it (a `remove` diff) |
| `emoji.render{text}` | | `{path, width, height}`: the emoji as a PNG cut from the device's colour emoji font (Noto Color Emoji, or `SIGIL_EMOJI_FONT`), made once per emoji; `unavailable` without such a font. Renderers that draw text from outlines have no colour emoji, so pictures stand in for reactions and the picker |
| `message.edit{roomId,eventId,body}` | | a kind-3 event referencing the message; own messages only; the item gains `isEdited` and its new body everywhere |
| `message.redact{roomId,eventId}` | | a kind-4 event; own messages only; receivers blank the item to kind `redacted` |
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
| `call.key{roomId}` | | `{key, epoch}`: the key call frames are sealed under, derived from the conversation's current MLS epoch (`kdf("sigil v1 call media", envelope_key)`), and the epoch number whose low byte names it in each frame. The app asks again when a frame arrives under a key id it lacks |
| `call.state` (event) | | pushed as `{roomId, callId, state: started\|ended, sender}` when a call event arrives or is sent; the timeline also gets a `call` item. The media stack (capture, encoding, playback, the WebRTC peer, the frame cipher) is the app's (`slint/src/call/`): it hands SDP in and gets SDP out, and keeps the QML `call` shape locally |

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
