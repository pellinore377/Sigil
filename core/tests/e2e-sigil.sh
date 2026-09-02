#!/usr/bin/env bash
# Two engine daemons on one Sigil server, driven over the JSON protocol the
# frontends use: account.create, users.search, dm.create, the invite room,
# room.join, message.send both ways, a reaction, and a restart.
# Needs server/target/debug/sigil-server and core/target/debug/sigil-engine.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
EN=$ROOT/core/target/debug/sigil-engine
W=$(mktemp -d); mkdir -p "$W/a" "$W/b" "$W/run"; cd "$W"; export W
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

A account.create username=@alice:sigil.test invite="$IA" envoy=ws://127.0.0.1:18444/envoy password="correct horse" | result || fail "alice account" engine-a.log
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
for i in $(seq 1 30); do grep -q '"state": "done"' c-events.json && break; sleep 1; done
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
# recovery through the engine: the backup loop has run; a fresh engine restores
A recovery.status | grep -q '"recovery": "enabled"' || fail "recovery not enabled"
CODE=$(A recovery.code | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['code'])")
sleep 7   # backup loop cadence
A recovery.status | grep -q '"backup": "enabled"' || fail "backup not uploaded" engine-a.log
mkdir -p "$W/d"; start_engine d; sleep 2.5
D() { SIGIL_SOCKET=$W/run/d.sock $EN cli "$@"; }
if D account.recover username=@alice:sigil.test password=wrong code="$CODE" envoy=ws://127.0.0.1:18444/envoy | result; then fail "wrong password accepted by engine"; fi
sleep 3
D account.recover username=@alice:sigil.test password="correct horse" code="$CODE" envoy=ws://127.0.0.1:18444/envoy | result || fail "account.recover" engine-d.log
D status | grep -q '"userId": "@alice:sigil.test"' || fail "recovered engine not signed in"
D rooms.list | grep -q '"name": "bob"' || fail "recovered engine has no conversation"
python3 -c "import json; h=json.load(open('$W/d/sigil/sigil-history.json'))['$ROOM']; assert any(i['body']=='hello both' for i in h), h" || fail "recovered history missing" engine-d.log
# A restored device is a clone of the lost one; the two are not meant to
# run side by side (see "removing a device" in docs/blind-backend.md), so
# the clone bows out before the conversation moves on.
D logout wipe:=false >/dev/null 2>&1 || true
kill "${PIDS[-1]}" 2>/dev/null || true; sleep 1
# a group through the engine, and a file
export GROUP; GROUP=$(A room.create name="the plan" invite:='["@bob:sigil.test"]' | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['roomId'])") || fail "room.create" engine-a.log
sleep 4
GREQ=$(B rooms.list | python3 -c "import json,sys; print([r['id'] for r in json.load(sys.stdin)['result']['rooms'] if r['isInvite']][0])") || fail "bob has no group invite" engine-b.log
B room.join roomIdOrAlias="$GREQ" | grep -q "\"roomId\": \"$GROUP\"" || fail "group join"
sleep 3
B rooms.list | python3 -c "import json,sys; r=[x for x in json.load(sys.stdin)['result']['rooms'] if x['id']=='$GROUP'][0]; assert r['name']=='the plan' and not r['isDm'], r" || fail "group room shape at bob"
A room.setSettings roomId="$GROUP" name="the better plan" | result || fail "rename"
sleep 3
B rooms.list | grep -q '"name": "the better plan"' || fail "rename not seen by bob" engine-b.log
B room.open roomId="$GROUP" initialItems:=60 | result
head -c 200000 /dev/urandom >"$W/pic.bin"
A attachment.send roomId="$GROUP" path="$W/pic.bin" caption="a file" | result || fail "attachment.send" engine-a.log
sleep 6
python3 - <<'PY' || fail "bob did not download the file" engine-b.log
import json,os
h=json.load(open(os.environ['W']+'/b/sigil/sigil-history.json'))[os.environ['GROUP']]
it=[i for i in h if i['kind']=='file'][-1]
assert it['media']['size']==200000 and it['media']['path'] and os.path.getsize(it['media']['path'])==200000, it['media']
PY
# edit and delete: alice changes her words, then takes one message back
A message.send roomId="$ROOM" body="a typo hree" | result || fail "send for edit"
sleep 3
EID=$(python3 -c "import json; print(json.load(open('$W/a/sigil/sigil-history.json'))['$ROOM'][-1]['eventId'])")
A message.edit roomId="$ROOM" eventId="$EID" body="a typo here, fixed" | result || fail "message.edit" engine-a.log
sleep 4
python3 -c "import json; h=json.load(open('$W/b/sigil/sigil-history.json'))['$ROOM']; it=[i for i in h if i['eventId']=='$EID'][0]; assert it['body']=='a typo here, fixed' and it['isEdited'], it" || fail "bob did not see the edit" engine-b.log
if B message.edit roomId="$ROOM" eventId="$EID" body="not mine" | result; then fail "bob edited alice's message"; fi
A message.redact roomId="$ROOM" eventId="$EID" | result || fail "message.redact" engine-a.log
sleep 4
python3 -c "import json; h=json.load(open('$W/b/sigil/sigil-history.json'))['$ROOM']; it=[i for i in h if i['eventId']=='$EID'][0]; assert it['kind']=='redacted', it" || fail "bob did not see the deletion" engine-b.log
python3 -c "import json; h=json.load(open('$W/a/sigil/sigil-history.json'))['$ROOM']; it=[i for i in h if i['eventId']=='$EID'][0]; assert it['kind']=='redacted', it" || fail "alice did not apply her own deletion"
# a call: announced in the group, seen by bob, and signalling reaches the
# forwarding unit (the engine has no media stack, so a bad offer is refused)
export CALL; CALL=$(A call.start roomId="$GROUP" | python3 -c "import json,sys; print(json.load(sys.stdin)['result']['callId'])") || fail "call.start" engine-a.log
sleep 4
python3 - <<'PY' || fail "bob did not see the call" engine-b.log
import json,os
h=json.load(open(os.environ['W']+'/b/sigil/sigil-history.json'))[os.environ['GROUP']]
it=h[-1]; assert it['kind']=='call' and it['callId']==os.environ['CALL'] and it['callState']=='started', it
PY
A call.join roomId="$GROUP" callId="$CALL" offer="not an offer" | grep -q "bad offer" || fail "call.join did not reach the forwarding unit" engine-a.log server.log
A call.poll roomId="$GROUP" callId="$CALL" peer="00000000000000000000000000000000" | grep -q "unknown peer" || fail "call.poll"
A call.end roomId="$GROUP" callId="$CALL" | result || fail "call.end"
sleep 3
python3 -c "import json,os; h=json.load(open('$W/b/sigil/sigil-history.json'))['$GROUP']; assert h[-1]['callState']=='ended', h[-1]" || fail "bob did not see the call end"
# the server restarts under everyone: the links reconnect on their own and
# the Envoy drains what it queued meanwhile
kill "${PIDS[0]}" 2>/dev/null || true; sleep 1
$SV -c sigil.toml run >>server.log 2>&1 & PIDS+=($!)
sleep 8
A message.send roomId="$ROOM" body="after the server came back" | result || fail "send after server restart" engine-a.log
sleep 6
python3 -c "import json; h=json.load(open('$W/b/sigil/sigil-history.json'))['$ROOM']; assert h[-1]['body']=='after the server came back', h[-1]" || fail "bob missed the message after the server restart" engine-b.log server.log
! grep -q "ERROR" engine-a.log engine-b.log engine-c.log engine-d.log || fail "engine logged errors" engine-a.log engine-b.log engine-c.log engine-d.log
echo "e2e-sigil ok"
