#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")/../.."
source app/tests/build.sh
scratch=$(mktemp -d /tmp/sigil-content-device.XXXXXX)
fixture_pid= port=
cleanup() {
  local result=$?
  if ((result != 0)) && [[ -f "$scratch/fixture.log" ]]; then cat "$scratch/fixture.log"; fi
  if [[ -n "$fixture_pid" ]]; then kill "$fixture_pid" >/dev/null 2>&1 || true; fi
  if [[ -n "$port" ]]; then "$adb" reverse --remove "tcp:$port" >/dev/null 2>&1 || true; fi
  "$adb" shell rm -f /data/local/tmp/sigil-content-fixture.db >/dev/null 2>&1 || true
  "$adb" shell pm clear "$app_id" >/dev/null 2>&1 || true
  rm -rf -- "$scratch"
}
trap cleanup EXIT
instrument() {
  "$adb" shell am instrument -w -e class "org.sigil.compose.$1" "$app_id.test/androidx.test.runner.AndroidJUnitRunner" > "$scratch/device.log" 2>&1
  cat "$scratch/device.log"
  rg -q '^OK \(' "$scratch/device.log"
}
"$adb" shell pm clear "$app_id" >/dev/null
instrument FixtureKeyTest
export SIGIL_ANDROID_CONTENT_EXPORT="$scratch"
"$adb" exec-out run-as "$app_id" cat cache/acceptance.key > "$scratch/key"
"$adb" shell run-as "$app_id" rm cache/acceptance.key
cargo test --locked --release -p sigil-client --features rtc-client --lib --no-run --message-format=json > "$scratch/build.json"
fixture=$(jq -r 'select(.reason=="compiler-artifact" and .target.name=="sigil_client" and .profile.test and .executable!=null) | .executable' "$scratch/build.json")
"$fixture" mobile::presentation_tests::content_acceptance::android_content_acceptance --ignored --exact --nocapture > "$scratch/fixture.log" 2>&1 &
fixture_pid=$!
for attempt in $(seq 1 60); do
  if [[ -f "$scratch/ready" ]]; then break; fi
  if ! kill -0 "$fixture_pid" 2>/dev/null; then cat "$scratch/fixture.log"; exit 1; fi
  sleep 1
done
test -f "$scratch/ready"
port=$(cat "$scratch/port")
"$adb" reverse "tcp:$port" "tcp:$port" >/dev/null
"$adb" push "$scratch/client.db" /data/local/tmp/sigil-content-fixture.db >/dev/null
"$adb" shell run-as "$app_id" mkdir -p no_backup/native
"$adb" shell run-as "$app_id" cp /data/local/tmp/sigil-content-fixture.db no_backup/native/client.db
"$adb" shell run-as "$app_id" chmod 700 no_backup/native
"$adb" shell run-as "$app_id" chmod 600 no_backup/native/client.db
instrument ContentTest
instrument MessagingUiTest
instrument DeviceLinkTest
touch "$scratch/done"
wait "$fixture_pid"
fixture_pid=
cat "$scratch/fixture.log"
