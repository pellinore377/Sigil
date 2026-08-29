# QA log — Slint vs QML 1:1 sweep (29 Aug)

Live findings; struck through when fixed and verified on device.

## Chat
- [ ] Inter-group bubble spacing ~3x the QML 10px — bubble.slint reserves
      reaction/pin lift unconditionally? Measure and match space(10)/space(3).
- [ ] Last message can sit behind the composer — list needs bottom padding
      (QML keeps the newest clear of the composer).
- [ ] Own-message sent/read mark not visible on latest own message — verify
      owns-receipt wiring end to end.
- [x] Root cause: ListView never pans on touch (probe-proven); TouchList everywhere. Verify on device.
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
