//! Page op-log routes. Bodies are sealed under the page key; tokens travel only in headers.
use serde::{Deserialize, Serialize};
pub use sigil_text::page_card::{
    validate_summary, Grant, PageCard, PageKind, MAX_GRANT_BYTES, MAX_HOST_BYTES,
};

pub const ACCESS_HEADER: &str = "sigil-page-access";
pub const MANAGE_HEADER: &str = "sigil-page-manage";
pub const MAX_OP_BYTES: usize = 64 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 3 * 1024 * 1024;
pub const MAX_BODY_PLAINTEXT: usize = 2 * 1024 * 1024;
pub const MAX_PRESENCE_BYTES: usize = 512;
pub const MAX_LABEL_BYTES: usize = 512;
pub const MAX_OPS_PAGE: usize = 512;
pub const MAX_WAIT_SECONDS: u64 = 25;
pub const MAX_PAGES_PER_ACCOUNT: u64 = 4096;
pub const PRESENCE_SECONDS: u64 = 30;
pub const VERSIONS_KEPT: usize = 50;
pub const COMPACTION_SECONDS: u64 = 30 * 86400;
pub const PAGE_RESERVATION: u64 = 2048;
pub const ROW_RESERVATION: u64 = 256;
/// Error codes beside the HTTP status.
pub const PAGE_REMOVED: &str = "page_removed";
pub const ACCESS_DENIED: &str = "page_access_denied";
pub const WRONG_EPOCH: &str = "wrong_epoch";
pub const BASE_BEYOND_HEAD: &str = "base_beyond_head";
pub const BASE_BEFORE_SNAPSHOT: &str = "base_before_snapshot";
pub const SNAPSHOT_ORPHANS_OP: &str = "snapshot_orphans_op";
pub const HEAD_MOVED: &str = "head_moved";
pub const OPS_COMPACTED: &str = "ops_compacted";
pub const REQUEST_REUSED: &str = "request_reused";

/// Lowercase hex on the wire; exact length for fixed fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bytes<const N: usize>(pub [u8; N]);
pub type PageId = Bytes<32>;
pub type TokenHash = Bytes<32>;
pub type Request = Bytes<16>;
pub type Slot = Bytes<16>;
impl<const N: usize> Bytes<N> {
    pub fn hex(&self) -> String {
        hex(&self.0)
    }
    pub fn parse(text: &str) -> Option<Self> {
        unhex(text, N)?.try_into().ok().map(Self)
    }
}
impl<const N: usize> Serialize for Bytes<N> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}
impl<'de, const N: usize> Deserialize<'de> for Bytes<N> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(d)?).ok_or_else(|| serde::de::Error::custom("invalid hex"))
    }
}

/// Sealed page bytes as lowercase hex.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sealed(pub Vec<u8>);
impl Serialize for Sealed {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex(&self.0))
    }
}
impl<'de> Deserialize<'de> for Sealed {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() > MAX_SNAPSHOT_BYTES * 2 {
            return Err(serde::de::Error::custom("sealed value too large"));
        }
        unhex(&text, text.len() / 2)
            .map(Self)
            .ok_or_else(|| serde::de::Error::custom("invalid hex"))
    }
}
impl Sealed {
    fn check(&self, limit: usize) -> Result<(), &'static str> {
        if self.0.is_empty() || self.0.len() > limit {
            return Err("invalid sealed size");
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str, len: usize) -> Option<Vec<u8>> {
    if text.len() != len * 2
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}
fn sequence(value: u64) -> Result<(), &'static str> {
    if value > i64::MAX as u64 {
        return Err("invalid sequence");
    }
    Ok(())
}
fn epoch(value: u64) -> Result<(), &'static str> {
    if value == 0 || value > i64::MAX as u64 {
        return Err("invalid epoch");
    }
    Ok(())
}

/// `POST /client/v0/pages` answers `Status`; the snapshot is sequence 0 of epoch 1.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub id: PageId,
    pub access_hash: TokenHash,
    pub manage_hash: TokenHash,
    pub expires_at: Option<u64>,
    pub snapshot: Sealed,
}
impl Create {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.0 == [0; 32] || self.access_hash == self.manage_hash {
            return Err("invalid page");
        }
        if self
            .expires_at
            .is_some_and(|v| v == 0 || v > i64::MAX as u64)
        {
            return Err("invalid expiry");
        }
        self.snapshot.check(MAX_SNAPSHOT_BYTES)
    }
}

/// `GET /client/v0/pages/{id}`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub epoch: u64,
    pub head: u64,
    pub snapshot_seq: u64,
    pub bytes: u64,
    pub expires_at: Option<u64>,
}

/// `POST /client/v0/pages/{id}/ops`; a repeated `request` answers its first result.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Append {
    pub epoch: u64,
    pub base: u64,
    pub request: Request,
    pub body: Sealed,
}
impl Append {
    pub fn validate(&self) -> Result<(), &'static str> {
        epoch(self.epoch)?;
        sequence(self.base)?;
        self.body.check(MAX_OP_BYTES)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Appended {
    pub seq: u64,
    /// Ops in (base, seq) the caller had not seen.
    pub missed: Vec<Op>,
}

/// One stored op; `base` and `request` are needed to open it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Op {
    pub seq: u64,
    pub base: u64,
    pub epoch: u64,
    pub created: u64,
    pub request: Request,
    pub body: Sealed,
}

/// `GET /client/v0/pages/{id}/ops?after=&wait=`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpsQuery {
    pub after: u64,
    #[serde(default)]
    pub wait: u64,
}
impl OpsQuery {
    pub fn wait_seconds(&self) -> u64 {
        self.wait.min(MAX_WAIT_SECONDS)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ops {
    pub epoch: u64,
    pub head: u64,
    pub ops: Vec<Op>,
}

/// `PUT /client/v0/pages/{id}/snapshot`: the sealed head at `seq`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PutSnapshot {
    pub epoch: u64,
    pub seq: u64,
    pub body: Sealed,
}
impl PutSnapshot {
    pub fn validate(&self) -> Result<(), &'static str> {
        epoch(self.epoch)?;
        sequence(self.seq)?;
        self.body.check(MAX_SNAPSHOT_BYTES)
    }
}
/// `GET …/snapshot` and `GET …/snapshots/{version}`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u64,
    pub seq: u64,
    pub epoch: u64,
    pub created: u64,
    pub body: Sealed,
    pub label: Option<Sealed>,
}
/// `GET …/snapshots`: versions without bodies, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Versions {
    pub versions: Vec<Version>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub version: u64,
    pub seq: u64,
    pub epoch: u64,
    pub created: u64,
    pub bytes: u64,
    /// Sealed under the label subkey of `epoch`, bound to `version`.
    pub label: Option<Sealed>,
}
/// `POST …/snapshots/{version}/label`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub label: Sealed,
}
impl Label {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.label.check(MAX_LABEL_BYTES)
    }
}

/// `PUT …/presence`: kept `PRESENCE_SECONDS` per slot; only the current epoch.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PutPresence {
    pub epoch: u64,
    pub slot: Slot,
    pub body: Sealed,
}
impl PutPresence {
    pub fn validate(&self) -> Result<(), &'static str> {
        epoch(self.epoch)?;
        self.body.check(MAX_PRESENCE_BYTES)
    }
}
/// `GET …/presence`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub epoch: u64,
    pub entries: Vec<PresenceEntry>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceEntry {
    pub slot: Slot,
    pub body: Sealed,
    pub updated: u64,
}

/// `POST …/rotate` with the manage token: compare-and-swap on `expected_epoch`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rotate {
    pub expected_epoch: u64,
    pub access_hash: TokenHash,
    pub manage_hash: TokenHash,
    /// The head sealed under the next epoch's key.
    pub seq: u64,
    pub snapshot: Sealed,
}
impl Rotate {
    pub fn validate(&self) -> Result<(), &'static str> {
        epoch(self.expected_epoch)?;
        if self.expected_epoch >= i64::MAX as u64 || self.access_hash == self.manage_hash {
            return Err("invalid rotation");
        }
        sequence(self.seq)?;
        self.snapshot.check(MAX_SNAPSHOT_BYTES)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rotated {
    pub epoch: u64,
}

/// Sealed presence plaintext; offsets in Unicode scalars of the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub cursor: u32,
    pub selection_end: u32,
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
