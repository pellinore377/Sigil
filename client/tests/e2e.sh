#!/usr/bin/env bash
# End to end over the real protocol: one server in `both` role, two users,
# a direct message started by username (MLS Welcome through the requests
# slot), messages both ways, own-message readback, the offline queue, a
# server restart, a bad invite and a double spend.
# Needs: server/target/debug/sigil-server and client/target/debug/sigil-cli.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
CL=$ROOT/client/target/debug/sigil-cli
W=$(mktemp -d); cd "$W"
trap 'pkill -x sigil-server >/dev/null 2>&1 || true; pkill -x sigil-cli >/dev/null 2>&1 || true; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null; done; exit 1; }

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18443 >/dev/null
$SV -c sigil.toml run >server.log 2>&1 &
sleep 1.5
A=$($SV -c sigil.toml invite); B=$($SV -c sigil.toml invite)
$CL -s alice.json init --username @alice:sigil.test --envoy ws://127.0.0.1:18443/envoy >/dev/null
$CL -s bob.json   init --username @bob:sigil.test   --envoy ws://127.0.0.1:18443/envoy >/dev/null
$CL -s alice.json register --invite "$A" >/dev/null
$CL -s bob.json   register --invite "$B" >/dev/null
$CL -s bob.json lookup @alice:sigil.test | grep -q "@alice:sigil.test on sigil.test" || fail lookup
if $CL -s alice.json register --invite nope 2>/dev/null; then fail "bad invite accepted"; fi

# DM by username: Welcome through the requests slot
timeout 25 $CL -s bob.json requests --accept --count 1 >bob_req.out 2>&1 &
sleep 3
$CL -s alice.json dm @bob:sigil.test "first" >/dev/null
sleep 4
grep -q "request from @alice:sigil.test: first" bob_req.out || fail "request not received" bob_req.out server.log
grep -q "^accepted" bob_req.out || fail "not accepted" bob_req.out
$CL -s bob.json list | grep -q "with @alice:sigil.test" || fail "bob has no conversation"

# messages both ways, plus own-message readback
timeout 20 $CL -s bob.json listen 0 --count 2 >bob_listen.out 2>&1 &
sleep 2
$CL -s alice.json send 0 "one" >/dev/null
$CL -s alice.json send 0 "two" >/dev/null
sleep 4
grep -q "^1 .*@alice:sigil.test: one$" bob_listen.out && grep -q "^2 .*@alice:sigil.test: two$" bob_listen.out || fail "bob did not get both" bob_listen.out
$CL -s bob.json send 0 "three" >/dev/null
sleep 0.5
timeout 10 $CL -s alice.json listen 0 --count 3 >alice_listen.out 2>&1 &
sleep 4
grep -q "^1 me: one$" alice_listen.out && grep -q "^2 me: two$" alice_listen.out && grep -q "^3 .*@bob:sigil.test: three$" alice_listen.out || fail "alice readback" alice_listen.out

# offline queue: bob subscribes, leaves, alice sends, bob returns
timeout 8 $CL -s bob.json listen 0 --count 9 >/dev/null 2>&1 &
sleep 2.5; pkill -x sigil-cli || true; sleep 0.5
$CL -s alice.json send 0 "four" >/dev/null
timeout 8 $CL -s bob.json listen 0 --count 4 >bob2.out 2>&1 &
sleep 4
grep -q "^4 .*@alice:sigil.test: four$" bob2.out || fail "offline queue" bob2.out

# double spend
python3 - <<'PY'
import json; s=json.load(open('bob.json')); s['tokens'].append(s['tokens'][-1]); json.dump(s,open('bob.json','w'))
PY
$CL -s bob.json send 0 "dup1" >/dev/null
if $CL -s bob.json send 0 "dup2" 2>/dev/null; then fail "double spend accepted"; fi

# restart persistence: history survives, and so does the MLS state on both sides
pkill -x sigil-server; sleep 0.5
$SV -c sigil.toml run >server2.log 2>&1 &
sleep 1.5
timeout 8 $CL -s alice.json listen 0 --count 5 >alice3.out 2>&1 &
sleep 4
[ "$(grep -c '^[0-9]' alice3.out)" = "5" ] || fail "history after restart" alice3.out
! grep -v 'listening on' server.log server2.log | grep -Eq '127\.0\.0\.1:[0-9]+' || fail "client address logged" server.log
echo "e2e ok"
