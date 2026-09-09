#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${ANDROID_HOME:?Set ANDROID_HOME}"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME}"
: "${JAVA_HOME:?Set JAVA_HOME to JDK 21}"
: "${SIGIL_ANDROID_KEYSTORE:?Set SIGIL_ANDROID_KEYSTORE}"
: "${SIGIL_ANDROID_PASSWORD_FILE:?Set SIGIL_ANDROID_PASSWORD_FILE}"
build_tools="$ANDROID_HOME/build-tools/35.0.0"
gradle=${GRADLE:-gradle}
export PATH="$JAVA_HOME/bin:$PATH"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
export RUSTFLAGS="--remap-path-prefix=$HOME=/build -C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
cargo build --locked --release --target aarch64-linux-android -p sigil-android -p sigil-core
mkdir -p app/build/rust/arm64-v8a app/build/release
cp target/aarch64-linux-android/release/libsigil_android.so target/aarch64-linux-android/release/libsigil_core.so app/build/rust/arm64-v8a/
"$gradle" :app:assembleRelease --console=plain
unsigned=app/build/outputs/apk/release/app-release-unsigned.apk
version=$("$build_tools/aapt" dump badging "$unsigned" | sed -n "s/^package:.*versionName='\([^']*\)'.*/\1/p")
[[ $version =~ ^[a-zA-Z0-9.-]+$ ]]
apk="app/build/release/sigil-$version-arm64-v8a.apk"
"$build_tools/zipalign" -c -P 16 4 "$unsigned"
"$build_tools/apksigner" sign --ks "$SIGIL_ANDROID_KEYSTORE" --ks-key-alias sigil \
  --ks-pass "file:$SIGIL_ANDROID_PASSWORD_FILE" \
  --v4-signing-enabled false --out "$apk" "$unsigned"
"$build_tools/apksigner" verify --verbose --print-certs "$apk"
(cd app/build/release && sha256sum "$(basename "$apk")" > SHA256SUMS)
printf 'Release APK: %s\n' "$apk"
