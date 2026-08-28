# Working on Sigil

Maintainer notes: layout, rebuild loop, test hooks, and the traps already paid
for in bugs. For what Sigil *is*, see the top-level README.

## Layout

Sigil is an engine plus frontends. The engine is the client; a frontend is a
view over it. Today there is one frontend — an Omarchy shell plugin — and the
repository is still shaped around it for historical reasons rather than because
that arrangement is right.

**The engine**, which is Sigil proper:

| | |
|---|---|
| `core/` | Rust daemon, installed to `~/.local/bin/sigil-engine`. `core/docs/protocol.md` is the socket contract. |
| socket | `$XDG_RUNTIME_DIR/sigil.sock` — JSON lines. `sigil-engine cli <req> k=v…` talks to it. |
| state | `~/.local/state/sigil/` — sqlite store, `session.json`, `store.key`, `settings.json` (0600) |
| cache | `~/.cache/sigil/media/` — media, thumbnails, avatars (GC at 2 GB) |
| video frames | `$XDG_RUNTIME_DIR/sigil/video-<track>.shm` — one file per live track |

**The Omarchy frontend**, one consumer of the above:

| | |
|---|---|
| `omarchy/Service.qml` | engine supervisor (spawn + reconnect), state model, notifications, IPC target `sigil`, call banner and pill |
| `omarchy/Panel.qml` + `mobile/`, `components/`, `calls/` | the panel (layer namespace `omarchy-sigil`), pages, timeline, composer, call view |
| `omarchy/BarWidget.qml` | bar icon with mention/unread/call badges |
| `omarchy/video/` | `VideoSurface` QML item reading shared-memory RGBA frames (`video/omv_shm.h`) |
| `omarchy/manifest.json`, `omarchy/install.sh` | plugin registration and installer |

## Rebuild loop

```
bash install.sh --apply    # full build + enable + theme rule + keybinds
bin/sigil-setup --check    # after a Qt upgrade: rebuild the .so only if Qt changed
```

Engine only:

```
cd core && CARGO_TARGET_DIR=~/.cache/omarchy-matrix/build cargo build --release
install -m755 ~/.cache/omarchy-matrix/build/release/sigil-engine ~/.local/bin/sigil-engine
pkill -f "^$HOME/.local/bin/sigil-engine daemon"    # Service.qml respawns it
```

The engine logs to `~/.local/state/sigil/engine.log`; the shell logs to the
journal.

> **Stale UI:** the shell compiles plugin QML into `~/.cache/quickshell/qmlcache/`.
> After adding files or reworking pages a restart can still render the *old* UI.
> `rm -rf ~/.cache/quickshell/qmlcache && omarchy restart shell`

## Test hooks

```
omarchy-shell sigil status|toggle|openRoom <id>|callToggle|markAllRead|debug
omarchy-shell sigilui goto <page> <arg>   # space|newspace|spacerooms manage|add|
                                          # spacesettings|members|notifications|
                                          # security|roles|permissions|home
omarchy-shell sigilui navState            # where the panel thinks it is
omarchy-shell sigilui spaceMenu 1
omarchy-shell sigilui notifMode mentions
omarchy-shell sigilui permLevel invite 50
sigil-engine cli status | rooms.list | room.open roomId=!x --follow | call.devices
sigil-engine shmtest --seconds 2      # test pattern for the video plugin
sigil-engine shmdump <key> out.png
```

Sandbox: a directory outside the plugin tree with a `shell.qml` hosting
Service+Panel against a fake shell object, driven by
`cargo run --example fake-engine` (add `--login-page` to start logged out).
`SIGIL_NO_SPAWN=1 SANDBOX_AUTO_OPEN=1 quickshell -p .`

## Conventions

- [`docs/portability.md`](portability.md) — the platform contract: input rules,
  what is Linux-only, and a dated readiness audit. Read before designing UI.
- [`docs/ui-conventions.md`](ui-conventions.md) — tokens, theme derivation, page
  anatomy, and pitfalls already paid for in bugs.
- **Comments are terse.** One line where one line does. Say what stops someone
  breaking it, never how it came to be, and never state a fact about your own
  machine, peripherals or server — that is how private data reaches a public
  repo.

## Traps worth knowing

**A space is a room.** `m.space` in the creation content is the only
difference, so the settings pages take a plain room id and serve both.

**A space's children are not in the room list.** They include rooms this
account has not joined, which have no `rooms` entry at all. `SpacePage` builds
from `space.hierarchy`; building from the room list showed an empty space.

**Notification modes are three-state**, not two: follow the account default, or
carry a per-room rule. Turning off "allow custom setting" *deletes* the rule
rather than setting it to "all". Writes must show optimistically — a read
issued the instant the write returns still sees the old value, because the push
rule has not reached the local store.

**Power levels are written one key at a time.** A batch that half-applies
leaves the room in a state nobody chose.

**Encryption is one-way** in the spec; the control locks once on rather than
pretending otherwise.

**Call membership refreshes reuse a stable `created_ts`** so Element X does not
read each refresh as a membership change and rotate media keys.

**The system `pipewire` crate must not be linked into the engine.** Its
`pw_init` segfaults against the PipeWire copy inside libwebrtc (confirmed by
coredump). The `glib-main-loop` feature on `livekit`/`libwebrtc` is required, or
the portal's GDBus callbacks never dispatch and the screen-share picker never
appears.

**Truncated MJPEG frames** (no `FF D9` end marker) decode into a garbage band
and are dropped before encoding.

**Delayed events (MSC4140).** With `msc4140_enabled` the server removes our call
membership when the engine dies; otherwise memberships carry a 4 h `expires`
and refresh every 25 s.
