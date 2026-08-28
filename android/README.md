# Sigil for Android

A Kotlin/Compose frontend over the same Rust engine the other platforms use.
The engine is linked in-process here — there is no daemon and no socket on a
phone — so `core` is built as a `cdylib` and reached through uniffi bindings.

**Status: a proof of the architecture, not an app.** It renders SigilText from
the engine and nothing else. No login, no sync, no rooms.

## What works

`sigiltext_render` crosses the FFI boundary and Compose draws the result:
colours, gradients, rainbow, bold/italic/strike/underline, size steps, `mark`
highlights, and the shake, wave, pulse, glow, barrel and flip animations.

Nothing here chooses a number. Colours arrive resolved for both grounds,
gradient stops arrive resolved, and `sigiltext_motion()` supplies every
duration, cubic-Bézier easing and displacement. Verified on a OnePlus 6,
arm64-v8a, Android 11.

## Building

Needs the Android SDK with NDK, and a JDK **17 or 21** — Android Gradle Plugin
8.7 rejects newer ones.

```
./build-engine.sh          # cross-compiles the engine, regenerates bindings
gradle assembleDebug       # or ./gradlew if a wrapper is present
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`build-engine.sh` writes two generated things that are **not** committed:
`app/src/main/jniLibs/arm64-v8a/libsigil_engine.so` and the bindings under
`app/src/main/java/uniffi/`. Regenerate them rather than editing them.

## Known gaps

- **arm64 only.** `armeabi-v7a` and `x86_64` (for emulators) need adding to
  `build-engine.sh` and to `abiFilters`.
- **Calls are compiled out** (`--no-default-features`). LiveKit/libwebrtc has no
  Android build wired up, and the camera backend is v4l2. The engine's `rtc`
  module is feature-gated behind `calls` so the rest still builds.
- **`sparkle`, `glitch`, `blur` and `typewriter` are not drawn yet.** Their
  specs ship from the engine; `SigilText.kt` implements shake, wave, pulse,
  glow, barrel and flip. The remaining four need particle, shader and reveal
  work respectively.
- **The Omarchy frontend still hardcodes its animation values.** The engine now
  owns them (`sigiltext.motion`), and QML should consume that instead of its own
  copy — until it does, the two can drift.
- **The FFI surface is one function.** Login, sync and the timeline still need
  exposing. The protocol is JSON in/JSON out by design, so that is a wider
  `handle_request` rather than a binding per call.
