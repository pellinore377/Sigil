#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"
shopt -s globstar nullglob
for asset in shared/build/dist/wasmJs/productionExecutable/**/*.{wasm,js,ttf,html}; do
    brotli -q 9 --force "$asset"
done
