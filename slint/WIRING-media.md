# Wiring: media group

Ported from QML 1:1 unless noted. Engine names from core/docs/protocol.md +
Service.qml. "Rust computes" = the QML did it in JS; the bridge now owns it.

## ChatThemePage (pages/chattheme.slint) — replaces stub
Pending theme lives Rust-side (Panel.qml keeps it in chat-themes.json).
- in: room-name; pending-accent/pending-wallpaper (strings, mirror of pending);
  pend-accent (color resolved from pending-accent, Color.accent when "");
  wallpaper-img (image when wallpaper is a photo path); gradients ([GradPair],
  the 9 gradPair() pairs — Rust computes, HSL math); grad-sel (int, N of
  "grad:N"); custom-sel; pick-color + pick-strip-end (hsv(pick-h,s,v) and
  hsv(h,s,1) resolved Rust-side via pick-changed).
- callbacks: apply (Panel.setChatTheme + applied()), set-accent(s),
  set-wallpaper(s), reset-pending, choose-photo (platform file pick),
  pick-changed(h,s,v), accept-custom (hexOf(hsv) -> pending.accent), closed.
- DIVERGENCE: custom picker is hue/sat/value sliders, not a wheel — Slint has
  no conic gradient. Hover-only swatch rings are now pressed+selected states.

## DocPage (pages/doc.slint) — replaces stub
- in: file-name, status, error, size-label, kind, subtitle (Rust builds the
  QML's subtitle binding), blocks [DocBlock], pdf-pages, pages [DocPageImage]
  (lazily filled), page-aspect, sheets [SheetTab], sheet-rows [SheetRow] (rows
  of sheet-index), sheet-cols, toast.
- callbacks: download-requested (media.saveAs -> ~/Downloads), page-requested
  (idx, px-width) -> doc.page {roomId,eventId,index,width}, sheet-picked(i),
  closed. Initial load: doc.preview {roomId,eventId} on open.
- Structs DocBlock/SheetRow/SheetTab/DocPageImage exported from this file;
  re-export through app.slint for Rust.

## AudioPage (pages/audio.slint) — replaces stub
- in: title, size-label, status, art (image) + have-art, tone (audio.info
  accent), duration/position (s), playing, toast, can-download.
- callbacks: toggle-requested (audio.play/audio.stop), seek-requested(s)
  (audio.play {seek}), download-requested (media.saveAs), closed.
- Data: audio.info {roomId,eventId} -> artPath/accent/duration/waveform.
- DIVERGENCE: playing "breathe" scale is an opacity nudge (no scale transform
  on the clipped art box); QML gradient stage kept via @linear-gradient.

## ImageViewer (viewer.slint) — NEW, not yet imported by app.slint
- in: items [ViewerItem] (Rust builds from the room timeline: image/video
  kinds, fmtTs ts-label, full image via media.get with thumbnail fallback),
  cur, forward-names (first ~8 room names), toast, playing-event, video-frame
  (image; engine shm -> slint::Image per frame), play-pos/play-duration,
  scrubbing.
- callbacks: page-changed(i) (fetch full via media.get), download-requested,
  delete-requested (message.redact + close), share-requested (wl-copy on
  desktop; Android intent later), forward-to(i) (attachment.send of cached
  path to that room), react(emoji) (message.react), toggle-playback
  (video.play/video.stop), seek-requested(s) (video.seek), scrub-state(on,pos),
  closed.
- GESTURE FINDINGS (the toolkit evidence this branch exists to gather):
  - PINCH IS NOT EXPRESSIBLE: Slint TouchArea is single-pointer. Zoom rides
    double-tap, ctrl+wheel (or wheel while zoomed), and two on-screen +/−
    buttons as the touch fallback. zoomAbout() math is the QML's verbatim.
  - No blurred-backdrop scrim (no backdrop shader); solid near-black.
  - No thumbnail-morph open animation (needs cross-page geometry mapping);
    plain fade would need the holder's opacity animated by the integrator.
  - Swipe paging is drag-follow + threshold release, not a physics filmstrip.

## AttachMenu (attach.slint) — NEW, not yet imported
- in: stickers [StickerItem] (art loaded Rust-side), stickers-loaded.
- callbacks: pick-files, open-location(mode) — integrator hosts the maps
  group's LocationPicker (pin|current|live heights 430px in QML),
  insert-emoji(s), create-poll(question, options[8 slots, blanks filtered
  Rust-side], closed) -> poll.create, send-sticker(i) -> sticker.send,
  load-stickers -> stickers.list, load-emoji, close-requested.
- The emoji page reserves an @children slot: mount EmojiPicker there.
- DIVERGENCE: poll options are 8 fixed slots gated by option-count (Slint
  models are immutable from the language); per-option remove buttons dropped —
  blank options are simply skipped.

## VoiceRecorder (voicerec.slint) — NEW, not yet imported
- in: state (idle|recording|ready — Rust flips around voice.start/stop),
  elapsed (Rust ticks 100ms while recording), clip-duration, levels (rolling
  60 of voice.level events), clip-waveform (voice.stop reply).
- callbacks: record -> voice.start; stop -> voice.stop (keep path/duration/
  waveform Rust-side for attach); restart -> voice.cancel + voice.start;
  attach -> voice.send {roomId,path,duration,waveform,caption}; cancelled ->
  voice.cancel when recording, then close the drawer.

## model.slint additions wanted (none blocking; all structs local for now)
- Consider promoting ViewerItem, StickerItem, GradPair, DocBlock/SheetRow/
  SheetTab/DocPageImage into model.slint when wiring, so Rust sees one set.
