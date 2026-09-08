#!/usr/bin/env bash
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
image=${1:-sigil-backend:dev}
scratch=$(mktemp -d /tmp/sigil-federation-test.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT
cd -- "$repo"
if ! cargo test --locked --release -p sigil-client --lib --no-run --message-format=json > "$scratch/build.json"; then
  jq -r 'select(.reason=="compiler-message") | .message.rendered // empty' "$scratch/build.json" >&2
  exit 1
fi
executable=$(jq -r 'select(.reason=="compiler-artifact" and .target.name=="sigil_client" and .profile.test and .executable!=null) | .executable' "$scratch/build.json")
test -x "$executable"
loader=$(readelf -l "$executable" | sed -n 's/.*interpreter: \(.*\)]/\1/p')
loader=$(readlink -f -- "$loader")
libraries=$(dirname -- "$loader")
docker run --rm --no-healthcheck --network none --read-only --cap-drop ALL --security-opt no-new-privileges:true \
  --memory 1g --cpus 4 --pids-limit 256 --tmpfs /tmp:rw,noexec,nosuid,size=256m \
  --add-host chat.example:127.0.0.1 --add-host federated.example:127.0.0.1 \
  --env SIGIL_FEDERATION_ACCEPTANCE=1 \
  --mount "type=bind,source=$executable,target=/sigil-tests,readonly" \
  --mount "type=bind,source=$libraries,target=/host-lib,readonly" \
  --entrypoint "/host-lib/$(basename -- "$loader")" "$image" \
  --library-path /host-lib /sigil-tests \
  federation::tests::two_servers_exchange_messages_groups_and_files_across_restart_and_outage --exact --ignored --nocapture
