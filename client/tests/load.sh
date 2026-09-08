#!/usr/bin/env bash
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
image=${1:-sigil-backend:dev}
scratch=$(mktemp -d /tmp/sigil-load.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT
cd -- "$repo"
cargo test --locked --release -p sigil-client --lib --no-run --message-format=json > "$scratch/build.json"
executable=$(jq -r 'select(.reason=="compiler-artifact" and .target.name=="sigil_client" and .profile.test and .executable!=null) | .executable' "$scratch/build.json")
test -x "$executable"
loader=$(readelf -l "$executable" | sed -n 's/.*interpreter: \(.*\)]/\1/p')
loader=$(readlink -f -- "$loader")
libraries=$(dirname -- "$loader")
docker run --rm --no-healthcheck --network none --read-only --cap-drop ALL --security-opt no-new-privileges:true \
  --memory 8g --cpus 4 --pids-limit 256 --tmpfs /tmp:rw,noexec,nosuid,size=3g \
  --mount "type=bind,source=$executable,target=/sigil-tests,readonly" \
  --mount "type=bind,source=$libraries,target=/host-lib,readonly" \
  --entrypoint "/host-lib/$(basename -- "$loader")" "$image" \
  --library-path /host-lib /sigil-tests load_tests:: --ignored --nocapture --test-threads=1
