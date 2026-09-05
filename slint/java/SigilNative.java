// The two things a notification can ask the app to do, as native methods.
//
// This class exists only to be a landing pad for JNI. Rust registers both
// methods with RegisterNatives (see `register_natives` in src/platform.rs)
// rather than exporting Java_com_sigil_slint_SigilNative_reply symbols,
// because the library is loaded by NativeActivity under its own name and the
// engine's request path is easier to reach from ordinary Rust.
//
// It lives in the APK's classes.dex, NOT in the dex that platform.rs loads
// into memory: SigilReceiver is a manifest component, so the system loads it
// with the app's own class loader, and a class registered on any other loader
// would be a different class with different natives.
//
// `ready` is the only guard. Android can start this process for a broadcast
// alone — no NativeActivity, no library, no engine — and calling an
// unregistered native there throws UnsatisfiedLinkError. Rust sets the flag
// once the methods are registered, and SigilReceiver checks it first.

package com.sigil.slint;

public final class SigilNative {
    private SigilNative() {}

    /// Set from Rust the moment RegisterNatives succeeds.
    public static volatile boolean ready = false;

    /// Send `text` to `roomId`, as if it had been typed in the composer.
    public static native void reply(String roomId, String text);

    /// Mark everything in `roomId` read.
    public static native void markRead(String roomId);
}
