# Sigil for Slint

The cross-platform frontend: one UI, in Slint, over the same engine as every
other frontend — built to decide whether Slint *becomes* Sigil's UI toolkit.
Android is the proving ground because it is the platform QML cannot follow us
to; the same crate runs on the desktop for fast iteration.

**Status: a test case, on the `slint` branch.** Login, room list and a text
timeline against a real homeserver. No calls, no media bodies, no SigilText
effects yet.

## Architecture

There is no daemon here. The engine (`../core`) is a path dependency, hosted on
a tokio runtime inside the app:

```
  ui/*.slint  ──callbacks──▶  src/bridge.rs  ──Request──▶  sigil_engine::Engine
  (view only)  ◀──models───   (UI thread)     ◀──Hub────   (tokio, in-process)
```

The JSON protocol is byte-for-byte the socket protocol (`core/docs/protocol.md`)
— `bridge.rs` replaces the transport, not the contract. `timeline.diff` ops
apply to a shadow `Vec<Value>` on the UI thread, which then projects to the
Slint model; the shadow keeps *every* item because filtering the list the
diff indices point into is how views desynchronise (docs/ui-conventions.md).

Design tokens (`ui/style.slint`) mirror the qs.Commons contract from
docs/portability.md — same names, same roles. One deliberate difference:
there is no `Style.space()`. Slint's `px` is already a logical pixel scaled by
the device pixel ratio, which is the entire job `Style.space()` did on
Quickshell. A design px in QML is a `px` here, 1:1.

Icons come from `ui/icons.slint`, generated from `shared/icons.json` by
`shared/icongen` — the same table QML uses, third emitter. Fonts are the
bundled `shared/fonts/` files, embedded at compile time via `.slint` imports.

## Desktop

```
cd slint && cargo run
```

State lives in `~/.local/state/sigil-slint/` — sandboxed on purpose, so this
is always a separate Matrix device from the daily `sigil-engine` daemon. Two
sync loops over one crypto store corrupt sessions.

## Android

Needs: Android SDK + NDK (`ANDROID_HOME`, usually `~/Android/Sdk`), a JDK,
`rustup target add aarch64-linux-android`, and `cargo install cargo-apk`.

```
cd slint
export ANDROID_HOME=~/Android/Sdk
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/27.2.12479018
cargo apk run --lib --target aarch64-linux-android
```

That builds, signs, installs and launches `com.sigil.slint` on the connected
device. It coexists with the Compose spike (`com.sigil.app`) — same engine,
two toolkits, one phone, which is the comparison this crate exists to make.

App state lands in the app's private files dir; the engine's XDG path
resolution is pointed there before anything else runs (`android_main`).

## UI iteration without a device

`SIGIL_SLINT_DEMO=1 cargo run` renders the app from canned events pushed
through the real bridge pipeline — no engine, no login. Add
`SIGIL_SLINT_DEMO_CHAT=1` to open the demo conversation, or
`SIGIL_SLINT_DEMO_RECOVERY=1` to land on the recovery page. What the demo
renders is what the pipeline renders; only the source of JSON differs.

## Known gaps

- **Spaces tab** lists spaces from `rooms.list` flat; the `spaces.tree`
  hierarchy, space filtering and the hero header are not built.
- **Media bodies** render as icon+filename chips. Images, the viewer, voice
  playback: later, they need `media.get` plumbing and a texture story.
- **SigilText effects** are not drawn — the timeline shows plain `body`.
  Solid colours/bold/etc can ride Slint's `StyledText`; the animated set
  needs the per-character layout planned for `sigiltext_render`.
- **No recovery-key page yet**: E2EE history stays locked until the session
  is verified from another client, or the page is built (it is small).
- **No pagination**: the newest 60 items per room.
- **The timeline is top-anchored**: a room with little history stacks from
  the top instead of hugging the composer. Needs a content-height measure
  Slint's ListView does not expose directly.
- **Composer is single-line**; Shift+Enter and the formatting affordances
  need Slint's TextEdit story evaluated.
