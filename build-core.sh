#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"
export RUSTFLAGS="--remap-path-prefix=$HOME=/build"
cargo test --manifest-path core/Cargo.toml
cargo build --release --manifest-path core/Cargo.toml
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
cargo build --release --target aarch64-linux-android -p sigil-core -p sigil-android
mkdir -p app/build/rust/arm64-v8a
cp target/aarch64-linux-android/release/libsigil_core.so app/build/rust/arm64-v8a/
cp target/aarch64-linux-android/release/libsigil_android.so app/build/rust/arm64-v8a/
cargo build --release --target wasm32-unknown-unknown --manifest-path core/Cargo.toml
wasm-bindgen target/wasm32-unknown-unknown/release/sigil_core.wasm --target web --out-dir target/web
