# QA log — Slint vs QML 1:1 sweep (29 Aug)

Live findings; struck through when fixed and verified on device.

## Chat
- [ ] Inter-group bubble spacing ~3x the QML 10px — bubble.slint reserves
      reaction/pin lift unconditionally? Measure and match space(10)/space(3).
- [ ] Last message can sit behind the composer — list needs bottom padding
      (QML keeps the newest clear of the composer).
- [ ] Own-message sent/read mark not visible on latest own message — verify
      owns-receipt wiring end to end.
- [ ] Scroll fix built; verify on device once installed.
- [ ] Session day-labels: verify one renders after a >1h gap.

## Home
- [ ] DM avatars lack presence dots (Avatar has no presence ring — port it).
- [ ] Preview second line absent for rooms without lastMessage decryption; OK,
      matches engine data — recheck after backfill.

## Global
- [ ] Compare against QML panel side-by-side once desktop login completes.

## Critical
- [ ] Android hardware/gesture BACK exits the app (NativeActivity default)
      instead of go-back(). Intercept KEYCODE_BACK and route to nav.
- [ ] Notifications page: "Allow custom setting" toggle not derived from mode
      (shows ON while mode=default, no radio selected).
- [ ] Overflow menu card too translucent — bubbles ghost through (QML: 0.98).
- [ ] RoomSettings identity avatar: image never loaded (initials always).
- [ ] SpacePage: hierarchy list overlaps the hero (member chip half-hidden) —
      list geometry starts too high.
- [ ] Message sheet: action list renders EMPTY (prepare→sheet-actions not
      landing or not bound); reactions + spotlight fine.
- [ ] Quick-reaction pill: 2nd emoji tofu (likely U+2764 heart + VS16).
- [ ] Space hero "1 member" chip clipped behind list card.
