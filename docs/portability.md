# Writing Sigil so it runs everywhere

Sigil targets **Linux, Android, macOS, Windows, iOS and Web**, and is meant to
be given away free. Android is next, because that is the phone in the author's
pocket.

That is not a distant aspiration to be retrofitted. It is a constraint on every
UI element written from here on: **if a control only works on one platform's
input model, it is not finished.**

## The rule that produced this document

Panning the map was built on a `DragHandler` that held a *contested exclusive
pointer grab* and stole it from other items. It worked on the author's machine
until it didn't, and days went into hunting what was cancelling the grab — a
virtual wheel device, a gesture-aware pointer, something in the compositor.

That was the wrong question. An exclusive grab can be revoked by a touchscreen,
a stylus, a gesture-aware touchpad, a compositor that drops pointer focus, or a
platform that owns drags outright (Android, iOS, web). The trigger on one
machine was never the bug. **Building interaction on holding a contested grab
was the bug**, and it would have failed on every touch platform we ship to.

The fix was to stop needing the grab: a `PointHandler` takes only a *passive*
grab, reports where a pressed point is, and never asks to own it. Nothing to
steal, nothing to cancel — and as a bonus, sampling the point once per frame
deleted the 700-events-per-second rounding workaround too.

**Generalised: prefer the mechanism that cannot fail over the mechanism that
usually works. When a bug looks environment-specific, ask what the design
assumed about the environment before hunting the trigger.**

## Input: design for the union, not the intersection

| | desktop | touch (Android/iOS) | web |
|---|---|---|---|
| hover | yes | **never** | yes, absent on touchscreens |
| right-click | yes | **never** | yes, often intercepted |
| wheel | yes | **never** | yes |
| long-press | rare | primary secondary action | yes |
| cursor shape | yes | meaningless | yes |
| precise pointer | yes | ~44px minimum target | mixed |

Rules:

- **Hover may reveal, never gate.** Anything reachable *only* by hovering does
  not exist on a phone. Hover is allowed to make an affordance prettier or
  bring it forward; it must not be the only way to find it.
- **Every right-click needs a long-press twin**, wired to the same handler.
  `TapHandler { longPressThreshold: 0.5; onLongPressed: ... }` — note Qt 6 uses
  `onLongPressed`, *not* `onPressAndHold` (grepping for the wrong one will tell
  you long-press is missing when it isn't).
- **Every wheel interaction needs a touch equivalent** — flick for scroll,
  pinch for zoom.
- **Tap targets ≥ 44×44** device-independent pixels for anything a finger uses.
- **Never rely on an exclusive pointer grab.** Passive tracking, or a plain
  `MouseArea` you own outright, or a `Flickable`. If a handler needs
  `grabPermissions` tuned to steal from other items, the layout is wrong —
  give the handlers their own geometry instead.

## Platform surface: keep the non-portable parts thin

Current coupling, measured:

| layer | files | portable? |
|---|---|---|
| `QtQuick`, `QtQuick.Controls`, `QtQuick.Effects` | 62 | yes, everywhere Qt runs |
| `qs.Commons` / `qs.Ui` (host design tokens) | 59 / 52 | **shim needed** — but tiny surface |
| `Quickshell*` (PanelWindow, Wayland, Io, Hyprland) | 17 | **Linux/Wayland only** |
| `QtLocation` / `QtPositioning` | 1 | availability varies by platform |

The good news is that the token surface is small. Everything Sigil asks of the
host shell is:

- `Style.space(px)`, `Style.spaceReal(px)`, `Style.spacing.*`
- `Style.font.family` and `caption | bodySmall | body | subtitle | title | heading`
- `Color.accent | background | foreground | muted | urgent`,
  `Color.menu.{text,background,border}`, `Color.popups.{text,background,border}`
- `Util.alpha(color, a)`

That is roughly fourteen colour tokens and a handful of metrics. **Porting the
look is a small job; porting the shell integration is the real work.** Keep it
that way: never reach past these tokens into host internals.

`Style.space()` multiplies by the host's spacing scale, which is how Sigil gets
DPI independence for free today. Any port must supply the same contract, or
every dimension in the app is wrong at once.

## What is Linux-only today, and what replaces it

| used now | why it's Linux-only | needs, per platform |
|---|---|---|
| `PanelWindow`, `WlrLayershell` | wlroots layer shell | a real window / Activity / view |
| `IpcHandler` (`sigilui`, `sigil`) | Quickshell IPC | test hooks need another transport |
| `Process` spawning `omarchy-file-select` | Linux binary | SAF on Android, `UIDocumentPicker` on iOS, `<input type=file>` on web |
| `notify-send` | freedesktop | platform notification APIs |
| unix socket to `sigil-engine` | AF_UNIX | in-process/JNI on Android, WASM or WebSocket on web |
| POSIX shm video frames (`video/`) | `shm_open`, `mmap` | platform texture path; no shm on Android/iOS/web |
| Nerd Font glyphs | font installed system-wide | **must be bundled** — see below |

The engine is Rust and already isolates Matrix state behind a line protocol,
which is the right shape. The **transport** is what changes per platform, not
the protocol.

## Icons: the largest single portability debt

**219 Nerd Font glyphs across 47 files**, drawn as private-use codepoints in
string literals (`"󰁍"`, `"󰍎"`, …) with `font.family: "CaskaydiaMono Nerd Font"`.

On Linux this works because the font is installed. On macOS, Windows, Android,
iOS and web it renders as tofu — every icon in the app, blank.

Options, in order of preference:

1. **Bundle the font** as a Qt resource and reference it by the family name the
   bundled file declares. Cheapest, keeps every call site as-is. Check the
   licence permits redistribution.
2. Replace with an `Icon.qml` component mapping semantic names (`Icon.back`,
   `Icon.pin`) to glyphs or SVGs. More work, but decouples the call sites and
   makes swapping icon sets possible later.

Do **not** ship without resolving this. It is invisible on the dev machine and
total on every other platform.

## Order of work for the Android port

1. Bundle the icon font. Nothing else is judgeable until icons render.
2. Provide `qs.Commons`/`qs.Ui` equivalents so `Style` and `Color` resolve.
   `mobile/` is ~12,400 lines and is already phone-shaped; `components/` is
   largely desktop and can wait.
3. Replace the Quickshell window host with an Android-appropriate root.
4. Give the engine an in-process transport; drop the unix socket.
5. Audit input: long-press twins, tap-target sizes, flick surfaces.
6. Calls and video last — the shm frame path needs a full replacement.

---

# Audit: readiness as of 2026-08-27

Measured against the current tree (62 QML files, ~19,000 lines).

## Healthy

- **Sizing is DPI-independent.** 1566 `Style.space()` calls and 765
  `Style.font.*` references against 20 bare numbers. Supply the token contract
  on a new platform and every dimension is already correct.
- **The design-token surface is tiny** — ~14 colour tokens plus metrics. The
  look ports cheaply.
- **`mobile/` is already phone-shaped**: 30 files, ~12,400 lines, versus 2,274
  lines of desktop `components/`. Android inherits the bulk of the real UI, and
  only four `components/` files are reached from it (`Avatar`, `EmojiPicker`,
  `ScrollBarStyle`, `Spinner`).
- **Touch input is in better shape than expected.** Exactly one hover-gated
  affordance file in `mobile/`, and the message menu already has a long-press
  twin alongside its right-click.
- **Quickshell coupling is concentrated**, not smeared: 17 of 62 files.

## Blocking for Android

1. ~~**Icon font — 219 glyphs across 47 files.**~~ **Fixed 2026-08-27.** All 219
   literals now go through the `Icons` singleton (104 named icons), and Roboto,
   Roboto Mono and Material Symbols Rounded are bundled in `fonts/` (~1.5 MB,
   all Apache-2.0) and loaded by the `components/Fonts.qml` singleton. Nothing
   asks the system for a font any more.

   **Made portable 2026-08-28.** The table itself is now `shared/icons.json`;
   `shared/icongen` generates `components/Icons.qml` from it, and a Swift,
   Kotlin or TypeScript emitter is one more function in that crate.

   Two traps worth knowing, both of which bit during the switch:
   - QML's `font` grouped property exposes `family`, **not** `families`. There
     is no per-character fallback, so the font is chosen per element and a
     string mixing an icon with words must be split into two Text elements.
   - A Text that should draw an icon but gets the *text* font does not show
     tofu — the missing PUA codepoints fall back to whatever system font has
     them, which on a developer machine is often a Nerd Font, so it silently
     draws a **different icon**. Grep for `text:` bindings that reach an icon
     through a model field (`modelData.icon`) — those are the ones a
     literal-matching pass misses.
2. **Platform shell layer.** `PanelWindow`/`WlrLayershell` root, `IpcHandler`
   test hooks, `Process`-spawned `omarchy-file-select`, `notify-send`, and the
   AF_UNIX socket to `sigil-engine` all need Android equivalents. The engine's
   line protocol is the right shape; only the transport changes.

## Gaps worth fixing before the port, while the code is fresh

3. ~~**`components/ImageViewer.qml` has no pinch-zoom and no drag-pan.**~~
   **Fixed 2026-08-27.** Pinch, double-tap (fit ⇄ 2.5x at the tapped point) and
   ctrl+wheel all drive the same two numbers; panning uses a passive
   `PointHandler` for the reasons above. The pager stops being interactive while
   magnified so a sideways drag pans the picture instead of turning the page.
   Verified against the transform maths with `sigilui viewerZoom`.
4. **`mobile/ChatThemePage.qml`** reveals two swatch affordances on hover only
   (lines ~327 and ~354). Invisible on touch.
5. **Calls/video.** The `video/` C++ plugin reads POSIX shm frames written by
   the engine. There is no shm on Android, iOS or web; this needs a different
   texture path entirely. Deliberately last in the work order.

## Not yet assessed

- macOS/Windows: expected to be easier than Android (real windows, pointer
  input, Qt widely supported) but the icon font and shell layer apply equally.
- Web: the largest unknown. Qt for WebAssembly would need the engine as WASM or
  behind a WebSocket, and the whole shm video path is unavailable.
- **There is currently no way to test on any platform but the author's Linux
  desktop.** That is the biggest process gap, and it is why a single machine's
  input quirk was able to consume days: with no second device, machine-specific
  and design-level faults are indistinguishable.
