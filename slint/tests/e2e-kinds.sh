#!/usr/bin/env bash
# Everything beyond text and files, for real: Bob (the Slint app) pins,
# asks a poll, answers in a thread, sends a sticker, a contact card and a
# place, and gets a link preview from a page served on loopback; Alice on
# the command-line client votes, answers in the thread and shares a place
# when the drive prints the ids she needs, and sees every event arrive.
# Needs the same binaries as e2e-home.sh, plus server/target/debug/examples/
# static-site (`cargo build --example static-site` in server/) for the web page.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
CL=$ROOT/client/target/debug/sigil-cli
DRIVE=$ROOT/slint/target/debug/drive
SITE=$ROOT/server/target/debug/examples/static-site
W=$(mktemp -d); mkdir -p "$W/state" "$W/cache" "$W/shots" "$W/www"; cd "$W"
PIDS=()
keep_logs() { if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/*.out "$W"/*.err "$W"/*.log "$W"/shots/*.png "$KEEP_SHOTS"/ 2>/dev/null || true; fi; }
trap 'keep_logs; kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -80; done; exit 1; }
waitline() { # waitline <pattern> <file> <seconds>
  for i in $(seq 1 "$3"); do grep -q "$1" "$2" 2>/dev/null && return 0; sleep 1; done; return 1; }

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18449 >/dev/null
sed -i 's|^media_udp = .*|media_udp = "127.0.0.1:0"|' sigil.toml
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
base64 -d >www/pic.png <<'PNG'
iVBORw0KGgoAAAANSUhEUgAAABAAAAAJCAIAAAC0SDtlAAAAfElEQVR4Ae3AA6AkWZbG8f937o3IzKdy
S2Oubdu2bdu2bdu2bWmMnpZKr54yMyLu+Xa3anqmhztr1a/CZx+H43AcjsNxOA7H4Tgch+NwHI7DcTgO
x6FyE/8aVG7iX4PKTfxrULmJfw0qN/GvQeUm/jWo3MS/BpWb+NfgHwFEigJ1ymuMuQAAAABJRU5ErkJg
gg==
PNG
cat >www/page.html <<'HTML'
<html><head><title>fallback</title>
<meta property="og:title" content="A Sigil test page">
<meta property="og:description" content="Served on loopback for the preview test.">
<meta property="og:image" content="/pic.png">
</head><body>hello</body></html>
HTML
$SITE 127.0.0.1:18450 www >www.log 2>&1 & PIDS+=($!)
sleep 1.5
IA=$($SV -c sigil.toml invite); IB=$($SV -c sigil.toml invite)

XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache HOME=$W \
  timeout 420 "$DRIVE" "$W/shots" 127.0.0.1:18449 "$IB" bob kinds >drive.out 2>drive.err & PIDS+=($!)
DRIVE_PID=$!
waitline "signed in as @bob:sigil.test" drive.out 60 || fail "bob did not sign in" drive.out drive.err

$CL -s alice.json init --username @alice:sigil.test --envoy ws://127.0.0.1:18449/envoy >/dev/null
$CL -s alice.json register --invite "$IA" >/dev/null || fail "alice register" server.log
$CL -s alice.json dm @bob:sigil.test "hello from alice" >/dev/null || fail "alice dm" server.log

# Alice plays her part as the drive asks for it. Each `event` also prints
# what arrived meanwhile, so the outputs together are everything she saw.
waitline "^poll " drive.out 180 || fail "no poll" drive.out drive.err
POLL=$(grep "^poll " drive.out | head -1 | cut -d' ' -f2)
$CL -s alice.json event 0 16 '{"ids":["1"]}' --reference "$POLL" >alice1.out 2>&1 || fail "alice vote" alice1.out
waitline "^thread " drive.out 180 || fail "no thread" drive.out drive.err
ROOT_ID=$(grep "^thread " drive.out | head -1 | cut -d' ' -f2)
$CL -s alice.json event 0 1 "alice in the thread" --reference "thread:$ROOT_ID" >alice2.out 2>&1 || fail "alice thread reply" alice2.out
waitline "^location sent" drive.out 240 || fail "no location" drive.out drive.err
$CL -s alice.json event 0 18 '{"lat":48.8566,"lon":2.3522,"description":"Paris"}' >alice3.out 2>&1 || fail "alice place" alice3.out
wait "$DRIVE_PID" || fail "drive" drive.out drive.err
timeout 25 $CL -s alice.json listen 0 --count 50 >alice4.out 2>&1 || true
cat alice1.out alice2.out alice3.out alice4.out >alice-all.out

grep -q "^drive kinds ok" drive.out || fail "drive did not finish" drive.out drive.err
grep -q "policy updated by @bob:sigil.test" alice-all.out || fail "alice did not see the pin" alice-all.out
grep -q "(event kind 15)" alice-all.out || fail "alice did not get the poll" alice-all.out
grep -q "(event kind 16)" alice-all.out || fail "alice did not get bob's vote" alice-all.out
grep -q "(event kind 17)" alice-all.out || fail "alice did not get the poll end" alice-all.out
grep -q "@bob:sigil.test: in the thread" alice-all.out || fail "alice did not get the thread reply" alice-all.out
grep -q "\[file\] smile.png" alice-all.out || fail "alice did not get the sticker" alice-all.out
grep -q "\[file\] alice.vcf" alice-all.out || fail "alice did not get the contact card" alice-all.out
grep -q "(event kind 18)" alice-all.out || fail "alice did not get the place" alice-all.out
grep -q "page.html" alice-all.out || fail "alice did not get the link message" alice-all.out
for p in live-pins live-poll live-poll-ended live-thread live-threads live-thread-chip live-sticker live-member-sheet live-contact live-locpick live-location live-map live-link; do
  [ -s "$W/shots/$p.png" ] || fail "missing capture $p" drive.out
done
if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/shots/*.png "$KEEP_SHOTS"/; fi
echo "e2e-kinds ok"
