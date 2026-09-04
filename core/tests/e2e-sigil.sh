#!/usr/bin/env bash
# Two engine daemons on one Sigil server, driven over the JSON protocol the
# frontends use: account.create, users.search, dm.create, the invite room,
# room.join, message.send both ways, a reaction, a restart, and a link card
# fetched from a web page served on loopback.
# Needs server/target/debug/sigil-server, server/target/debug/examples/
# static-site (`cargo build --example static-site` in server/) and, from
# `cargo build --bin sigil-engine --bin sigil-jq` in core/, sigil-engine and
# sigil-jq (the JSON query that reads the replies and the history files here).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
SS=$ROOT/server/target/debug/examples/static-site
EN=$ROOT/core/target/debug/sigil-engine
JQ=$ROOT/core/target/debug/sigil-jq
# Ports are drawn rather than fixed: two of these suites on one machine used
# to land on the same server and the second one's first call came back 401.
PORT=$((20000 + RANDOM % 20000)); SITE_PORT=$((PORT + 1))
W=$(mktemp -d); mkdir -p "$W/a" "$W/b" "$W/run"; cd "$W"; export W
PIDS=()
trap 'kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -40; done; exit 1; }
result() { $JQ --assert ok; }
H() { local d=$1; shift; $JQ -f "$W/$d/sigil/sigil-history.json" "$@"; }   # H <engine> <sigil-jq args>: that engine's history

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:${PORT} >/dev/null
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
# wait for the server rather than guess: under load `invite` would otherwise
# fall back to opening the database the server is opening
for i in $(seq 1 60); do grep -q 'listening on http' server.log 2>/dev/null && break; sleep 0.5; done
IA=$($SV -c sigil.toml invite); IB=$($SV -c sigil.toml invite)
start_engine() { # name
  XDG_STATE_HOME=$W/$1 XDG_CACHE_HOME=$W/$1/cache SIGIL_SOCKET=$W/run/$1.sock $EN daemon --log-level info >>engine-$1.log 2>&1 & PIDS+=($!)
}
A() { SIGIL_SOCKET=$W/run/a.sock $EN cli "$@"; }
B() { SIGIL_SOCKET=$W/run/b.sock $EN cli "$@"; }
start_engine a; start_engine b; sleep 2.5

A account.create username=@alice:sigil.test invite="$IA" envoy=ws://127.0.0.1:${PORT}/envoy password="correct horse" | result || fail "alice account" engine-a.log
B account.create username=@bob:sigil.test   invite="$IB" envoy=ws://127.0.0.1:${PORT}/envoy | result || fail "bob account" engine-b.log
A status | grep -q '"session": "loggedIn"' || fail "alice not logged in"
# Finding someone to write to. All three ways of typing a name reach the
# same person: bare, with the @, and with the server. Alice knows nobody
# yet, so each of these is the front desk answering about a whole name.
for Q in bob @bob @bob:sigil.test bob:sigil.test BOB; do
  A users.search query="$Q" | $JQ --assert 'result.results[0].userId == "@bob:sigil.test"' \
    || fail "users.search did not find bob for '$Q'" engine-a.log
done
A users.search query=nobody | $JQ --assert '! result.results.length' || fail "users.search invented someone"
A users.search query="@bob:sigil.test" | $JQ --assert '! result.results[0].known' || fail "bob is not someone alice knows yet"
# Suggestions, before there is anyone to suggest: an empty query asks the
# server nothing (there is no directory to list) and so comes back empty.
A users.search query="" | $JQ --assert '! result.results.length' || fail "an empty query invented suggestions"
ROOM=$(A dm.create userId=@bob:sigil.test | $JQ result.roomId)
R="[\"$ROOM\"]"   # the room's list in a history file
sleep 4
REQ=$(B rooms.list | $JQ 'result.rooms[?isInvite].id') || fail "bob has no invite" engine-b.log
B room.join roomIdOrAlias="$REQ" | grep -q "\"roomId\": \"$ROOM\"" || fail "room.join"
# Now that they have a conversation, bob is someone alice knows: he is what
# the Start page suggests with nothing typed, and a fragment of his name
# finds him without the front desk being asked about "bo" at all.
A users.search query="" | $JQ --assert 'result.results[?userId=="@bob:sigil.test"].known' \
  || fail "the suggestions do not hold the person alice is talking to" engine-a.log
A users.search query=bo | $JQ --assert 'result.results[?userId=="@bob:sigil.test"].length' \
  || fail "a fragment did not find someone alice knows" engine-a.log
# And it is still alice's own conversation list, not the server's users:
# bob knows alice for the same reason, and nobody knows a name they have
# never met.
B users.search query="" | $JQ --assert 'result.results[?userId=="@alice:sigil.test"].length' \
  || fail "bob's suggestions do not hold alice" engine-b.log
A users.search query=marlowe | $JQ --assert '! result.results.length' || fail "a name nobody has appeared"
A room.open roomId="$ROOM" initialItems:=60 | result || fail "alice room.open"
B room.open roomId="$ROOM" initialItems:=60 | result || fail "bob room.open"
timeout 15 env SIGIL_SOCKET=$W/run/b.sock $EN cli ping --follow >bob-events.json 2>&1 &
sleep 1
A message.send roomId="$ROOM" body="hello bob, red::this is red;" | result || fail "message.send"
sleep 3
B message.send roomId="$ROOM" body="hi alice" | result || fail "bob message.send"
sleep 3
ITEMS="[][?event==\"timeline.diff\"][?roomId==\"$ROOM\"].ops[].item"
$JQ --stream -f bob-events.json \
  --assert "$ITEMS[?sender==\"@alice:sigil.test\"][?body==\"hello bob, this is red\"][?effects].length" \
  --assert "$ITEMS[?sender==\"@bob:sigil.test\"][?body==\"hi alice\"][?!effects].length" || fail "bob events" bob-events.json
# reaction
EID=$(H a "$R[-1].eventId")
B message.react roomId="$ROOM" eventId="$EID" key=👍 | result || fail "react"
sleep 3
H a --assert "$R[-1].reactions[0].key == \"👍\"" || fail "reaction not applied at alice"
# the local echo, under a send that is still in the air. A send holds the
# account across its round trips; everything that only reads the account —
# the next message's row, a place's row, the room list — must not queue
# behind it. Three sends go out staggered and nothing waits on them.
A message.send roomId="$ROOM" body="echo one" >/dev/null & E1=$!
sleep 0.3
A message.send roomId="$ROOM" body="echo two" >/dev/null & E2=$!
sleep 0.3
A location.send roomId="$ROOM" lat:=51.5007 lon:=-0.1246 description="the meeting point" >/dev/null & E3=$!
sleep 0.5
H a --assert "$R[?body==\"echo two\"].sendState == \"sending\"" \
  || fail "the second message had no row until the first had gone out" engine-a.log
H a --assert "$R[?kind==\"location\"].sendState == \"sending\"" \
  || fail "a place had no row until it had gone out" engine-a.log
T0=$(date +%s%N); A rooms.list >/dev/null || fail "rooms.list while sending" engine-a.log; T1=$(date +%s%N)
LIST_MS=$(( (T1 - T0) / 1000000 ))
[ "$LIST_MS" -lt 800 ] || fail "the room list waited ${LIST_MS} ms behind sends in flight" engine-a.log
wait $E1 $E2 $E3
sleep 4
H a --assert "$R[?body==\"echo two\"].sendState == \"sent\"" || fail "the echo never settled" engine-a.log
H a --assert "$R[?kind==\"location\"].sendState == \"sent\"" || fail "the place never settled" engine-a.log
H b --assert "$R[][?body==\"echo two\"].length" || fail "bob missed echo two" engine-b.log
H b --assert "$R[][?kind==\"location\"].length" || fail "bob missed the place" engine-b.log
# a poll, and a vote that counts on the tap. The tally is filed under the
# name of whoever cast it, so our own copy coming back off the slot lands on
# the same numbers and nothing is counted twice.
A poll.create roomId="$ROOM" question="tea or coffee" options:='["tea","coffee"]' | result || fail "poll.create" engine-a.log
POLL=$(H a "$R[?kind==\"poll\"].eventId")
A poll.vote roomId="$ROOM" eventId="$POLL" answers:='["1"]' >/dev/null & PV=$!
sleep 0.5
H a --assert "$R[?kind==\"poll\"].poll.answers[1].mine" \
    --assert "$R[?kind==\"poll\"].poll.answers[1].votes == 1" \
  || fail "the vote did not count until it had gone out" engine-a.log
wait $PV
sleep 4
H a --assert "$R[?kind==\"poll\"].poll.answers[1].votes == 1" --assert "$R[?kind==\"poll\"].poll.voters == 1" \
  || fail "the vote did not settle to one" engine-a.log
H b --assert "$R[?kind==\"poll\"].poll.answers[1].votes == 1" || fail "bob did not see the vote" engine-b.log
# restart bob's engine: session, rooms, history come back
kill "${PIDS[-1]}" 2>/dev/null || true; sleep 1
start_engine b; sleep 3
B status | grep -q '"session": "loggedIn"' || fail "bob restore" engine-b.log
B rooms.list | grep -q '"name": "alice"' || fail "bob rooms after restart"
B room.open roomId="$ROOM" initialItems:=60 | result || fail "bob room.open after restart"
A message.send roomId="$ROOM" body="after restart" | result
sleep 4
H b --assert "$R[-1].body == \"after restart\"" || fail "bob missed message after restart" engine-b.log
# link a second device for alice: engine c shows an offer, engine a scans and confirms
mkdir -p "$W/c"; start_engine c; sleep 2.5
C() { SIGIL_SOCKET=$W/run/c.sock $EN cli "$@"; }
timeout 60 env SIGIL_SOCKET=$W/run/c.sock $EN cli ping --follow >c-events.json 2>&1 &
OFFER=$(C link.offer username=@alice:sigil.test envoy=ws://127.0.0.1:${PORT}/envoy | $JQ result.offer)
sleep 1
SAS_A=$(A link.scan offer="$OFFER" | $JQ result.sas)
A link.confirm ok:=true | result || fail "link.confirm" engine-a.log
for i in $(seq 1 30); do grep -q '"state": "done"' c-events.json && break; sleep 1; done
$JQ --stream -f c-events.json \
  --assert '[][?event=="link.state"][?state=="done"].length' \
  --assert "[][?event==\"link.state\"][?state==\"sas\"][-1].sas == \"$SAS_A\"" || fail "link events at the new device" c-events.json
C status | grep -q '"userId": "@alice:sigil.test"' || fail "linked device not signed in" engine-c.log
C rooms.list | grep -q '"name": "bob"' || fail "linked device has no conversation" engine-c.log
C room.open roomId="$ROOM" initialItems:=60 | result || fail "linked device room.open"
# the linked device sends; alice's first device and bob both see it as alice
C message.send roomId="$ROOM" body="from the second device" | result || fail "linked device send" engine-c.log
sleep 4
H a --assert "$R[-1].body == \"from the second device\"" --assert "$R[-1].isOwn" || fail "first device did not see the linked device's message" engine-a.log
H b --assert "$R[-1].body == \"from the second device\"" --assert "$R[-1].sender == \"@alice:sigil.test\"" || fail "bob did not see the linked device's message" engine-b.log
# and bob's reply reaches both of alice's devices
B message.send roomId="$ROOM" body="hello both" | result
sleep 4
for d in a c; do H $d --assert "$R[-1].body == \"hello both\"" || fail "device $d missed bob's reply" engine-$d.log; done
# recovery through the engine: the backup loop has run; a fresh engine restores
A recovery.status | grep -q '"recovery": "enabled"' || fail "recovery not enabled"
CODE=$(A recovery.code | $JQ result.code)
sleep 7   # backup loop cadence
A recovery.status | grep -q '"backup": "enabled"' || fail "backup not uploaded" engine-a.log
mkdir -p "$W/d"; start_engine d; sleep 2.5
D() { SIGIL_SOCKET=$W/run/d.sock $EN cli "$@"; }
if D account.recover username=@alice:sigil.test password=wrong code="$CODE" envoy=ws://127.0.0.1:${PORT}/envoy | result 2>/dev/null; then fail "wrong password accepted by engine"; fi
sleep 3
D account.recover username=@alice:sigil.test password="correct horse" code="$CODE" envoy=ws://127.0.0.1:${PORT}/envoy | result || fail "account.recover" engine-d.log
D status | grep -q '"userId": "@alice:sigil.test"' || fail "recovered engine not signed in"
D rooms.list | grep -q '"name": "bob"' || fail "recovered engine has no conversation"
H d --assert "$R[][?body==\"hello both\"].length" || fail "recovered history missing" engine-d.log
# A restored device is a clone of the lost one; the two are not meant to
# run side by side (see "removing a device" in docs/blind-backend.md), so
# the clone bows out before the conversation moves on.
D logout wipe:=false >/dev/null 2>&1 || true
kill "${PIDS[-1]}" 2>/dev/null || true; sleep 1
# a group through the engine, and a file
GROUP=$(A room.create name="the plan" invite:='["@bob:sigil.test"]' | $JQ result.roomId) || fail "room.create" engine-a.log
G="[\"$GROUP\"]"
sleep 4
GREQ=$(B rooms.list | $JQ 'result.rooms[?isInvite].id') || fail "bob has no group invite" engine-b.log
B room.join roomIdOrAlias="$GREQ" | grep -q "\"roomId\": \"$GROUP\"" || fail "group join"
sleep 3
B rooms.list | $JQ --assert "result.rooms[?id==\"$GROUP\"].name == \"the plan\"" --assert "! result.rooms[?id==\"$GROUP\"].isDm" || fail "group room shape at bob"
A room.setSettings roomId="$GROUP" name="the better plan" | result || fail "rename"
sleep 3
B rooms.list | grep -q '"name": "the better plan"' || fail "rename not seen by bob" engine-b.log
B room.open roomId="$GROUP" initialItems:=60 | result
head -c 200000 /dev/urandom >"$W/pic.bin"
A attachment.send roomId="$GROUP" path="$W/pic.bin" caption="a file" | result || fail "attachment.send" engine-a.log
sleep 6
H b --assert "$G[][?kind==\"file\"][-1].media.size == 200000" || fail "bob did not download the file" engine-b.log
FILE=$(H b "$G[][?kind==\"file\"][-1].media.path")
[ -n "$FILE" ] && [ "$(stat -c %s "$FILE")" = 200000 ] || fail "bob's copy of the file is not whole" engine-b.log
# edit and delete: alice changes her words, then takes one message back
A message.send roomId="$ROOM" body="a typo hree" | result || fail "send for edit"
sleep 3
EID=$(H a "$R[-1].eventId")
A message.edit roomId="$ROOM" eventId="$EID" body="a typo here, fixed" | result || fail "message.edit" engine-a.log
sleep 4
H b --assert "$R[?eventId==\"$EID\"].body == \"a typo here, fixed\"" --assert "$R[?eventId==\"$EID\"].isEdited" || fail "bob did not see the edit" engine-b.log
if B message.edit roomId="$ROOM" eventId="$EID" body="not mine" | result 2>/dev/null; then fail "bob edited alice's message"; fi
A message.redact roomId="$ROOM" eventId="$EID" | result || fail "message.redact" engine-a.log
sleep 4
H b --assert "$R[?eventId==\"$EID\"].kind == \"redacted\"" || fail "bob did not see the deletion" engine-b.log
H a --assert "$R[?eventId==\"$EID\"].kind == \"redacted\"" || fail "alice did not apply her own deletion"
# a call: announced in the group, seen by bob, and signalling reaches the
# forwarding unit (the engine has no media stack, so a bad offer is refused)
CALL=$(A call.start roomId="$GROUP" | $JQ result.callId) || fail "call.start" engine-a.log
sleep 4
H b --assert "$G[-1].kind == \"call\"" --assert "$G[-1].callId == \"$CALL\"" --assert "$G[-1].callState == \"started\"" || fail "bob did not see the call" engine-b.log
A call.join roomId="$GROUP" callId="$CALL" offer="not an offer" | grep -q "bad offer" || fail "call.join did not reach the forwarding unit" engine-a.log server.log
A call.poll roomId="$GROUP" callId="$CALL" peer="00000000000000000000000000000000" | grep -q "unknown peer" || fail "call.poll"
A call.end roomId="$GROUP" callId="$CALL" | result || fail "call.end"
sleep 3
H b --assert "$G[-1].callState == \"ended\"" || fail "bob did not see the call end"
# the server restarts under everyone: the links reconnect on their own and
# the Envoy drains what it queued meanwhile
kill "${PIDS[0]}" 2>/dev/null || true; sleep 1
$SV -c sigil.toml run >>server.log 2>&1 & PIDS+=($!)
sleep 8
A message.send roomId="$ROOM" body="after the server came back" | result || fail "send after server restart" engine-a.log
sleep 6
H b --assert "$R[-1].body == \"after the server came back\"" || fail "bob missed the message after the server restart" engine-b.log server.log
# a link card, from a page served on loopback — a committed test never
# reaches the real web. The page hides its metadata behind 700 KB of filler,
# where a video site's own <title> sits: the read has to follow the head that
# far and stop there, and the cached image has to be named for the format it
# is or nothing downstream can decode it.
mkdir -p "$W/www"
base64 -d >"$W/www/pic.png" <<'PNG'
iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAfElEQVR4Ae3AA6AkWZbG8f937o3IzKdy
S2Oubdu2bdu2bdu2bWmMnpZKr54yMyLu+Xa3anqmhztr1a/CZx+H43AcjsNxOA7H4Tgch+NwHI7DcTgO
x6FyE/8aVG7iX4PKTfxrULmJfw0qN/GvQeUm/jWo3MS/BpWb+NfgHwFEigJ1ymuMuQAAAABJRU5ErkJg
gg==
PNG
{ printf '<!doctype html><html><head><!-- '
  head -c 700000 /dev/zero | tr '\0' 'x'
  printf ' -->\n<title>fallback</title>\n'
  printf '<meta property="og:title" content="A page with a card">\n'
  printf '<meta property="og:description" content="Served on loopback for the preview test.">\n'
  printf '<meta property="og:site_name" content="Example">\n'
  printf '<meta property="og:image" content="/pic.png">\n'
  printf '</head><body>the words after the head are never read</body></html>\n'
} >"$W/www/page.html"
$SS 127.0.0.1:${SITE_PORT} "$W/www" >site.log 2>&1 & PIDS+=($!)
UP=""
for i in $(seq 1 30); do if grep -q "static site at" site.log 2>/dev/null; then UP=1; break; fi; sleep 0.5; done
[ -n "$UP" ] || fail "the page server did not start" site.log
PAGE=http://127.0.0.1:${SITE_PORT}/page.html
# off by default: the site learns the device's address, so nothing is fetched
A link.preview url="$PAGE" | grep -q '"code": "disabled"' || fail "a card was fetched with the switch off" site.log
! grep -q "GET /page.html" site.log || fail "the page was fetched with the switch off" site.log
A shape.settings linkPreviews:=true | $JQ --assert result.linkPreviews || fail "link previews would not turn on"
A link.preview url="$PAGE" \
  | $JQ --assert 'result.title == "A page with a card"' \
        --assert 'result.description == "Served on loopback for the preview test."' \
        --assert 'result.siteName == "Example"' \
        --assert result.imagePath \
        --assert 'result.imageWidth == 16' --assert 'result.imageHeight == 9' \
  || fail "no card for the page" site.log engine-a.log
A shape.settings linkPreviews:=false >/dev/null
! grep -q "ERROR" engine-a.log engine-b.log engine-c.log engine-d.log || fail "engine logged errors" engine-a.log engine-b.log engine-c.log engine-d.log
echo "e2e-sigil ok"
