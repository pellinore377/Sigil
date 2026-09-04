#!/usr/bin/env bash
# The doors, for real: a sigil-server on loopback, the Slint app with the
# engine linked in, no display. `drive` types the server, reads what it
# offers, creates an account with a password, sees the recovery code,
# lands on Home and opens Settings, capturing each page. Needs
# server/target/debug/sigil-server and slint/target/debug/drive.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
DRIVE=$ROOT/slint/target/debug/drive
# The driver is built from this tree rather than assumed: a stale one here
# silently tests code that is no longer in the repo, which cost a day once.
(cd "$ROOT/slint" && cargo build -q --bin drive)
W=$(mktemp -d); mkdir -p "$W/state" "$W/cache" "$W/shots"; cd "$W"
PIDS=()
trap 'kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -60; done; exit 1; }

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18445 >/dev/null
sed -i 's|^media_udp = .*|media_udp = "127.0.0.1:0"|' sigil.toml
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
sleep 1.5
INVITE=$($SV -c sigil.toml invite)

XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache HOME=$W \
  timeout 180 "$DRIVE" "$W/shots" 127.0.0.1:18445 "$INVITE" alice >drive.out 2>drive.err || fail "drive" drive.out drive.err server.log
grep -q "server offers registration=invite" drive.out || fail "probe" drive.out
grep -q "signed in as @alice:sigil.test" drive.out || fail "create" drive.out drive.err
grep -q "recovery code shown" drive.out || fail "recovery code" drive.out
grep -q "^drive ok" drive.out || fail "settings" drive.out drive.err
for p in live-door-server live-door-choose live-recovery-code live-home live-settings; do
  [ -s "$W/shots/$p.png" ] || fail "missing capture $p" drive.out
done
# nothing about the client leaked into the server log
! grep -v 'listening on\|media to' server.log | grep -Eq '127\.0\.0\.1:[0-9]+' || fail "client address logged" server.log
if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/shots/*.png "$KEEP_SHOTS"/; fi
echo "e2e-doors ok"
