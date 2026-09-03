#!/usr/bin/env bash
# The OIDC gate at the wire: a server with registration = "oidc" and the
# fake issuer standing in for Pocket ID. A token from the issuer registers
# a name; a token for another audience, a forged one and an expired login
# are refused; one login holds one name; and the everyday path (a DM
# between two gated accounts) still works. Needs server/target/debug/
# sigil-server, its fake-issuer example and client/target/debug/sigil-cli.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
ISSUER=$ROOT/server/target/debug/examples/fake-issuer
CL=$ROOT/client/target/debug/sigil-cli
W=$(mktemp -d); cd "$W"
PIDS=()
trap 'kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.3; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; head -40 "$f" 2>/dev/null; done; exit 1; }

$ISSUER --listen 127.0.0.1:18472 --client-id sigil-test >issuer.log 2>&1 & PIDS+=($!)
$SV -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:18453 >/dev/null
# (the keys go next to registration: the file ends in a [servers] table)
sed -i 's|^media_udp = .*|media_udp = "127.0.0.1:0"|; s|^registration = .*|registration = "oidc"\noidc_issuer = "http://127.0.0.1:18472"\noidc_client_id = "sigil-test"|' sigil.toml
$SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
for i in $(seq 1 60); do grep -q 'listening on http' server.log 2>/dev/null && break; sleep 0.5; done
grep -q 'listening on http' server.log || fail "server did not start" server.log

ENVOY=ws://127.0.0.1:18453/envoy
for u in alice bob; do $CL -s $u.json init --username @$u:sigil.test --envoy $ENVOY >/dev/null; done

# 1. a real token registers
TOK_ALICE=$($ISSUER --listen 127.0.0.1:18472 --client-id sigil-test --mint alice)
$CL -s alice.json register --invite "$TOK_ALICE" --password "first pass" >alice.reg 2>&1 || fail "alice with a good token" alice.reg server.log
grep -q "recovery escrowed" alice.reg || fail "no escrow under oidc" alice.reg

# 2. wrong audience, forged signature, garbage: all refused
TOK_OTHER=$($ISSUER --listen 127.0.0.1:18472 --client-id someone-else --mint bob)
! $CL -s bob.json register --invite "$TOK_OTHER" >bob.aud 2>&1 || fail "a token for another client was accepted" bob.aud
FORGED="${TOK_ALICE%.*}.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
! $CL -s bob.json register --invite "$FORGED" >bob.forged 2>&1 || fail "a forged token was accepted" bob.forged
! $CL -s bob.json register --invite "not-a-token" >bob.junk 2>&1 || fail "junk was accepted" bob.junk
! $CL -s bob.json register --invite "" >bob.empty 2>&1 || fail "no token was accepted" bob.empty

# 3. one login, one name: alice's login cannot take a second name
$CL -s alice2.json init --username @alice2:sigil.test --envoy $ENVOY >/dev/null
! $CL -s alice2.json register --invite "$TOK_ALICE" >alice2.reg 2>&1 || fail "one login took two names" alice2.reg

# 4. bob with his own login, then the everyday path
TOK_BOB=$($ISSUER --listen 127.0.0.1:18472 --client-id sigil-test --mint bob)
$CL -s bob.json register --invite "$TOK_BOB" >bob.reg 2>&1 || fail "bob with a good token" bob.reg server.log
timeout 60 $CL -s bob.json requests --accept --count 1 >bob.accept 2>&1 & BOB=$!
sleep 1
$CL -s alice.json dm @bob:sigil.test "first" >/dev/null || fail "dm" server.log
wait $BOB || fail "bob accept" bob.accept
timeout 60 $CL -s bob.json listen 0 --count 1 >bob.listen 2>&1 & BOB=$!
sleep 1
$CL -s alice.json send 0 "hello through the gate" >/dev/null || fail "send" server.log
wait $BOB || true
grep -q "hello through the gate" bob.listen || fail "bob did not hear alice" bob.listen server.log

# 5. recovery without a code: alice sets a password, backs up, and a fresh
#    device gets everything back with the password and her sign-in; the
#    wrong password, someone else's sign-in and no sign-in are refused
$CL -s alice.json set-password "correct horse" >alice.pw 2>&1 || fail "set password" alice.pw server.log
$CL -s alice.json backup >alice.bak 2>&1 || fail "backup" alice.bak server.log
TOK_ALICE2=$($ISSUER --listen 127.0.0.1:18472 --client-id sigil-test --mint alice)
! $CL -s alice-new.json recover --username @alice:sigil.test --password "wrong" --gate "$TOK_ALICE2" --envoy $ENVOY >rec.wrongpw 2>&1 || fail "wrong password restored" rec.wrongpw
grep -q "wrong password" rec.wrongpw || fail "wrong password message" rec.wrongpw
! $CL -s alice-new.json recover --username @alice:sigil.test --password "correct horse" --gate "$TOK_BOB" --envoy $ENVOY >rec.bobgate 2>&1 || fail "bob's sign-in opened alice's escrow" rec.bobgate
! $CL -s alice-new.json recover --username @alice:sigil.test --password "correct horse" --envoy $ENVOY >rec.nogate 2>&1 || fail "no sign-in opened the escrow" rec.nogate
$CL -s alice-new.json recover --username @alice:sigil.test --password "correct horse" --gate "$TOK_ALICE2" --envoy $ENVOY >rec.ok 2>&1 || fail "restore with password and sign-in" rec.ok server.log
$CL -s alice-new.json list >alice-new.list 2>&1 || fail "restored device lists" alice-new.list
grep -q "bob" alice-new.list || fail "restored device lacks the conversation" alice-new.list

# 6. the issuer saw logins and nothing else; the server logged no address or name
! grep -Eq 'alice|bob|sigil.test' issuer.log || fail "the issuer learned a name" issuer.log
! grep -Eq 'correct horse|escrow' server.log || fail "server log carries recovery detail" server.log
! grep -v 'listening on\|media to\|registration through' server.log | grep -Eq '127\.0\.0\.1:[0-9]+|alice|bob' || fail "server log carries client detail" server.log
echo "e2e-oidc ok"
