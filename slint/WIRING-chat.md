# WIRING — chat surface (chat.slint, bubble.slint, sheet.slint, emoji.slint)

## ChatPage (chat.slint)

Existing bindings kept: room-name, room-subtitle, room-encrypted,
room-initials, room-avatar, room-tint, items, typing-line; callbacks back,
send(string), composer-edited(string); functions scroll-to-end(),
clear-composer(). New functions: arm-reply(event-id, sender, body),
arm-edit(event-id, body).

### New `in` properties the bridge must fill
| property | source |
|---|---|
| typing-avatar / typing-initials / typing-tint | first entry of `room.typing` event's `users` |
| is-dm, is-invite | room object from `rooms.list` |
| is-thread | view key contains `\|thread:` (Service.roomOfKey/isThreadKey) |
| pagination-state | `timeline.paginationState` event: idle / paginating / timelineStart |
| in-call | Service.inCall (call.state ∈ joining/connected/reconnecting) |
| call-here | call.state.roomId == open room && inCall |
| sheet-actions | build in Rust when a menu opens — port of MessageSheet.actionsFor (see below) |
| emoji-rows | EmojiItem rows of 8 from /usr/share/omarchy/shell/plugins/emojis/emojis.json (fields e=glyph, k=keywords); refilter on emoji-search(q) against k |
| emoji-cat-rows | row index of first emoji of each category (find "👋","🐵","🍇","🌍","🎃","👓","🏧","🏁") |
| accent | chat theme accent when themes land; Theme.accent until then |
| bottom-inset | window kb-overlap (app.slint already computes it) |

### New callbacks → engine requests
| callback | request |
|---|---|
| send-reply(event-id, text) | `message.reply {roomId, eventId, body, markdown:true}` |
| send-edit(event-id, text) | `message.edit {roomId, eventId, body, markdown:true}` |
| start-call(video) | `call.start {roomId, video}` |
| join-call | `call.join {roomId, video:false}` |
| nav-requested(a) | app nav: search/threads/pins/chattheme/roomsettings (win.go(a)) |
| open-attach / open-recorder | show attach.slint / voicerec.slint (media group) |
| accept-invite / decline-invite | `room.join {roomIdOrAlias}` / `room.leave {roomId}` + back |
| paginate | `timeline.paginate {roomId, count:50}` — guard on pagination-state=="idle" bridge-side too |
| menu-action(action, event-id) | see sheet actions below; "prepare" fires when a menu opens (build sheet-actions then) |
| react(event-id, key) | `message.react {roomId, eventId, key}` |
| open-thread(root-id) | `thread.open {roomId, rootId, initialItems:60}` → nav "thread" with returned key |
| voice-toggled(event-id) | `audio.play {roomId, eventId, seek:0}` / `audio.stop` toggle; drive voice-playing + voice-frac rows via a 200ms tick (Service voicePos pattern) |
| voice-seeked(event-id, frac) | `audio.play {roomId, eventId, seek: frac * duration}` (duration from item media) |
| vote(event-id, option-id) | `poll.vote {roomId, eventId, answers:[optionId]}` |
| mark-read | `room.markRead` — fires when the list touches bottom |
| open-image/play-video/open-document/open-audio/open-location | viewer/doc/audio/map surfaces |

### Sheet actions (build in Rust, port of MessageSheet.actionsFor)
sending/failed: retry("retry"), copy, cancelsend(danger).
else: reply, forward, copy; +eventId: openthread|thread (thread-count>0 picks
openthread), pin/unpin("pin"); +can.edit && kind!=poll: edit (media kinds:
caption); +poll && !ended && can.redact: endpoll; +can.redact: redact(danger).
Sheet handles reply/edit/thread locally; the rest arrive via menu-action:
copy → wl-copy (desktop) / clipboard (Android: Slint clipboard API),
forward → nav "forward" with staged item, pin → `message.pin/unpin`,
redact → `message.redact`, retry → `message.retry {roomId, id, txnId}`,
cancelsend → `message.cancelSend`, endpoll → `poll.end`.

## Bubble rows (bridge fills TimelineRow)
- thumb: slint Image from media.thumbnailPath (fallback path); thumb-w/h from
  media.width/height. On `media.ready` patch the row (Service.applyMediaReady).
  Request via `media.get` when an image row has no thumbnailPath.
- waveform: resample media.waveform to ≤28 bars 0..1 (QML resampleWave).
- voice-playing/voice-frac: from the page's playback state, rebuilt per tick.
- duration: media.durationLabel or m:ss from media.duration.
- media-size: durationLabel · sizeLabel joined (file rows).
- reply/has-reply: item.replyTo. reactions: decorateReactions (mine = senders
  contains own userId). receipt-count: readBy minus self. owns-receipt: newest
  own message with eventId == receipt line owner (ChatPage.recomputeReceipt).
- pinned: room.pinned event list contains eventId.
- thread-count: threadSummary count (items carry threadSummary; also
  threads.list for the room's own timeline).
- poll-*: item.poll (question/options/votes/ended/multi).
- contact-*: item.contact or vcard.read result.
- location-label/live/ended: item.location.description / liveShare.
- is-read-marker: kind == readMarker (keep the row, zero content otherwise).
- day-label: sessionLabelFor (h:mm AP · Yesterday · weekday · d MMM) — note
  QML uses 12h session stamps, dayDivider uses dayLabelFor.

## Skipped / approximations (findings)
- Link-preview cards: not ported (needs link.preview cache plumbing) — v1
  renders the plain body.
- GIF inline playback: Slint has no AnimatedImage; stills only, no GIF badge.
- Code parts: TimelineRow needs `parts` ([{kind, body, lang}]) for the
  text/code/text split; v1 shows fenced code as plain body text. Syntax
  highlighting needs rich-text spans (engine html) — same gap as SigilText.
- SigilText effects: not drawn (known plan: engine-side line layout).
- HTML bodies: TextInput/Text render plain text only — body shows the plain
  `body`, not `html`. Links are not tappable yet (no rich text). This is the
  single biggest chat-fidelity gap vs QML.
- Sheet scrim: deep dim instead of blurred snapshot (no arbitrary blur).
- Enter-to-send: multi-line TextInput consumes Enter; send is the button
  (phone convention). Desktop Enter-to-send needs a key filter later.
- Read-receipt faces: QML stacks reader avatars with drop physics; v1 shows
  an eye + count. Needs readers array per receipt line to upgrade.
- Emoji rendering: femtovg colour-emoji support is partial — reactions may
  render monochrome on some platforms; verify on device.
- Message entry animation (rise from composer) and jump-flash ring: not
  ported in v1; ListView rows have no per-row entry hook — revisit with a
  custom timeline widget.

## Compile status
`cargo build` green with all four files + existing app.slint bindings intact.
