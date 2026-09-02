#!/usr/bin/env bash
# End-to-end: one server in `both` role, two users, registration, tokens,
# racing send/subscribe with backfill, offline queue, restart persistence,
# rejected invite, rejected double-spend. Needs a built target/debug.
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
SV=$HERE/target/debug/sigil-server
CL=$HERE/target/debug/sigil-cli
W=$(mktemp -d)
cd "$W"
trap 'pkill -x sigil-server >/dev/null 2>&1 || true; rm -rf "$W"' EXIT

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18443 >/dev/null
$SV -c sigil.toml run >server.log 2>&1 &
sleep 1.5
A=$($SV -c sigil.toml invite); B=$($SV -c sigil.toml invite)
$CL -s alice.json init --username @alice:sigil.test --envoy ws://127.0.0.1:18443/envoy >/dev/null
$CL -s bob.json   init --username @bob:sigil.test   --envoy ws://127.0.0.1:18443/envoy >/dev/null
$CL -s alice.json register --invite "$A" >/dev/null
$CL -s bob.json   register --invite "$B" >/dev/null
$CL -s bob.json lookup @alice:sigil.test | grep -q "@alice:sigil.test on sigil.test"
if $CL -s alice.json register --invite nope 2>/dev/null; then echo "FAIL: bad invite accepted"; exit 1; fi

EPOCH=$(python3 -c "import os;print(os.urandom(32).hex())")
timeout 20 $CL -s alice.json listen --epoch "$EPOCH" --count 3 >alice.out 2>/dev/null &
$CL -s bob.json send --epoch "$EPOCH" "one" >/dev/null
$CL -s bob.json send --epoch "$EPOCH" "two" >/dev/null
sleep 2.5
$CL -s bob.json send --epoch "$EPOCH" "three" >/dev/null
sleep 3
grep -q "^1 .* one$" alice.out && grep -q "^2 .* two$" alice.out && grep -q "^3 .* three$" alice.out || { echo "FAIL: alice"; cat alice.out; exit 1; }
[ "$(grep -c '^[0-9]' alice.out)" = "3" ] || { echo "FAIL: duplicates"; cat alice.out; exit 1; }

# offline queue: bob subscribes, leaves, alice sends, bob returns
timeout 8 $CL -s bob.json listen --epoch "$EPOCH" --count 4 >/dev/null 2>&1 &
sleep 2.5; pkill -x sigil-cli || true; sleep 0.5
$CL -s alice.json send --epoch "$EPOCH" "four" >/dev/null
timeout 8 $CL -s bob.json listen --epoch "$EPOCH" --count 4 >bob.out 2>/dev/null &
sleep 3
grep -q "^4 .* four$" bob.out || { echo "FAIL: offline queue"; cat bob.out; exit 1; }

# double spend
python3 - <<'PY'
import json; s=json.load(open('bob.json')); s['tokens'].append(s['tokens'][-1]); json.dump(s,open('bob.json','w'))
PY
$CL -s bob.json send --epoch "$EPOCH" "dup1" >/dev/null
if $CL -s bob.json send --epoch "$EPOCH" "dup2" 2>/dev/null; then echo "FAIL: double spend accepted"; exit 1; fi

# restart persistence
pkill -x sigil-server; sleep 0.5
$SV -c sigil.toml run >server2.log 2>&1 &
sleep 1.5
[ "$(( $($CL -s alice.json history --epoch "$EPOCH" | grep -c '^[0-9]') ))" = "5" ] || { echo "FAIL: history after restart"; exit 1; }
# the store must hold no timestamps: grep the database for this year's epoch-ms prefix is meaningless
# (encrypted), so check instead that no log line carries a client address
! grep -v 'listening on' server.log server2.log | grep -Eq '127\.0\.0\.1:[0-9]+' || { echo "FAIL: client address logged"; exit 1; }
echo "e2e ok"
