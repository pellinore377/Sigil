#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")/../.."
source app/tests/build.sh
scratch=$(mktemp -d /tmp/sigil-call-device.XXXXXX)
container= fixture_pid= progress_pid= port=
cleanup() {
  local result=$?
  if ((result != 0)) && [[ -f "$scratch/fixture.log" ]]; then cat "$scratch/fixture.log"; fi
  if [[ -n "$fixture_pid" ]]; then kill "$fixture_pid" >/dev/null 2>&1 || true; fi
  if [[ -n "$progress_pid" ]]; then kill "$progress_pid" >/dev/null 2>&1 || true; fi
  if [[ -n "$port" ]]; then "$adb" reverse --remove "tcp:$port" >/dev/null 2>&1 || true; fi
  "$adb" reverse --remove tcp:39813 >/dev/null 2>&1 || true
  "$adb" shell rm -f /data/local/tmp/sigil-call-fixture.db >/dev/null 2>&1 || true
  "$adb" shell pm clear "$app_id" >/dev/null 2>&1 || true
  if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null 2>&1 || true; fi
  rm -rf -- "$scratch"
}
trap cleanup EXIT
cargo test --locked --release -p sigil-client --features rtc-client --lib --no-run --message-format=json > "$scratch/build.json"
fixture=$(jq -r 'select(.reason=="compiler-artifact" and .target.name=="sigil_client" and .profile.test and .executable!=null) | .executable' "$scratch/build.json")
test -x "$fixture"
openssl x509 -inform DER -in client/tests/fixtures/synthetic-server.der -out "$scratch/cert.pem"
openssl pkey -inform DER -in client/tests/fixtures/synthetic-server-key.der -out "$scratch/key.pem"
chmod 755 "$scratch"
chmod 644 "$scratch/cert.pem" "$scratch/key.pem"
container=$(docker run --detach --network host --read-only --cap-drop ALL --cap-add NET_BIND_SERVICE \
  --security-opt no-new-privileges:true --tmpfs /tmp:rw,noexec,nosuid,size=8m --memory 128m --pids-limit 64 \
  --mount "type=bind,source=$scratch,target=/test-tls,readonly" --entrypoint turnserver \
  coturn/coturn@sha256:aa68aab64a3b929d57fc2924c98ea447bf996cf8dade2508e7b71eaf23f1f14e \
  -n --listening-ip=127.0.0.1 --relay-ip=127.0.0.1 --listening-port=39781 --tls-listening-port=39813 \
  --min-port=39782 --max-port=39812 --realm=synthetic --use-auth-secret --static-auth-secret=synthetic-rest-secret-for-call-tests \
  --cert=/test-tls/cert.pem --pkey=/test-tls/key.pem --no-dtls --no-cli --allow-loopback-peers \
  --no-multicast-peers --relay-threads=1 --log-file=stdout --pidfile=/tmp/turn.pid)
"$adb" reverse tcp:39813 tcp:39813 >/dev/null
instrument() {
  "$adb" shell am instrument -w -e call_end "${mode:-ui}" -e class "org.sigil.compose.$1" "$app_id.test/androidx.test.runner.AndroidJUnitRunner" > "$scratch/device.log" 2>&1
  cat "$scratch/device.log"
  rg -q '^OK \(' "$scratch/device.log"
}
modes=("$@")
if ((${#modes[@]} == 0)); then modes=(transport ui notification group); fi
for mode in "${modes[@]}"; do
  [[ "$mode" == transport || "$mode" == ui || "$mode" == notification || "$mode" == group ]] || exit 2
  "$adb" shell pm clear "$app_id" >/dev/null
  instrument FixtureKeyTest
  export SIGIL_ANDROID_CALL_EXPORT="$scratch/fixture"
  mkdir -m 700 "$SIGIL_ANDROID_CALL_EXPORT"
  "$adb" exec-out run-as "$app_id" cat cache/acceptance.key > "$SIGIL_ANDROID_CALL_EXPORT/key"
  "$adb" shell run-as "$app_id" rm cache/acceptance.key
  if [[ "$mode" != transport ]]; then touch "$SIGIL_ANDROID_CALL_EXPORT/ui"; fi
  if [[ "$mode" == group ]]; then touch "$SIGIL_ANDROID_CALL_EXPORT/group"; fi
  SIGIL_ANDROID_CALL_RELAY='turns:chat.example:39813?transport=tcp' \
    "$fixture" calls::rtc_transport::tests::android_codec_transport_acceptance --ignored --exact --nocapture > "$scratch/fixture.log" 2>&1 &
  fixture_pid=$!
  for attempt in $(seq 1 90); do
    if [[ -f "$SIGIL_ANDROID_CALL_EXPORT/ready" ]]; then break; fi
    if ! kill -0 "$fixture_pid" 2>/dev/null; then cat "$scratch/fixture.log"; exit 1; fi
    sleep 1
  done
  test -f "$SIGIL_ANDROID_CALL_EXPORT/ready"
  port=$(cat "$SIGIL_ANDROID_CALL_EXPORT/port")
  "$adb" reverse "tcp:$port" "tcp:$port" >/dev/null
  "$adb" push "$SIGIL_ANDROID_CALL_EXPORT/client.db" /data/local/tmp/sigil-call-fixture.db >/dev/null
  if [[ "$mode" == transport ]]; then directory=cache/call-transport-test; test_class=CallTransportTest
  else directory=no_backup/native; test_class=CallUiTest; fi
  "$adb" shell run-as "$app_id" mkdir -p "$directory"
  "$adb" shell run-as "$app_id" cp /data/local/tmp/sigil-call-fixture.db "$directory/client.db"
  "$adb" shell run-as "$app_id" chmod 700 "$directory"
  "$adb" shell run-as "$app_id" chmod 600 "$directory/client.db"
  if [[ "$mode" == group ]]; then
    (
      while kill -0 "$fixture_pid" 2>/dev/null; do
        if [[ -f "$SIGIL_ANDROID_CALL_EXPORT/continued" ]]; then
          "$adb" shell run-as "$app_id" touch cache/call-continued
          break
        fi
        sleep 1
      done
    ) &
    progress_pid=$!
  fi
  instrument "$test_class"
  if [[ -n "$progress_pid" ]]; then wait "$progress_pid"; progress_pid=; fi
  touch "$SIGIL_ANDROID_CALL_EXPORT/done"
  if ! wait "$fixture_pid"; then cat "$scratch/fixture.log"; exit 1; fi
  fixture_pid=
  cat "$scratch/fixture.log"
  "$adb" reverse --remove "tcp:$port" >/dev/null; port=
  rm -rf -- "$SIGIL_ANDROID_CALL_EXPORT"
done
