#!/usr/bin/env bash
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd -- "$repo"
scratch=$(mktemp -d /tmp/sigil-call-relay.XXXXXX)
container=
cleanup() {
  if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null 2>&1 || true; fi
  rm -f -- "$scratch/cert.pem" "$scratch/key.pem" "$scratch/calls.json" "$scratch/client.json"
  rmdir -- "$scratch"
}
trap cleanup EXIT
if ! cargo test --locked --release -p sigil-calls --features forwarder --test interop --no-run --message-format=json > "$scratch/calls.json"; then
  jq -r 'select(.reason=="compiler-message") | .message.rendered // empty' "$scratch/calls.json" >&2
  exit 1
fi
if ! cargo test --locked --release -p sigil-client --lib --no-run --message-format=json > "$scratch/client.json"; then
  jq -r 'select(.reason=="compiler-message") | .message.rendered // empty' "$scratch/client.json" >&2
  exit 1
fi
calls=$(jq -r 'select(.reason=="compiler-artifact" and .target.name=="interop" and .executable!=null) | .executable' "$scratch/calls.json")
client=$(jq -r 'select(.reason=="compiler-artifact" and .target.name=="sigil_client" and .profile.test and .executable!=null) | .executable' "$scratch/client.json")
test -x "$calls"
test -x "$client"
openssl x509 -inform DER -in client/tests/fixtures/synthetic-server.der -out "$scratch/cert.pem"
openssl pkey -inform DER -in client/tests/fixtures/synthetic-server-key.der -out "$scratch/key.pem"
chmod 755 "$scratch"
chmod 644 "$scratch/cert.pem" "$scratch/key.pem"
for auth in static rest; do
  if [[ "$auth" == static ]]; then
    credentials=(--lt-cred-mech --user=synthetic:synthetic-test-secret)
  else
    credentials=(--use-auth-secret --static-auth-secret=synthetic-rest-secret-for-call-tests)
  fi
  container=$(docker run --detach --network host --read-only --cap-drop ALL --cap-add NET_BIND_SERVICE \
    --security-opt no-new-privileges:true --tmpfs /tmp:rw,noexec,nosuid,size=8m --memory 128m --pids-limit 64 \
    --mount "type=bind,source=$scratch,target=/test-tls,readonly" --entrypoint turnserver \
    coturn/coturn@sha256:aa68aab64a3b929d57fc2924c98ea447bf996cf8dade2508e7b71eaf23f1f14e \
    -n --listening-ip=127.0.0.1 --relay-ip=127.0.0.1 --listening-port=39781 --tls-listening-port=39813 \
    --min-port=39782 --max-port=39812 --realm=synthetic "${credentials[@]}" \
    --cert=/test-tls/cert.pem --pkey=/test-tls/key.pem --no-dtls --no-cli --allow-loopback-peers \
    --no-multicast-peers --relay-threads=1 --log-file=stdout --pidfile=/tmp/turn.pid)
  deadline=$((SECONDS + 10))
  until (exec 3<>/dev/tcp/127.0.0.1/39781) 2>/dev/null; do
    if ((SECONDS >= deadline)); then docker logs "$container"; exit 1; fi
    sleep 0.1
  done
  for url in 'turn:127.0.0.1:39781?transport=udp' 'turn:127.0.0.1:39781?transport=tcp' 'turns:127.0.0.1:39813?transport=tcp'; do
    if [[ "$auth" == static ]]; then
      SIGIL_TEST_TURN_URL="$url" "$calls" same_turn_relay_interoperability --exact --ignored --nocapture
    else
      SIGIL_TEST_TURN_URL="$url" "$client" calls::network_tests::native_rest_turn_media_and_server_restart --exact --ignored --nocapture
    fi
  done
  docker rm -f "$container" >/dev/null
  container=
done
