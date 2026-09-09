#![forbid(unsafe_code)]
mod roster;
pub use roster::{Delegation, Member, Roster, SignedRoster};
mod connect;
pub use connect::{
    validate_turn_url, Answer, Connect, Downstream, Layout, Relay, RelayRequest, SignedConnect,
    Track,
};
#[cfg(feature = "forwarder")]
pub mod forwarder;
#[cfg(feature = "forwarder")]
mod forwarding;
mod frame;
pub use frame::{Context, Frame, KeyShare, MediaKind, Receiver, Sender};
mod packet;
pub use packet::{packetize, Assembly};
mod state;
pub use state::{Attestation, Ready, Share, State, Tracks};
pub type Id = [u8; 32];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Authentication,
    Conflict,
    Expired,
    Limit,
    Replay,
    Entropy,
}
pub fn random_id() -> Result<Id, Error> {
    let mut id = [0; 32];
    getrandom::fill(&mut id).map_err(|_| Error::Entropy)?;
    Ok(id)
}
fn hash(domain: &[u8], bytes: &[u8]) -> Id {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(bytes);
    hash.finalize().into()
}
