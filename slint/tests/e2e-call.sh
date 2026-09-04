#!/usr/bin/env bash
# A voice call between two copies of the app on one loopback server: Bob
# starts a conversation with Alice, she answers, he calls; both hear each
# other (a test tone stands in for the microphone), a reaction and a mute
# cross, he hangs up and her call ends. Frames go through the forwarding
# unit sealed under the conversation's epoch key. Needs the same binaries
# as e2e-home.sh.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
DRIVE=$ROOT/slint/target/debug/drive
# The driver is built from this tree rather than assumed: a stale one here
# silently tests code that is no longer in the repo, which cost a day once.
(cd "$ROOT/slint" && cargo build -q --bin drive)
W=$(mktemp -d); mkdir -p "$W/a/state" "$W/a/cache" "$W/b/state" "$W/b/cache" "$W/shots"; cd "$W"
PIDS=()
keep_logs() { if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/*.out "$W"/*.err "$W"/*.log "$W"/shots/*.png "$KEEP_SHOTS"/ 2>/dev/null || true; fi; }
trap 'keep_logs; kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; grep -v "^   \|^  *at " "$f" 2>/dev/null | head -60; done; exit 1; }

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18451 >/dev/null
sed -i 's|^media_udp = .*|media_udp = "127.0.0.1:0"|' sigil.toml
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
sleep 1.5
IA=$($SV -c sigil.toml invite); IB=$($SV -c sigil.toml invite)

SIGIL_FAKE_AUDIO=1 XDG_STATE_HOME=$W/b/state XDG_CACHE_HOME=$W/b/cache HOME=$W/b \
  timeout 420 "$DRIVE" "$W/shots" 127.0.0.1:18451 "$IA" alice callee >callee.out 2>callee.err & PIDS+=($!)
CALLEE=$!
sleep 3
SIGIL_FAKE_AUDIO=1 XDG_STATE_HOME=$W/a/state XDG_CACHE_HOME=$W/a/cache HOME=$W/a \
  timeout 420 "$DRIVE" "$W/shots" 127.0.0.1:18451 "$IB" bob caller >caller.out 2>caller.err & PIDS+=($!)
CALLER=$!
wait "$CALLER" || fail "caller" caller.out caller.err callee.out callee.err server.log
wait "$CALLEE" || fail "callee" callee.out callee.err caller.out server.log
grep -q "^drive caller ok" caller.out || fail "caller did not finish" caller.out caller.err
grep -q "^drive callee ok" callee.out || fail "callee did not finish" callee.out callee.err
grep -q "^in the call, both heard" caller.out || fail "no audio both ways" caller.out
grep -q "incoming call from bob" callee.out || fail "no incoming banner" callee.out
for p in live-call live-call-react live-call-muted live-call-pip live-call-incoming live-call-callee live-call-callee-react live-call-callee-muted; do
  [ -s "$W/shots/$p.png" ] || fail "missing capture $p" caller.out callee.out
done
keep_logs
echo "e2e-call ok"
