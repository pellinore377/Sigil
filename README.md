# Sigil

A native messenger, built to run everywhere without feeling like a web page
wrapped in a window.

> **This branch (`encrypt`) is Sigil-native.** The Matrix backend has been
> removed from the engine here; the engine speaks to a Sigil server
> ([`server/`](server/)) through the blind protocol in
> [`docs/blind-backend.md`](docs/blind-backend.md). The Matrix client lives
> on `main` until the two converge.

A Rust daemon owns the session — encryption, conversations, media, calls — and
speaks a small JSON protocol over a local socket. Every user interface is a
thin view over that protocol and talks to no network itself. That split is what
makes the same client reasonable to ship as a Linux shell panel, a desktop
application, and eventually a phone app, without three copies of the hard part.

> **Status: early.** Sigil runs daily on Linux under Hyprland. Nothing else is
> built yet, the desktop UI is being rewritten, and there are no release
> builds — you install it by building it.

## What it does

**Messaging** — rooms, direct messages and spaces; replies, edits, reactions,
threads and pinned messages; read receipts and typing indicators; search.

**Encryption** — end-to-end encrypted rooms, with cross-signing and history
unlocked by your existing recovery key. Verified devices show as verified in
Element and every other client.

**Media** — images, video, audio and voice messages, with previews for PDFs and
office documents. Locations render as real maps. Contacts send and receive as
vCards.

**Calls** — native voice and video over MatrixRTC and LiveKit, with
end-to-end-encrypted media, screen sharing, and per-device microphone and
speaker selection. Interoperates with Element and Element X.

**Spaces** — browse a space's rooms including ones you have not joined yet,
manage its children, and edit membership, roles, permissions, notifications and
privacy.

## Requirements

A Sigil server: one container, see [`server/README.md`](server/README.md).
Registration is by invite code from the operator.

To build: `cargo` 1.93+, `clang`, `pkg-config`, `make`, and Qt 6 development
headers. Calls want `pipewire-pulse`; screen sharing wants an
`xdg-desktop-portal` implementation.

## Install

The only frontend built so far runs on [Omarchy](https://omarchy.org):

```
git clone https://github.com/pellinore377/Sigil.git
cd Sigil && bash install.sh --apply
```

This builds the engine, installs it to `~/.local/bin/sigil-engine`, enables the
plugin and adds a keybinding. The first build downloads a prebuilt libwebrtc
(~300 MB) and takes several minutes.

Open with the bar icon or `SUPER + ALT + M`.

## Signing in

Sigil opens your browser to your homeserver's login page and completes the
sign-in on a localhost redirect. The session persists across restarts and
appears in your other clients as a device named `Sigil on <host>`.

On first login it asks for your **recovery key** to unlock secret storage. That
verifies the device and decrypts existing history; you can skip it and enter it
later from the avatar menu.

## Keyboard

| | |
|---|---|
| `SUPER+ALT+M` | open / close |
| `SUPER+ALT+SHIFT+M` | answer or hang up, from anywhere |
| `Esc` | back / close |
| `Ctrl+K` · `Ctrl+F` | filter rooms |
| `Alt+↑/↓` | previous / next room (`+Shift`: unread only) |
| `Enter` · `Shift+Enter` | send · newline |
| `↑` in an empty composer | edit your last message |
| `Ctrl+Shift+M` · `Ctrl+Shift+H` | mute · hang up |

## How it works

```
  frontend  ──JSON over a unix socket──▶  sigil-engine  ──sealed bags──▶  Envoy ──▶  Sigil server
  (QML today;                            (Rust: MLS, slots,
   native apps to come)                   media, calls)
```

The engine holds the sqlite crypto store, the media cache and the call stack.
Frontends render state and send actions. Adding a platform means writing a
view, not reimplementing Matrix — and a second frontend on the same machine
shares one session and one set of keys rather than registering a new device.

State lives in `~/.local/state/sigil/`, cache in `~/.cache/sigil/`.

## Platforms

Linux works today, through the Omarchy frontend. A standalone desktop application for
Linux and Windows is being written. macOS and iOS are planned in SwiftUI, and
Android in Kotlin, rather than a shared toolkit — Qt's licensing makes App
Store distribution painful, and native frameworks feel better on a phone. A web
frontend is planned last.

## Documentation

Protocol reference, contribution notes and design conventions are in
[`docs/`](docs/) and [`core/docs/protocol.md`](core/docs/protocol.md).
Working on Sigil itself: [`docs/development.md`](docs/development.md).
The plan for replacing the Matrix homeserver with a metadata-blind
Sigil-native backend: [`docs/blind-backend.md`](docs/blind-backend.md). Its
normative derivation spec is [`docs/spec/sigil-protocol-v1.md`](docs/spec/sigil-protocol-v1.md),
implemented and vector-tested by the `sigil-protocol` crate in [`protocol/`](protocol/).
The plan for rebuilding the whole UI in Slint on that backend, page by page:
[`docs/slint-port-plan.md`](docs/slint-port-plan.md).

## Licence

All rights reserved. The bundled fonts carry their own licences — see
[`shared/fonts/README.md`](shared/fonts/README.md).
