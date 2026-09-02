//! Sigil client core. Everything a Sigil client does that is not UI:
//! identity and account setup, the Envoy link, MLS conversations, slots,
//! requests, and a local store. The engine wraps this behind its
//! backend trait; `sigil-cli` drives it directly.

pub mod account;
pub mod conversation;
pub mod link;
pub mod linking;
pub mod provider;
pub mod state;

pub use link::Link;
pub use state::State;
