//! Sigil's engine as a library: the account on a Sigil server, MLS
//! conversations, media and (later) calls, with no assumption about how a
//! frontend reaches it.
//!
//! The desktop build wraps this in a daemon speaking JSON lines over a unix
//! socket (`main.rs`). Android and iOS link it directly instead — there is no
//! socket on a phone — which is why the modules live here rather than in the
//! binary.

// uniffi's scaffolding must sit at the crate root.
uniffi::setup_scaffolding!();

pub mod docs;
pub mod engine;
pub mod ffi;
/// The two families the app ships, embedded ONCE. Slint registers them
/// from here (slint/src/lib.rs), the map lettering and the message-effect
/// rasteriser read them from here; a second `include_bytes!` anywhere is
/// another 4 MB in the binary.
pub mod fonts {
    /// Google Sans Flex, the variable font (wght, wdth, opsz, slnt, GRAD, ROND).
    pub static SANS: &[u8] = include_bytes!("../../shared/fonts/GoogleSansFlex.ttf");
    /// Google Sans Code, variable (wght, MONO).
    pub static CODE: &[u8] = include_bytes!("../../shared/fonts/GoogleSansCode.ttf");
}
pub mod geo;
pub mod ipc;
pub mod maps;
pub mod media;
pub mod net;
pub mod notify;
pub mod paths;
/// Video frames over shared memory. Used by call video and by media playback,
/// so it is not behind the `calls` feature.
pub mod shm;
pub mod sigil;
pub mod timeline;

/// Install the crypto provider. Must run before any TLS work; both the daemon
/// and an in-process frontend need it, and calling it twice is harmless.
pub fn init_crypto() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();
}
