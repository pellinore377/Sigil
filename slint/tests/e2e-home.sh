#!/usr/bin/env bash
# Home, for real: the Slint app is Bob on a loopback sigil-server; Alice is
# the command-line client. Alice writes first, so Bob gets a request; Bob
# accepts it from the Requests tab, reads the message and replies; Alice
# hears the reply. Needs server/target/debug/sigil-server,
# client/target/debug/sigil-cli and slint/target/debug/drive.
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

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18446 >/dev/null
sed -i 's|^media_udp = .*|media_udp = "127.0.0.1:0"|' sigil.toml
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
sleep 1.5
IA=$($SV -c sigil.toml invite); IB=$($SV -c sigil.toml invite)

# Bob, in the app
XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache HOME=$W \
  timeout 240 "$DRIVE" "$W/shots" 127.0.0.1:18446 "$IB" bob home >drive.out 2>drive.err & PIDS+=($!)
for i in $(seq 1 60); do grep -q "signed in as @bob:sigil.test" drive.out 2>/dev/null && break; sleep 1; done
grep -q "signed in as @bob:sigil.test" drive.out || fail "bob did not sign in" drive.out drive.err

# Alice, at the command line, writes first
$CL -s alice.json init --username @alice:sigil.test --envoy ws://127.0.0.1:18446/envoy >/dev/null
$CL -s alice.json register --invite "$IA" >/dev/null || fail "alice register" server.log
$CL -s alice.json dm @bob:sigil.test "hello from alice" >/dev/null || fail "alice dm" server.log

# Bob accepts and replies; Alice hears it
timeout 120 $CL -s alice.json listen 0 --count 2 >alice.out 2>&1 || true
grep -q "@bob:sigil.test: hi back from bob" alice.out || fail "alice did not hear bob" alice.out drive.out drive.err
wait "${PIDS[-1]}" || fail "drive" drive.out drive.err
grep -q "request from alice" drive.out || fail "request row" drive.out
grep -q "^drive ok" drive.out || fail "drive did not finish" drive.out drive.err
for p in live-home-empty live-requests live-request-open live-chat-accepted live-chat-replied; do
  [ -s "$W/shots/$p.png" ] || fail "missing capture $p" drive.out
done
if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/shots/*.png "$KEEP_SHOTS"/; fi
echo "e2e-home ok"
