#!/usr/bin/env bash
# Groups, for real: Bob (the Slint app) makes a group, adds Alice (the
# command-line client), makes her an admin, renames it and leaves; Alice
# accepts the invitation and hears every policy change and the leave.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
CL=$ROOT/client/target/debug/sigil-cli
DRIVE=$ROOT/slint/target/debug/drive
# The driver is built from this tree rather than assumed: a stale one here
# silently tests code that is no longer in the repo, which cost a day once.
(cd "$ROOT/slint" && cargo build -q --bin drive)
W=$(mktemp -d); mkdir -p "$W/state" "$W/cache" "$W/shots"; cd "$W"
PIDS=()
trap 'kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -60; done; exit 1; }

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18448 >/dev/null
sed -i 's|^media_udp = .*|media_udp = "127.0.0.1:0"|' sigil.toml
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
sleep 1.5
IA=$($SV -c sigil.toml invite); IB=$($SV -c sigil.toml invite)

$CL -s alice.json init --username @alice:sigil.test --envoy ws://127.0.0.1:18448/envoy >/dev/null
$CL -s alice.json register --invite "$IA" >/dev/null || fail "alice register" server.log

# A signed-in device always listens for requests; Alice's listener has to be
# up before anyone can write to her requests slot.
timeout 200 $CL -s alice.json requests --accept >alice_req.out 2>&1 &
RPID=$!
sleep 2

XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache HOME=$W DRIVE_SYNC=$W/go \
  timeout 300 "$DRIVE" "$W/shots" 127.0.0.1:18448 "$IB" bob groups >drive.out 2>drive.err & PIDS+=($!)
for i in $(seq 1 90); do grep -q "invited alice" drive.out 2>/dev/null && break; sleep 1; done
grep -q "invited alice" drive.out || fail "bob did not add alice" drive.out drive.err

wait $RPID || fail "alice accept" alice_req.out
grep -q "^accepted" alice_req.out || fail "alice did not join" alice_req.out
touch "$W/go"
timeout 150 $CL -s alice.json listen 0 --count 0 >alice.out 2>&1 &
LPID=$!
wait "${PIDS[-1]}" || fail "drive" drive.out drive.err
sleep 6; kill $LPID 2>/dev/null || true
grep -c "(policy updated by @bob:sigil.test)" alice.out | grep -q "^[2-9]" || fail "alice missed the policy changes" alice.out drive.out
grep -q "membership:" alice.out || fail "alice missed the leave" alice.out
$CL -s alice.json list >alice_list.out 2>&1 || true
grep -q "the better plan" alice_list.out || fail "alice's group not renamed" alice_list.out alice.out
grep -q "^drive groups ok" drive.out || fail "drive did not finish" drive.out drive.err
for p in live-group-settings live-group-members live-group-admins live-group-privacy; do
  [ -s "$W/shots/$p.png" ] || fail "missing capture $p" drive.out
done
if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/shots/*.png "$KEEP_SHOTS"/; fi
echo "e2e-groups-app ok"
