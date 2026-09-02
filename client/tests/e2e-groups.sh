#!/usr/bin/env bash
# Groups and media over the real protocol: a three-person group, a message
# from each, an invite, a rename, a file, and a member leaving.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
CL=$ROOT/client/target/debug/sigil-cli
W=$(mktemp -d); cd "$W"
trap 'pkill -x sigil-server >/dev/null 2>&1 || true; pkill -x sigil-cli >/dev/null 2>&1 || true; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -40; done; exit 1; }
E=ws://127.0.0.1:18447/envoy

$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18447 >/dev/null
$SV -c sigil.toml run >server.log 2>&1 &
sleep 1.5
for u in alice bob carol dave; do
  I=$($SV -c sigil.toml invite)
  $CL -s $u.json init --username @$u:sigil.test --envoy $E >/dev/null
  $CL -s $u.json register --invite "$I" >/dev/null
  $CL -s $u.json tokens 40 >/dev/null
done

# group of three; bob and carol accept
timeout 25 $CL -s bob.json requests --accept --count 1 >bob_req.out 2>&1 &
timeout 25 $CL -s carol.json requests --accept --count 1 >carol_req.out 2>&1 &
sleep 3
$CL -s alice.json group "the plan" @bob:sigil.test @carol:sigil.test | grep -q "created the plan" || fail "group create"
sleep 5
grep -q "^accepted" bob_req.out || fail "bob accept" bob_req.out
grep -q "^accepted" carol_req.out || fail "carol accept" carol_req.out
$CL -s bob.json list | grep -q '"the plan" (3 members)' || fail "bob's member list" 

# everyone talks; carol sees it all in order
$CL -s alice.json send 0 "one from alice" >/dev/null
$CL -s bob.json send 0 "two from bob" >/dev/null
# carol's send catches up first, so her view of the others is in its output
$CL -s carol.json send 0 "three from carol" >carol1.out
grep -q "@alice:sigil.test: one from alice" carol1.out && grep -q "@bob:sigil.test: two from bob" carol1.out || fail "carol's view" carol1.out
timeout 8 $CL -s alice.json listen 0 --count 9 >alice1.out 2>&1 &
sleep 4
grep -q "@carol:sigil.test: three from carol" alice1.out || fail "alice's view" alice1.out

# invite dave; he joins and sees the group name and members
timeout 25 $CL -s dave.json requests --accept --count 1 >dave_req.out 2>&1 &
sleep 2
$CL -s alice.json invite 0 @dave:sigil.test | grep -q "invited" || fail "invite"
sleep 5
grep -q "^accepted" dave_req.out || fail "dave accept" dave_req.out
$CL -s dave.json list | grep -q '"the plan" (4 members)' || fail "dave's member list"
$CL -s dave.json send 0 "four from dave" >/dev/null
sleep 1
timeout 10 $CL -s bob.json listen 0 --count 9 >bob2.out 2>&1 &
sleep 5
grep -q "@dave:sigil.test: four from dave" bob2.out || fail "bob did not get dave's message" bob2.out
$CL -s bob.json list | grep -q '"the plan" (4 members)' || fail "bob's member list after invite"

# rename
$CL -s alice.json rename 0 "the better plan" >/dev/null
sleep 1
timeout 8 $CL -s carol.json listen 0 --count 9 >/dev/null 2>&1 &
sleep 4
$CL -s carol.json list | grep -q '"the better plan"' || fail "carol did not see the rename"

# a file: 300 KiB of noise, two chunks; dave downloads it on listen
head -c 307200 /dev/urandom >noise.bin
$CL -s alice.json sendfile 0 noise.bin --caption "here" | grep -q "2 chunks" || fail "sendfile"
sleep 1
timeout 15 $CL -s dave.json listen 0 --count 9 >dave2.out 2>&1 &
sleep 8
grep -q "\[file\] noise.bin (307200 bytes" dave2.out || fail "dave did not get the file" dave2.out
cmp -s noise.bin downloads/noise.bin || fail "downloaded file differs" dave2.out

# bob leaves; the lowest remaining identity commits his removal and the
# group carries on without him
$CL -s bob.json leave 0 | grep -q "left" || fail "leave"
sleep 1
timeout 10 $CL -s alice.json listen 0 --count 9 >/dev/null 2>&1 &
timeout 10 $CL -s carol.json listen 0 --count 9 >/dev/null 2>&1 &
timeout 10 $CL -s dave.json listen 0 --count 9 >/dev/null 2>&1 &
sleep 6
$CL -s alice.json list | grep -q '(3 members)' || fail "alice's member list after leave"
$CL -s carol.json send 0 "after bob left" >/dev/null
sleep 1
timeout 10 $CL -s dave.json listen 0 --count 9 >dave3.out 2>&1 &
sleep 5
grep -q "@carol:sigil.test: after bob left" dave3.out || fail "dave after bob left" dave3.out
echo "e2e-groups ok"
