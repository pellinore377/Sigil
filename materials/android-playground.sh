#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME}"
: "${ANDROID_HOME:?Set ANDROID_HOME}"
bash materials/build-android.sh materials/android-ui/build/rust
"${GRADLE:-gradle}" -p materials/android-ui assembleRelease --console=plain
"${ADB:-adb}" install -r materials/android-ui/build/outputs/apk/release/SigilMaterials-release.apk
"${ADB:-adb}" shell am force-stop org.sigil.materials
"${ADB:-adb}" shell am start -n org.sigil.materials/.MainActivity
