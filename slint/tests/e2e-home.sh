#!/usr/bin/env bash
# Home, for real: the Slint app is Bob on a loopback sigil-server; Alice is
# the command-line client. Alice writes first, so Bob gets a request; Bob
# accepts it from the Requests tab, reads the message and replies; Alice
# hears the reply. Then Bob opens Start chat: the suggestions hold the people
# he knows, and Wren — a third account he has never spoken to — is found by
# "wren", "@wren" and "@wren:sigil.test" alike and tapped into a chat.
# Needs server/target/debug/sigil-server,
# client/target/debug/sigil-cli and slint/target/debug/drive.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
CL=$ROOT/client/target/debug/sigil-cli
DRIVE=$ROOT/slint/target/debug/drive
# A drawn port, not a fixed one: two suites on one machine used to land on
# the same server and the second one's first call came back 401.
PORT=$((20000 + RANDOM % 20000))
# The driver is built from this tree rather than assumed: a stale one here
# silently tests code that is no longer in the repo, which cost a day once.
(cd "$ROOT/slint" && cargo build -q --bin drive)
W=$(mktemp -d); mkdir -p "$W/state" "$W/cache" "$W/shots"; cd "$W"
PIDS=()
trap 'kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -60; done; exit 1; }

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:${PORT} >/dev/null
sed -i 's|^media_udp = .*|media_udp = "127.0.0.1:0"|' sigil.toml
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
sleep 1.5
IA=$($SV -c sigil.toml invite); IB=$($SV -c sigil.toml invite); IW=$($SV -c sigil.toml invite)

# Wren, a third account on the same server that Bob has never spoken to.
# She exists before the app starts, so the Start page can look her up: the
# only way her row can appear is the front desk answering about her name.
$CL -s wren.json init --username @wren:sigil.test --envoy ws://127.0.0.1:${PORT}/envoy >/dev/null
$CL -s wren.json register --invite "$IW" >/dev/null || fail "wren register" server.log
# ...and she listens, so her requests slot exists: a registered name alone
# has no slot to drop a first message into (blind-backend "slot creation
# timing"), and the DM Bob starts from the search result lands here.
timeout 260 $CL -s wren.json requests --count 1 >wren.out 2>&1 & PIDS+=($!)
sleep 1

# Bob, in the app
XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache HOME=$W DRIVE_FIND_USER=wren \
  timeout 300 "$DRIVE" "$W/shots" 127.0.0.1:${PORT} "$IB" bob home >drive.out 2>drive.err & PIDS+=($!)
for i in $(seq 1 60); do grep -q "signed in as @bob:sigil.test" drive.out 2>/dev/null && break; sleep 1; done
grep -q "signed in as @bob:sigil.test" drive.out || fail "bob did not sign in" drive.out drive.err

# Alice, at the command line, writes first
$CL -s alice.json init --username @alice:sigil.test --envoy ws://127.0.0.1:${PORT}/envoy >/dev/null
$CL -s alice.json register --invite "$IA" >/dev/null || fail "alice register" server.log
$CL -s alice.json dm @bob:sigil.test "hello from alice" >/dev/null || fail "alice dm" server.log

# Bob accepts and replies; Alice hears it
timeout 120 $CL -s alice.json listen 0 --count 2 >alice.out 2>&1 || true
grep -q "@bob:sigil.test: hi back from bob" alice.out || fail "alice did not hear bob" alice.out drive.out drive.err
wait "${PIDS[-1]}" || fail "drive" drive.out drive.err
grep -q "request from alice" drive.out || fail "request row" drive.out
# Start chat: the suggestions hold the people Bob knows, and Wren — a
# stranger — is found by all three ways of typing her name, tapped, and the
# conversation opens.
grep -q "suggestions hold alice" drive.out || fail "the Start page suggested nobody" drive.out drive.err
for q in wren @wren @wren:sigil.test; do
  grep -q "found @wren:sigil.test by '$q'" drive.out || fail "the Start page did not find wren by '$q'" drive.out drive.err
done
grep -q "dm with @wren:sigil.test opened" drive.out || fail "tapping the result did not open a chat" drive.out drive.err
for i in $(seq 1 30); do grep -q "request from @bob:sigil.test" wren.out 2>/dev/null && break; sleep 1; done
grep -q "request from @bob:sigil.test" wren.out || fail "the DM never reached wren" wren.out drive.out
grep -q "^drive ok" drive.out || fail "drive did not finish" drive.out drive.err
for p in live-home-empty live-requests live-request-open live-chat-accepted live-chat-replied live-start-found; do
  [ -s "$W/shots/$p.png" ] || fail "missing capture $p" drive.out
done
if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/shots/*.png "$KEEP_SHOTS"/; fi
echo "e2e-home ok"
