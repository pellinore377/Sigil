//! Bindings for frontends that link the engine directly rather than reaching it
//! over a socket — Android and iOS, where there is no daemon to talk to.
//!
//! The shape matches the socket protocol deliberately: JSON in, JSON out. One
//! contract to keep correct instead of two, and a binding that does not need
//! regenerating every time a field is added.

use serde_json::json;


/// Parse SigilText into the render tree a frontend draws.
///
/// Returns `{ body, html, effects }` — `body` is the plain text, effect offsets
/// are character indices into it, and every span already carries its resolved
/// colours for both grounds. The caller never resolves a colour name.
#[uniffi::export]
pub fn sigiltext_render(source: String) -> String {
    let c = crate::timeline::effects::compose(&source);
    json!({
        "body": c.body,
        "html": c.html,
        "effects": crate::timeline::effects::to_json(&c.effects),
    })
    .to_string()
}

/// The engine's version, so a frontend can report what it is linked against.
#[uniffi::export]
pub fn engine_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// The motion specification: timings, Bézier easings and per-animation
/// parameters. Static, so a frontend fetches it once and drives its own
/// animation system with the numbers rather than choosing any.
#[uniffi::export]
pub fn sigiltext_motion() -> String {
    crate::timeline::motion::all().to_string()
}
