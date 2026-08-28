fn main() {
    // webrtc-sys's desktop_capturer drags in libwebrtc's xdg-desktop-portal code,
    // which references gio/gobject/glib symbols the prebuilt expects the host
    // binary to supply.
    #[cfg(feature = "calls")]
    {
        println!("cargo:rustc-link-lib=dylib=gio-2.0");
        println!("cargo:rustc-link-lib=dylib=gobject-2.0");
        println!("cargo:rustc-link-lib=dylib=glib-2.0");
    }
}
