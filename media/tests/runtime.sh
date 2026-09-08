#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
for sigil_tool in bwrap ffmpeg heif-enc curl tar sha256sum; do command -v "$sigil_tool" >/dev/null; done
export SIGIL_TEST_OFFICE_LIBRARIES="${SIGIL_TEST_OFFICE_LIBRARIES:-/usr/lib}"
test -x "$SIGIL_TEST_OFFICE_LIBRARIES/libreoffice/program/soffice"
sigil_runtime_tmp=$(mktemp -d)
trap 'rm -rf -- "$sigil_runtime_tmp"' EXIT
if [[ -z "${SIGIL_TEST_PDFIUM:-}" ]]; then
  [[ $(uname -sm) == 'Linux x86_64' ]]
  curl --fail --location --silent --show-error https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/7881/pdfium-linux-x64.tgz -o "$sigil_runtime_tmp/pdfium.tgz"
  printf '%s  %s\n' 1470e21b8b4a3b4ad7f85684e2da11d94f3b69a86d81dee11b9b6709d927ac1d "$sigil_runtime_tmp/pdfium.tgz" | sha256sum --check --status
  tar -xzf "$sigil_runtime_tmp/pdfium.tgz" -C "$sigil_runtime_tmp"
  export SIGIL_TEST_PDFIUM="$sigil_runtime_tmp/lib/libpdfium.so"
fi
cargo test --locked --release -p sigil-media --test runtime -- --ignored
