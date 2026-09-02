# Sigil for Slint

The cross-platform frontend: one UI, in Slint, over the Sigil engine linked
in process. This is the port described in
[`docs/slint-port-plan.md`](../docs/slint-port-plan.md), built phase by
phase against the QML app under `omarchy/`, on the Sigil backend and not
Matrix.

## Architecture

There is no daemon here. The engine (`../core`) is a path dependency hosted
on a tokio runtime inside the app:

```
  ui/*.slint  ──callbacks──▶  src/bridge.rs  ──Request──▶  sigil_engine::Engine
  (view only)  ◀──models───   (UI thread)     ◀──Hub────   (tokio, in-process)
```

The JSON protocol is byte-for-byte the socket protocol
(`core/docs/protocol.md`); `bridge.rs` replaces the transport, not the
contract. Rust does the thinking, Slint does the drawing: everything the
QML computed inline lives in `src/`, and the `.slint` files get flat
structs and finished strings.

## Run it

```
cd slint && cargo run
```

Desktop state lives in `~/.local/state/sigil-slint/`, apart from the daily
daemon's store. `SIGIL_SLINT_DEMO=1 cargo run` shows the fixtures instead
of an engine.

## Prove it

Two binaries run the real components with no display, on a Slint platform
that renders into a pixel buffer (`src/headless.rs`):

- `cargo run --bin shots -- out/` captures every page with the demo
  fixtures, one PNG each. This is the side-by-side sheet each phase of the
  plan ends with.
- `cargo run --bin drive -- out/ <server> <invite>` is the end-to-end
  driver: the real engine against a real server, walking the doors as a
  person would and capturing each page. `tests/e2e-doors.sh` starts a
  server on loopback and runs it.

## Where the port stands

Phases 0 and 1 of the plan are done: the harness, the Matrix-only pages
cut out, Home with a Requests tab, and the doors (server first, then
create, restore, or link this device, with the recovery code and the
settings page). The chat, group, media and call pages are the first
attempt's and are reworked in their phases; their gaps are in the plan.

## Android

Needs: Android SDK + NDK (`ANDROID_HOME`), a JDK,
`rustup target add aarch64-linux-android`, and `cargo install cargo-apk`.

```
cd slint
export ANDROID_HOME=~/Android/Sdk
cargo apk run --lib
```
