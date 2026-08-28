//! Sigil's engine as a library: the Matrix session, encryption, media and
//! calls, with no assumption about how a frontend reaches it.
//!
//! The desktop build wraps this in a daemon speaking JSON lines over a unix
//! socket (`main.rs`). Android and iOS link it directly instead — there is no
//! socket on a phone — which is why the modules live here rather than in the
//! binary.

// uniffi's scaffolding must sit at the crate root.
uniffi::setup_scaffolding!();

pub mod ffi;
pub mod docs;
pub mod engine;
pub mod geo;
pub mod ipc;
pub mod maps;
pub mod media;
pub mod notify;
pub mod paths;
pub mod presence;
/// Video frames over shared memory. Used by call video and by media playback,
/// so it is not behind the `calls` feature.
pub mod shm;
#[cfg(feature = "calls")]
pub mod rtc;
pub mod session;
pub mod sync;
pub mod timeline;

/// Install the crypto provider. Must run before any TLS work; both the daemon
/// and an in-process frontend need it, and calling it twice is harmless.
pub fn init_crypto() {
    rustls::crypto::aws_lc_rs::default_provider().install_default().ok();
}
