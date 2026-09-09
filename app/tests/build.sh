#!/usr/bin/env bash
set -euo pipefail
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME}"
: "${ANDROID_HOME:?Set ANDROID_HOME}"
adb="$ANDROID_HOME/platform-tools/adb"
gradle=${GRADLE:-gradle}
app_id=org.sigil.compose.acceptance
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
RUSTFLAGS="--remap-path-prefix=$HOME=/build -C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384" \
  cargo build --locked --release --target aarch64-linux-android -p sigil-android -p sigil-core --features sigil-android/test-fixture
mkdir -p app/build/rust-acceptance/arm64-v8a
cp target/aarch64-linux-android/release/libsigil_android.so target/aarch64-linux-android/release/libsigil_core.so app/build/rust-acceptance/arm64-v8a/
"$gradle" -PsigilAcceptance :app:assembleAcceptance :app:assembleAcceptanceAndroidTest --console=plain
"$adb" install -r app/build/outputs/apk/acceptance/app-acceptance.apk
"$adb" install -r app/build/outputs/apk/androidTest/acceptance/app-acceptance-androidTest.apk
