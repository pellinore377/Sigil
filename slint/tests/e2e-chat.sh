#!/usr/bin/env bash
# The conversation, for real: the home scenario, then Bob (the Slint app)
# replies with a quote, reacts, edits and deletes, sends a picture, a
# document and a track and opens each, and Alice on the command-line
# client sees each event arrive. Needs the same binaries as
# e2e-home.sh.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
CL=$ROOT/client/target/debug/sigil-cli
DRIVE=$ROOT/slint/target/debug/drive
W=$(mktemp -d); mkdir -p "$W/state" "$W/cache" "$W/shots"; cd "$W"
PIDS=()
trap 'kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -60; done; exit 1; }

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18447 >/dev/null
sed -i 's|^media_udp = .*|media_udp = "127.0.0.1:0"|' sigil.toml
$SV -c sigil.toml run >server.log 2>&1 & SV_PID=$!; PIDS+=($SV_PID)
sleep 1.5
IA=$($SV -c sigil.toml invite); IB=$($SV -c sigil.toml invite)

mkdir -p "$W/sync"
XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache HOME=$W DRIVE_SYNC=$W/sync \
  timeout 480 "$DRIVE" "$W/shots" 127.0.0.1:18447 "$IB" bob chat >drive.out 2>drive.err & PIDS+=($!)
DRIVE_PID=$!
for i in $(seq 1 60); do grep -q "signed in as @bob:sigil.test" drive.out 2>/dev/null && break; sleep 1; done
grep -q "signed in as @bob:sigil.test" drive.out || fail "bob did not sign in" drive.out drive.err

$CL -s alice.json init --username @alice:sigil.test --envoy ws://127.0.0.1:18447/envoy >/dev/null
$CL -s alice.json register --invite "$IA" >/dev/null || fail "alice register" server.log
$CL -s alice.json dm @bob:sigil.test "hello from alice" >/dev/null || fail "alice dm" server.log

# Alice sees: her own hello, bob's reply, the quoted reply, the reaction (kind 2),
# the edit (kind 3) and the deletion (kind 4). --count counts messages only.
timeout 200 $CL -s alice.json listen 0 --count 3 >alice.out 2>&1 || true
# The offline stage: the server goes away while the app sends, comes back
# on the same data, and the app's retry delivers the message.
for i in $(seq 1 240); do grep -q "^server down please" drive.out 2>/dev/null && break; sleep 1; done
grep -q "^server down please" drive.out || fail "drive did not reach the offline stage" drive.out drive.err
kill "$SV_PID"; wait "$SV_PID" 2>/dev/null || true; sleep 1
touch "$W/sync/down"
for i in $(seq 1 180); do grep -q "^server up please" drive.out 2>/dev/null && break; sleep 1; done
grep -q "^server up please" drive.out || fail "the message did not fail while offline" drive.out drive.err
$SV -c sigil.toml run >>server.log 2>&1 & SV_PID=$!; PIDS+=($SV_PID)
sleep 2
touch "$W/sync/up"
wait "$DRIVE_PID" || fail "drive" drive.out drive.err
timeout 60 $CL -s alice.json listen 0 --count 8 >alice2.out 2>&1 || true
grep -q "@bob:sigil.test: hi back from bob" alice.out || fail "alice did not hear bob" alice.out drive.out
grep -q "@bob:sigil.test: quoting you" alice.out || fail "alice did not get the quoted reply" alice.out
# the small events land in whichever listen was running when they arrived
cat alice.out alice2.out >alice-all.out
grep -q "(event kind 2)" alice-all.out || fail "alice did not get the reaction" alice-all.out
grep -q "(event kind 3)" alice-all.out || fail "alice did not get the edit" alice-all.out
grep -q "(event kind 4)" alice-all.out || fail "alice did not get the deletion" alice-all.out
grep -q "\[file\] live-chat-reacted.png" alice-all.out || fail "alice did not get the picture" alice-all.out
grep -q "\[file\] notes.md" alice-all.out || fail "alice did not get the document" alice-all.out
[ "$(grep -c "\[file\] tone.wav" alice-all.out)" -ge 2 ] || fail "alice did not get the track and the voice message" alice-all.out
grep -q "^drive chat ok" drive.out || fail "drive did not finish" drive.out drive.err
grep -q "@bob:sigil.test: sent while offline" alice-all.out || fail "alice did not get the retried message" alice-all.out drive.out
for p in live-chat-reacted live-chat-edited live-chat-deleted live-chat-picture live-viewer live-chat-doc live-doc live-chat-audio live-audio live-chat-voice live-chat-failed live-chat-retried; do
  [ -s "$W/shots/$p.png" ] || fail "missing capture $p" drive.out
done
if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/shots/*.png "$KEEP_SHOTS"/; fi
echo "e2e-chat ok"
