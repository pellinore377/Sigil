# Upstream: Flickable + touch kills delayed press forwarding (no long-press)

Slint 1.17.1, android-activity backend (also reproducible with any input
source that streams stationary Moved events while a finger is down).

## Symptom

A `TouchArea` inside a `Flickable` never receives `PointerEventKind.down`
while a finger is held on it. A quick tap delivers `down` and `up`
back-to-back at release; a hold longer than the flick threshold delivers
nothing at all. Timer-based long-press detection (500ms hold to open a
context sheet — this app's message sheet) is therefore impossible inside any
scrolling list on touch devices, while the same code works with a mouse.

## Cause

On `Pressed`, `Flickable`'s input filter returns
`DelayForwarding(FORWARD_DELAY = 100ms)`; `process_mouse_input` parks the
press with a one-shot timer that would forward it to children
(`input.rs`, `result.delayed`).

Touch input streams `Moved` events (hardware jitter or repeated identical
coordinates) from the moment of contact. Each `Moved` runs a fresh dispatch;
for a stationary move the Flickable filter returns
`ForwardAndInterceptGrab`, the move is accepted, and the freshly-built
`MouseInputState` **replaces** the previous one. The previous state owned the
`delayed` (timer, press) pair — dropping it cancels the timer, so the press
is never forwarded. A mouse held still produces no `Moved` events, which is
why the bug is touch-only.

## Interim fix (vendored)

`vendor/i-slint-core/input.rs`, in `process_mouse_input`: keep the pending
state when the incoming event is `Moved` and a delayed press exists (marked
`SIGIL PATCH`). Large moves still start the flick — the filter intercepts on
a subsequent event once the delayed press has been flushed by its timer —
and a quick tap keeps its flush-on-release behaviour.

Wired via `[patch.crates-io]` in `slint/Cargo.toml`. Drop the vendor
directory and the patch section when upstream resolves this.

## Repro used

`debug()` prints in the row's press `TouchArea` `pointer-event`; on device:
`adb shell input tap X Y` → `down`,`up` together at release;
`adb shell input swipe X Y X Y 1200` (stationary hold) → no events at all.
