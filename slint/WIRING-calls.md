# Wiring: calls & maps group

Files: `slint/ui/callpage.slint` (CallPage + exported CallTile, SpeakingRipple,
Floater struct), `slint/ui/callpip.slint` (CallPip), `slint/ui/callbanner.slint`
(CallBanner), `slint/ui/pages/map.slint` (MapPage + LocationPicker).
The three call files are NOT yet imported by app.slint — the integrator mounts
them (CallPage as an overlay page, CallPip floating at content level, CallBanner
at the top of the content area when `call.incoming` is set).

The page renders; the BRIDGE decides: Slint expressions cannot scan arrays, so
everything Service.qml/CallPage.qml computed by iterating arrives pre-digested.

## CallPage (callpage.slint)

Fill from every `call.state` push:
- `state` — call.state.state (idle|joining|connected|reconnecting|leaving).
- `status-text` — port of CallPage.statusText: "Calling…"/"Reconnecting…"/
  "Ending…"; connected with no remotes → "Ringing…"; else the running duration
  from `call.since`, ticked ONCE PER SECOND bridge-side (the QML `now` timer).
- `error`, `encrypted` — call.state fields.
- `mic-muted` / `camera-on` / `screen-sharing` — call.state.local.
- `tiles: [CallParticipant]` — gridTiles port: every remote with its BEST live
  track (screen first, else camera) as `frame`, SELF LAST ("You",
  own avatar/level). `has-frame` ONLY when the track is live: a muted camera
  stays published in LiveKit; hasVideo must go false (ParticipantTile.trackLive).
- `spotlight-idx` — index into `tiles` of the screen-share tile, -1 if none.
- `featured` + `has-featured` — CallPage.featured port (remote screen > remote
  camera; a camera-muted participant stays featured so the layout doesn't flip).
- `local-cam` + `has-local-cam` — self camera track for the 1:1 PiP.
- `group-mode` — remotes.length > 1 || any screen share.
- `video-mode` — has-featured || has-local-cam.
- `peer-*` — first remote, else the room (voice layout identity).
- `mics` / `speakers` — call.devices → DeviceRow {id, label:name,
  selected: selected[kind]==id || (selected empty && default)}. Refresh via
  `call.devices` when settings-open flips true, and ~600ms after select-device
  (the QML devRefresh timer).
- `floaters: [Floater]` — call.reaction events append {fid: seq, emoji, who
  (empty for own)}; prune each after ~2.6s and cap at 12. Delegates animate
  themselves from birth via animation-tick; the bridge only appends/prunes.
  The QML also plays a sound for others' reactions via pw-play — desktop-only
  seam, not wired.

Video frames: `CallParticipant.frame` is a slint Image the bridge rebuilds from
the engine's shm files (`call.state.participants[].tracks[].shmPath`, layout in
video/omv_shm.h) — in-process read → SharedPixelBuffer per frame tick. Mirroring
the local camera (QML mirrors self-view) must happen bridge-side when converting
the buffer; Slint has no horizontal flip.

Callbacks → requests:
- set-mic(muted) → `call.mute {muted}` (page passes current mic-muted, i.e. the
  QML's callSetMic(micMuted) toggle convention)
- set-camera(on) → `call.camera {enabled}`
- set-screenshare(on) → `call.screenshare {enabled}` (the QML fires
  beforeScreenshare() first to hide the panel for the portal picker — desktop
  seam for the integrator)
- hang-up → `call.leave` + navigate back
- select-device(kind, id) → `call.setDevice {kind, id_}` (note the underscore)
- react(emoji) → `call.react {emoji}`
- minimize → shell: hide CallPage, show CallPip

## CallPip (callpip.slint)
- `featured` — same featured pick as the page (screen > camera, else first
  remote as avatar tile). `duration-label` — m:ss from call.since, 1s tick;
  "•" before connect. `parent-width/height` — the host content area (a Slint
  child cannot read its parent's size).
- expand-requested → show CallPage; hangup-requested → `call.leave`.
- Mount when inCall && CallPage not open (Service's CallPill condition).

## CallBanner (callbanner.slint)
- From `call.incoming`: caller-name/-initials/-tint(+avatar if cached),
  room-name, video-intent (incoming.intent == "video").
- accept → `call.join {roomId, video:false}` + open that room (Service
  openRoomAfterAccept); accept-video → same with video:true; decline →
  `call.decline {roomId}`.

## MapPage (pages/map.slint)
Digest of the viewed timeline item (MapPage.qml locals):
- `who` ("You"/senderName), `own`, `live` (liveShare.live), `ended` (liveLocation
  && !live), `stoppable` (own && live && Service.liveSharing — the location.live
  event), `openable` (pin marker || someone else's running share).
- `status-text`: ended → "Live location ended"; live → "Sharing until h:mm AP"
  (expiresAt); else "Shared h:mm AP" (item ts).
- `remaining-label`: expiresAt − now as "1h 05m" / "m:ss", 1s tick while live.
- `lat`/`lon`/`description` from item.location; `self-marker` = liveLocation ||
  location.asset == "m.self"; sender avatar/initials/tint.
- Callbacks: stop-live → `location.stopLive`; open-osm(url) →
  platform::open_url (the page builds the openstreetmap.org link).

## LocationPicker (pages/map.slint)
- `mode` current|live|pin; `have-fix`/`fix-lat`/`fix-lon`/`position-error` from
  the `position` event; my avatar identity; retry-position → `position.refresh`.
- share-requested(lat, lon, durationMs) → durationMs > 0 ?
  `location.startLive {roomId, durationMs}` : `location.send {roomId, lat, lon,
  selfLocation: mode != "pin"}` (the picker shares position, the send call
  decides pin vs self by mode — Service.sendLocation).
- Mount inside AttachMenu's location slot (attach.slint reserves it).

## Toolkit findings (the evidence this branch gathers)
- NO MAP RENDERER: QtLocation/MapLibre has no Slint counterpart. MapPage and
  the picker render real data on static cards and say so. Restoring maps means
  either a Slint map widget upstream or engine-side tile rasterisation to an
  Image (the engine already owns the style URL logic).
- Pin mode is inert without a tappable map (stated in the UI).
- No shader ripple on the live marker: approximated with a sine-driven ring.
- PiP throw physics (velocity-tracked spring with bounce) is not expressible
  without a per-frame callback; both PiPs ease to the nearest corner instead.
- SpeakingRipple no longer breathes with the live audio level: CallParticipant
  carries no `level` field. Adding `level: float` (and, for completeness,
  `quality: string` for the signalOff badge) to the struct restores QML parity.
