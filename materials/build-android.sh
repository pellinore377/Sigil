#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME}"
output=${1:?Provide the JNI output directory}
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="--remap-path-prefix=$HOME=/build -C link-arg=-Wl,-z,max-page-size=16384"
cargo build --locked --release --target aarch64-linux-android --manifest-path materials/android-native/Cargo.toml
mkdir -p "$output/arm64-v8a"
cp "$CARGO_TARGET_DIR/aarch64-linux-android/release/libsigil_material_android.so" "$output/arm64-v8a/"
