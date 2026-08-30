# QA log — Slint vs QML 1:1 sweep (29 Aug)

Live findings; struck through when fixed and verified on device.

## Chat
- [ ] Inter-group bubble spacing ~3x the QML 10px — bubble.slint reserves
      reaction/pin lift unconditionally? Measure and match space(10)/space(3).
- [x] Opens at bottom; newest clear of composer (verified).
- [x] Read tick renders on newest own message (verified on device).
- [x] SCROLLING FIXED & VERIFIED: std ListView never pans on touch (probe);
      TouchList (Flickable) everywhere; opens at bottom via layout-settle
      rescroll; verified both directions on device.
- [ ] Session day-labels: verify one renders after a >1h gap.

## Home
- [x] Presence dots ported (Avatar badge + home DMs + members).
- [ ] Preview second line absent for rooms without lastMessage decryption; OK,
      matches engine data — recheck after backfill.

## Global
- [ ] Compare against QML panel side-by-side once desktop login completes.

## Critical
- [x] Back routed via close-request → go-back(); leaves app only from Home. VERIFY on device.
- [x] Notifications mode convention fixed ("" = account default).
- [x] Overflow menu opaque.
- [x] RoomSettings avatar image loads.
- [x] Space hero height from layout padding.
- [x] Sheet actions: one-time assignment replaced with a binding.
- [x] Heart without VS16.
- [ ] Space hero "1 member" chip clipped behind list card.

## Verified on device (fix build, 29 Aug afternoon)
- Scroll both directions; opens at bottom; session labels; reply quotes;
  reactions; read tick; image bubbles with captions; action sheet complete
  (reply/forward/copy/pin/thread; correct failed-send variant); red heart.
- Frosted Glass palette matches the desktop QML panel.

## Open items (next session)
- [ ] Long-press on media bubbles (inner tap TouchAreas eat the press;
      QML opens the menu from media too).
- [!] Android back: UPSTREAM. Slint delivers the back key as neither a
      close-request nor any UI key (FocusScope with focus receives nothing).
      Interim: the activity exits; session restore is fast. File with a
      key-logging repro; the fix belongs in i-slint-backend-android-activity.
- [ ] Bubble inter-group spacing: measure against QML with same-density
      fixture (earlier read may have been correct behavior).
- [ ] Settings identity avatar image (fix landed; verify).
- [ ] GIF playback, rich text bodies, link previews (structural, in readout).

## Verified on device (evening run, 29 Aug — build fdaa767)
- Pin marker renders on the pinned message (pins.list reply now handled).
- 🎯 reaction from the desktop QML session shows cross-device.
- Doc preview card: white page, text lines, TEXT chip, correct height.
- Attach menu: bottom drawer, opaque chrome, all seven tiles
  (Files/Emojis/Stickers/Poll/Current Location/Live Location/Drop a Pin),
  scrim over the timeline. Matches the QML drawer.
- Voice recorder: QML-style bottom card — "Tap to record your voice",
  Cancel · mic · Attach row. Replaced the full-screen overlay.
- Keyboard inset: composer rises above the IME; content height shrinks.
- History state events render as centered notices (power levels, history
  visibility, avatar changes); location cards + "Live location ended";
  scroll-to-bottom FAB appears when scrolled up.
- Header "1 members" is verbatim QML (ChatPage.qml:1012 concatenates
  joinedMembers + " members" with no singular form) — kept 1:1.
- Cargo.toml: lto = "thin" restored for shipping; final LTO build validated.

## Still open (deferred, in the readout)
- [ ] Thread-count chip on the thread root not yet sighted on device
      (bridge reads threadSummary.count; field confirmed in core — verify
      with a fresh follow dump of the root item).
- [ ] Threads page / Pins page on-device pass with the seeded fixtures.
- [ ] Image viewer open/zoom/download; search page; forward flow; theme apply.
- [ ] Composer-insert for emoji picker; Android SAF file pick; clipboard JNI.

## End of run (29 Aug ~5pm)
- Threads page verified on device: "Sigil Test · 2 threads", avatar rows,
  bold reply count + root body + timestamp. Thread view verified: root with
  pin marker and 🎯 reaction, thread reply, lock glyph, read tick.
- Home header renders the real account avatar image (open item closed).
- SESSION INVALIDATED BY SERVER mid-run: MAS rejected the phone's OAuth
  refresh token ("invalid_grant" after ~24h suspended). The engine emitted
  the invalidation and the app dropped to the login page — correct behavior,
  same as QML. Phone needs a fresh SSO sign-in; further authenticated
  on-device QA blocked until then.
- Pins page: not reached before the invalidation — first open item for the
  next signed-in session.

## Parity audit sweep (29 Aug evening)
Three source-level audits diffed every QML mobile page against its Slint
counterpart. ~60 deviations fixed across 8 commits:
- Bubble: press-area moved BENEATH the bodies (inner taps were dead),
  fullBleed set widened (doc/location/contact/audio), 40px width floor,
  +22 text padding, jump-flash ring, thread-chip latest-reply preview,
  reaction scale, MarkStack reader faces replace the eye+count, details
  row on received messages, PollBody ported whole (own card, pick dot,
  percentages, hidden-results footer, retract/multi-select voting),
  DocThumb paper/type sizes/badge, location pin-only fallback + live pill,
  AudioBody card (cover + palette strip via audio.info).
- Chat: convoC ÷1.35 with the accent mix gated on a set theme, composer
  tint band + QML geometry (56/38/r19), quote bar 52/inset-5/close-inside,
  caption composer mode (Add/Edit caption through message.editCaption),
  jump-to-latest centred at ¾-screen, 8px list end spacers, BottomToTop
  short-conversation alignment, composer hidden on invites, whitespace
  send guard, attach + → cancel affordance with the 90° turn, DM presence
  dot on the header avatar, ⋮ menu 170/34/no-border.
- Sheet: pill clamps with its own width, VS16 heart key restored (bare
  glyph still displayed — femtovg tofu), card height double-count fixed,
  confirm dims instead of hiding the sheet, 150px labels, caption/thread
  actionsFor variants, QML entry order.
- Home: spaces hero + rounded-square space rows with counts + chevron,
  space-filter chip, drafts ("Draft:" in red italics, banked per room),
  scroll-to-top button, QML header/search/tabs/badge/FAB/account-menu
  metrics, top-level-space filter, own presence dot.
- Recorder rebuilt to VoiceRecorder.qml (inline 230px panel under the
  composer, 7-bar idle wave, blink dot + zero-padded timer, restart-take,
  120px record pill, attach-while-recording).
- Attach: QML margins, centred grid with left-aligned second row, chip
  poll fields + remove buttons + trimmed create, sticker flow packing.
- Pages: roles exact-level guard, security dirty seeding, addpeople
  2-char debounce + presence, members ambiguity rule, roomsettings member
  presence + conditional spacers, threads 64px rows, pins 6-line clamp +
  width floor, search spacing/viewport, viewer letterbox-tap close +
  22px actions, chattheme wallpaper branch + corner radii + HSV re-seed.
- Shared: two-letter initials, initials 0.42 with the 8px floor,
  transparent presence pad riding 2% out, SettingsHeader 22px back
  button (QML PanelActionButton), sublabel two-line clamp, OverflowMenu
  QML card metrics.
Verified visually via the desktop demo build (SIGIL_SLINT_DEMO): home
list (two-letter initials, badges) and chat (bottom-aligned timeline,
composer band, ground tone, sheen) render correctly.

## Deliberate skips (no Slint equivalent / out of scope, commented in-file)
- Entry/hover animations needing element scale; FrameAnimation glides.
- Filmstrip page-peek in the viewer; live-location countdown clock.
- Staged-attachment composer row (files/contact/voice chips) — needs the
  Android SAF seam first; "Open as window" header button (desktop-only).
- Slint trim() gaps: send guarded in Rust; two caption gates use
  character-count without trimming.

## Second evening round (29 Aug, after the audit sweep)
- Voice clips now STAGE in the composer like the QML (light chip inset in
  the pill: play/pause preview via audio.playFile, waveform, duration,
  discard ×; placeholder flips to "Add text"; send posts the clip with the
  typed caption). voice-attach no longer fires the send directly.
- Viewer filmstrip: the neighbouring image peeks in during a drag
  (SnapOneItem look, 12px page gap).
- 120–130ms opacity fades on the overflow menu, the Home account menu and
  the message sheet (the QML's scale-in has no Slint equivalent; the fade
  carries the motion).
- Live-location pill counts down (m:ss) from liveShare.expiresAt, anchored
  to the wall clock via boot-epoch-s + animation-tick().
- Desktop demo re-verified after the round: chat renders unchanged.

## Remaining gaps, all structural or platform (nothing further portable)
- Toolkit: rich text, GIF playback, pinch zoom, element scale animations,
  ElideMiddle, backdrop blur (decision evidence, in the readout).
- Platform seams: Android SAF file picking (blocks staged FILE chips and
  the contact chip), share intents, notifications, hardware back
  (upstream android-activity gap).

## Third round (29 Aug): the last portable bubble features
- Fenced-code messages render as parts: text runs + a #242428 code ground
  with mono text and the language tag (the engine's `parts`; highlight
  colours flattened — they need rich text). Demo-verified: full-bleed
  card, entities decoded, grouped corners.
- Link preview cards: og image (aspect-fit, 340px cap, play disc for
  video), title/description two-line clamps, favicon-letter + domain row
  over the page-accent ground; a body that IS the link renders the card
  alone (cardOnly). Tap opens the browser; long-press opens the sheet.
  Card data via link.preview, cached per url.
- Bubble contentW now pinned to bubbleMax for card kinds (QML:381) —
  wrapped text reports a tiny preferred width in Slint.

## Fixture verification (29 Aug, desktop demo)
The demo timeline now seeds every bubble kind (poll, voice, live location,
contact, audio, receipts/reactions/thread, failed reply, code parts) —
`SIGIL_SLINT_DEMO=1 SIGIL_SLINT_DEMO_CHAT=1` renders them without a login.
Verified: PollBody card (pick dots, leading bar, `2  67%` tallies,
"3 votes"), voice row (white disc, wave, 0:07), MarkStack reader faces,
thread chip with latest-body preview, failed-send mark on a reply, code
card with lang tag, audio strip. Fixed en route: the contact card wore
the SENDER's initials/tint instead of the contact's.
Still to eyeball on a signed-in phone: reaction chips (❤ next to 👍),
the live-pill countdown (cut off in the landscape demo window).

## Fourth round (29 Aug evening): two "structural" gaps closed
- transform-scale-x/-y + transform-origin EXIST in Slint 1.17 (undocumented
  in our earlier survey) — every QML scale animation is now ported: the
  overflow/account menus scale from the top-right, the sheet pill scales in
  on X from 0.4 and the menu on Y, the scroll-to-top button pops with the
  OutBack overshoot, attach tiles grow 1.05 on hover.
- GIF playback: new engine op `media.gifFrames` decodes an animated GIF
  into ≤64 frame PNGs (480px cap, per-frame delays, cached beside the
  media); the bubble cycles them on a per-frame Timer — QML's
  AnimatedImage, materialized. Decode path validated against a 3-frame
  rig-built GIF (frames + 400ms delays correct); the on-device round-trip
  needs the signed-in session. Viewer still shows the still frame.
- Structural list now: rich text, pinch (single-pointer input, upstream),
  backdrop blur, ElideMiddle. That is the complete remainder.
- GIFs animate in the image viewer as well (same frame strip).
