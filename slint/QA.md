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
