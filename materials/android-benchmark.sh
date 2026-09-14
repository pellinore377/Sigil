#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to the installed NDK}"
adb="${ADB:-adb}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS='-C link-arg=-Wl,-z,max-page-size=16384'
cargo build --locked --release --target aarch64-linux-android --manifest-path materials/Cargo.toml --bin benchmark
args=()
case "${1:-}" in
    "") ;;
    --single-sample) args=(--single-sample) ;;
    *) echo 'Usage: android-benchmark.sh [--single-sample]' >&2; exit 2 ;;
esac
remote=/data/local/tmp/sigil-material-benchmark
trap '"$adb" shell rm -f "$remote" >/dev/null' EXIT
"$adb" push "$CARGO_TARGET_DIR/aarch64-linux-android/release/benchmark" "$remote"
"$adb" shell chmod 700 "$remote"
"$adb" shell "$remote" "${args[@]}"
