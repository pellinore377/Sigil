//! Storage: one redb file. Keys are built with SPE; values are small
//! fixed layouts. There is deliberately no column anywhere that holds a
//! wall-clock time: expiry is a day number, envelopes carry sequence
//! numbers, and nothing else is temporal.

use redb::{Database, TableDefinition};
use sigil_protocol::encoding::{Reader, Writer};
use std::path::Path;

pub const SLOTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("slots");
pub const ENVELOPES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("envelopes");
pub const SUBS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("subs");
pub const ACKS: TableDefinition<&[u8], u64> = TableDefinition::new("acks");
pub const REQ_OWNER: TableDefinition<&[u8], &[u8]> = TableDefinition::new("req_owner");
pub const SHELVES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("shelves");
pub const BLOBS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("blobs");
pub const BLOB_EXPIRY: TableDefinition<&[u8], u32> = TableDefinition::new("blob_expiry");
pub const NAMES: TableDefinition<&str, &[u8]> = TableDefinition::new("names");
pub const WRAPS: TableDefinition<&str, &[u8]> = TableDefinition::new("wraps");
pub const BACKUPS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("backups");
pub const SPENT: TableDefinition<&[u8], ()> = TableDefinition::new("spent");
pub const CREDS: TableDefinition<&[u8], ()> = TableDefinition::new("creds");
pub const KEYS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("keys");
pub const PENDING: TableDefinition<&[u8], &[u8]> = TableDefinition::new("pending");
pub const INVITES: TableDefinition<&str, ()> = TableDefinition::new("invites");
/// OIDC gate: which login (`sub`) holds which localpart.
pub const OIDC_SUBS: TableDefinition<&str, &str> = TableDefinition::new("oidc_subs");
pub const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
// Envoy role
pub const HANDLES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("handles");
pub const QUEUES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("queues");
pub const QUEUE_META: TableDefinition<&[u8], &[u8]> = TableDefinition::new("queue_meta");
pub const PUSH: TableDefinition<&[u8], &[u8]> = TableDefinition::new("push");

pub struct Store {
    pub db: Database,
}

/// Days since the Unix epoch. The only clock the store ever sees.
pub fn today() -> u32 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 86_400) as u32
}

pub fn key2(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut k = a.to_vec();
    k.extend_from_slice(b);
    k
}
pub fn key_seq(a: &[u8], seq: u64) -> Vec<u8> {
    key2(a, &seq.to_be_bytes())
}

#[derive(Debug, Clone)]
pub struct SlotMeta {
    pub write_pub: [u8; 32],
    pub next_seq: u64,
    pub expiry_day: u32,
    /// 0 conversation, 1 requests
    pub kind: u8,
}

impl SlotMeta {
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .fixed(&self.write_pub)
            .u64(self.next_seq)
            .u32(self.expiry_day)
            .u8(self.kind)
            .finish()
    }
    pub fn decode(b: &[u8]) -> Option<SlotMeta> {
        let mut r = Reader::new(b);
        Some(SlotMeta {
            write_pub: r.fixed().ok()?,
            next_seq: r.u64().ok()?,
            expiry_day: r.u32().ok()?,
            kind: r.u8().ok()?,
        })
    }
}

impl Store {
    pub fn open(dir: &Path) -> anyhow::Result<Store> {
        std::fs::create_dir_all(dir)?;
        let db = Database::create(dir.join("sigil.redb"))?;
        let w = db.begin_write()?;
        for t in [
            SLOTS, ENVELOPES, SUBS, REQ_OWNER, SHELVES, BLOBS, BACKUPS, KEYS, PENDING, HANDLES,
            QUEUES, QUEUE_META, PUSH,
        ] {
            w.open_table(t)?;
        }
        w.open_table(ACKS)?;
        w.open_table(BLOB_EXPIRY)?;
        w.open_table(NAMES)?;
        w.open_table(WRAPS)?;
        w.open_table(SPENT)?;
        w.open_table(CREDS)?;
        w.open_table(INVITES)?;
        w.open_table(OIDC_SUBS)?;
        w.open_table(META)?;
        w.commit()?;
        Ok(Store { db })
    }

    pub fn meta_get(&self, k: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let r = self.db.begin_read()?;
        let t = r.open_table(META)?;
        Ok(t.get(k)?.map(|v| v.value().to_vec()))
    }
    pub fn meta_set(&self, k: &str, v: &[u8]) -> anyhow::Result<()> {
        let w = self.db.begin_write()?;
        w.open_table(META)?.insert(k, v)?;
        w.commit()?;
        Ok(())
    }
    /// Get-or-create a 32-byte secret under `k`.
    pub fn meta_seed(&self, k: &str) -> anyhow::Result<[u8; 32]> {
        if let Some(v) = self.meta_get(k)? {
            return v.try_into().map_err(|_| anyhow::anyhow!("bad seed"));
        }
        let seed: [u8; 32] = rand::random();
        self.meta_set(k, &seed)?;
        Ok(seed)
    }
}
