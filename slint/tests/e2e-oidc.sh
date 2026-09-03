#!/usr/bin/env bash
# The OIDC gate, for real: a sigil-server whose registration is a sign-in
# at an identity provider (the fake issuer stands in for Pocket ID), and
# the Slint app going through the doors: probe, "Sign in with …", the
# browser round-trip (curl plays the browser: the issuer's login page
# redirects straight back to the app's loopback listener), then create.
# Needs server/target/debug/sigil-server, server/target/debug/examples/
# fake-issuer and slint/target/debug/drive.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SV=$ROOT/server/target/debug/sigil-server
ISSUER=$ROOT/server/target/debug/examples/fake-issuer
DRIVE=$ROOT/slint/target/debug/drive
W=$(mktemp -d); mkdir -p "$W/state" "$W/cache" "$W/shots"; cd "$W"
PIDS=()
keep() { if [ -n "${KEEP_SHOTS:-}" ]; then mkdir -p "$KEEP_SHOTS"; cp "$W"/shots/*.png "$W"/*.out "$W"/*.err "$W"/*.log "$KEEP_SHOTS"/ 2>/dev/null || true; fi; }
trap 'keep; kill "${PIDS[@]}" >/dev/null 2>&1 || true; sleep 0.5; rm -rf "$W"' EXIT
fail() { echo "FAIL: $1"; shift; for f in "$@"; do echo "--- $f"; cat "$f" 2>/dev/null | head -60; done; exit 1; }

$ISSUER --listen 127.0.0.1:18471 --client-id sigil-test --user marlowe >issuer.log 2>&1 & PIDS+=($!)

# the server is configured the way a container would be: from the environment
SIGIL_HOSTNAME=sigil.test SIGIL_LISTEN=127.0.0.1:18452 SIGIL_MEDIA_UDP=127.0.0.1:0 \
SIGIL_REGISTRATION=oidc SIGIL_OIDC_ISSUER=http://127.0.0.1:18471 SIGIL_OIDC_CLIENT_ID=sigil-test \
  $SV -c sigil.toml run >server.log 2>&1 & PIDS+=($!)
for i in $(seq 1 60); do grep -q 'listening on http' server.log 2>/dev/null && break; sleep 0.5; done
grep -q 'registration through http://127.0.0.1:18471' server.log || fail "server did not take the gate from its environment" server.log
grep -q '^registration = "oidc"' sigil.toml || fail "config not written from the environment" sigil.toml

# a browser that never shows anything: fetch the page and follow it home
cat >browser.sh <<'EOF'
#!/usr/bin/env bash
( sleep 0.3; curl -sS -L -o /dev/null "$1" ) &
EOF
chmod +x browser.sh

SIGIL_BROWSER=$W/browser.sh XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache HOME=$W \
  timeout 180 "$DRIVE" "$W/shots" 127.0.0.1:18452 "" marlowe oidc >drive.out 2>drive.err || fail "drive" drive.out drive.err server.log issuer.log
grep -q "server offers registration=oidc via 127.0.0.1" drive.out || fail "probe" drive.out
grep -q "signed in at the provider as marlowe" drive.out || fail "sign-in" drive.out drive.err
grep -q "signed in as @marlowe:sigil.test" drive.out || fail "create" drive.out drive.err server.log
grep -q "^drive oidc ok" drive.out || fail "finish" drive.out drive.err
for p in live-door-oidc live-door-oidc-done live-home-oidc; do
  [ -s "$W/shots/$p.png" ] || fail "missing capture $p" drive.out
done
# the provider saw a login and nothing else; the server logged no address
! grep -Eq 'marlowe|sigil.test' issuer.log || fail "the issuer learned a name" issuer.log
! grep -v 'listening on\|media to\|registration through' server.log | grep -Eq '127\.0\.0\.1:[0-9]+' || fail "client address logged" server.log
keep
echo "e2e-oidc ok"
