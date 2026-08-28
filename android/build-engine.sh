#!/usr/bin/env bash
# Cross-compiles the engine for Android and regenerates the Kotlin bindings.
# Run before ./gradlew assembleDebug on a fresh checkout.
#
#   ANDROID_HOME   defaults to ~/Android
#   NDK            defaults to the single version under $ANDROID_HOME/ndk
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
SDK="${ANDROID_HOME:-$HOME/Android}"
NDK_DIR="$(ls -d "$SDK"/ndk/* 2>/dev/null | head -1)"
[ -n "$NDK_DIR" ] || { echo "no NDK under $SDK/ndk — install with: sdkmanager 'ndk;27.2.12479018'" >&2; exit 1; }
BIN="$NDK_DIR/toolchains/llvm/prebuilt/linux-x86_64/bin"
TARGET=aarch64-linux-android
API=26

rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"

export CC_aarch64_linux_android="$BIN/$TARGET$API-clang"
export AR_aarch64_linux_android="$BIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$BIN/$TARGET$API-clang"

BUILD="${SIGIL_ANDROID_BUILD_DIR:-$HOME/.cache/sigil-android}"

# `calls` is off: LiveKit/libwebrtc has no Android build here yet, and the
# camera backend is v4l2. Messaging works; calls are a later port.
echo "building engine for $TARGET (no calls)…"
(cd "$REPO/core" && CARGO_TARGET_DIR="$BUILD" cargo build --release --target "$TARGET" --no-default-features)

SO="$BUILD/$TARGET/release/libsigil_engine.so"
install -Dm644 "$SO" "$HERE/app/src/main/jniLibs/arm64-v8a/libsigil_engine.so"
echo "installed $(du -h "$SO" | cut -f1) .so"

echo "generating Kotlin bindings…"
rm -rf "$HERE/app/src/main/java/uniffi"
(cd "$REPO/core" && cargo run -q --bin uniffi-bindgen -- \
  generate --library "$SO" --language kotlin --out-dir "$HERE/app/src/main/java")
echo "done — now: cd android && gradle assembleDebug"
