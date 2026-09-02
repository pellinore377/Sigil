#!/usr/bin/env bash
# Two engine daemons on one Sigil server, driven over the JSON protocol the
# frontends use: account.create, users.search, dm.create, the invite room,
# room.join, message.send both ways, a reaction, and a restart.
# Needs server/target/debug/sigil-server and core/target/debug/sigil-engine.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
EN=$ROOT/core/target/debug/sigil-engine
W=$(mktemp -d); mkdir -p "$W/a" "$W/b" "$W/run"; cd "$W"
PIDS=()
trap 'kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -40; done; exit 1; }
result() { python3 -c "import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get('ok') else 1)"; }

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18444 >/dev/null
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
sleep 1.5
IA=$($SV -c sigil.toml invite); IB=$($SV -c sigil.toml invite)
start_engine() { # name
  XDG_STATE_HOME=$W/$1 XDG_CACHE_HOME=$W/$1/cache SIGIL_SOCKET=$W/run/$1.sock $EN daemon --log-level info >>engine-$1.log 2>&1 & PIDS+=($!)
}
A() { SIGIL_SOCKET=$W/run/a.sock $EN cli "$@"; }
B() { SIGIL_SOCKET=$W/run/b.sock $EN cli "$@"; }
start_engine a; start_engine b; sleep 2.5

A account.create username=@alice:sigil.test invite="$IA" envoy=ws://127.0.0.1:18444/envoy | result || fail "alice account" engine-a.log
B account.create username=@bob:sigil.test   invite="$IB" envoy=ws://127.0.0.1:18444/envoy | result || fail "bob account" engine-b.log
A status | grep -q '"session": "loggedIn"' || fail "alice not logged in"
A users.search query=bob:sigil.test | grep -q '"userId": "@bob:sigil.test"' || fail "users.search"
ROOM=$(A dm.create userId=@bob:sigil.test | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['roomId'])")
sleep 4
REQ=$(B rooms.list | python3 -c "import json,sys; print([r['id'] for r in json.load(sys.stdin)['result']['rooms'] if r['isInvite']][0])") || fail "bob has no invite" engine-b.log
B room.join roomIdOrAlias="$REQ" | grep -q "\"roomId\": \"$ROOM\"" || fail "room.join"
A room.open roomId="$ROOM" initialItems:=60 | result || fail "alice room.open"
B room.open roomId="$ROOM" initialItems:=60 | result || fail "bob room.open"
timeout 15 env SIGIL_SOCKET=$W/run/b.sock $EN cli ping --follow >bob-events.json 2>&1 &
sleep 1
A message.send roomId="$ROOM" body="hello bob, red::this is red;" | result || fail "message.send"
sleep 3
B message.send roomId="$ROOM" body="hi alice" | result || fail "bob message.send"
sleep 3
python3 - "$ROOM" <<'PY' || fail "bob events" bob-events.json
import json,sys
raw=open('bob-events.json').read(); dec=json.JSONDecoder(); i=0; diffs=[]
while i<len(raw):
    while i<len(raw) and raw[i] in ' \n\r\t': i+=1
    if i>=len(raw): break
    try: d,i=dec.raw_decode(raw,i)
    except Exception: break
    if d.get('event')=='timeline.diff' and d.get('roomId')==sys.argv[1]:
        for op in d['ops']: diffs.append((op['item']['sender'], op['item']['body'], bool(op['item'].get('effects'))))
assert ('@alice:sigil.test','hello bob, this is red',True) in diffs, diffs
assert ('@bob:sigil.test','hi alice',False) in diffs, diffs
PY
# reaction
EID=$(python3 -c "import json; print(json.load(open('$W/a/sigil/sigil-history.json'))['$ROOM'][-1]['eventId'])")
B message.react roomId="$ROOM" eventId="$EID" key=👍 | result || fail "react"
sleep 3
python3 -c "import json; it=json.load(open('$W/a/sigil/sigil-history.json'))['$ROOM'][-1]; assert it['reactions'][0]['key']=='👍', it" || fail "reaction not applied at alice"
# restart bob's engine: session, rooms, history come back
kill "${PIDS[-1]}" 2>/dev/null || true; sleep 1
start_engine b; sleep 3
B status | grep -q '"session": "loggedIn"' || fail "bob restore" engine-b.log
B rooms.list | grep -q '"name": "alice"' || fail "bob rooms after restart"
B room.open roomId="$ROOM" initialItems:=60 | result || fail "bob room.open after restart"
A message.send roomId="$ROOM" body="after restart" | result
sleep 4
python3 -c "import json; h=json.load(open('$W/b/sigil/sigil-history.json'))['$ROOM']; assert h[-1]['body']=='after restart', h[-1]" || fail "bob missed message after restart" engine-b.log
# link a second device for alice: engine c shows an offer, engine a scans and confirms
mkdir -p "$W/c"; start_engine c; sleep 2.5
C() { SIGIL_SOCKET=$W/run/c.sock $EN cli "$@"; }
timeout 60 env SIGIL_SOCKET=$W/run/c.sock $EN cli ping --follow >c-events.json 2>&1 &
OFFER=$(C link.offer username=@alice:sigil.test envoy=ws://127.0.0.1:18444/envoy | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['offer'])")
sleep 1
SAS_A=$(A link.scan offer="$OFFER" | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['sas'])")
A link.confirm ok:=true | result || fail "link.confirm" engine-a.log
sleep 4
python3 - "$SAS_A" <<'PY' || fail "link events at the new device" c-events.json
import json,sys
raw=open('c-events.json').read(); dec=json.JSONDecoder(); i=0; states=[]; sas=None
while i<len(raw):
    while i<len(raw) and raw[i] in ' \n\r\t': i+=1
    if i>=len(raw): break
    try: d,i=dec.raw_decode(raw,i)
    except Exception: break
    if d.get('event')=='link.state':
        states.append(d['state'])
        if d['state']=='sas': sas=d['sas']
assert 'done' in states, states
assert sas==sys.argv[1], (sas, sys.argv[1])
PY
C status | grep -q '"userId": "@alice:sigil.test"' || fail "linked device not signed in" engine-c.log
C rooms.list | grep -q '"name": "bob"' || fail "linked device has no conversation" engine-c.log
C room.open roomId="$ROOM" initialItems:=60 | result || fail "linked device room.open"
# the linked device sends; alice's first device and bob both see it as alice
C message.send roomId="$ROOM" body="from the second device" | result || fail "linked device send" engine-c.log
sleep 4
python3 -c "import json; h=json.load(open('$W/a/sigil/sigil-history.json'))['$ROOM']; it=h[-1]; assert it['body']=='from the second device' and it['isOwn'], it" || fail "first device did not see the linked device's message" engine-a.log
python3 -c "import json; h=json.load(open('$W/b/sigil/sigil-history.json'))['$ROOM']; it=h[-1]; assert it['body']=='from the second device' and it['sender']=='@alice:sigil.test', it" || fail "bob did not see the linked device's message" engine-b.log
# and bob's reply reaches both of alice's devices
B message.send roomId="$ROOM" body="hello both" | result
sleep 4
for d in a c; do python3 -c "import json; h=json.load(open('$W/$d/sigil/sigil-history.json'))['$ROOM']; assert h[-1]['body']=='hello both', h[-1]" || fail "device $d missed bob's reply" engine-$d.log; done
! grep -q "ERROR" engine-a.log engine-b.log engine-c.log || fail "engine logged errors" engine-a.log engine-b.log engine-c.log
echo "e2e-sigil ok"
