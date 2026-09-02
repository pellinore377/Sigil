//! Expiry. Everything the server keeps has a day number, never a time of
//! day; once a day the sweep removes what has expired: slots whose expiry
//! day has passed (with their envelopes, subscriptions and acks), blobs
//! past their expiry, deliveries held for Envoys that never came back,
//! and Envoy queues older than the retention window.

use crate::store::{
    key_seq, today, SlotMeta, Store, ACKS, BLOBS, BLOB_EXPIRY, ENVELOPES, PENDING, QUEUES,
    QUEUE_META, SLOTS, SUBS,
};
use redb::ReadableTable;
use std::sync::Arc;
use std::time::Duration;

/// Days an Envoy holds a queue, and the server holds a pending delivery.
const QUEUE_DAYS: u32 = 30;
const PENDING_DAYS: u32 = 1;

pub fn start(store: Arc<Store>) {
    tokio::spawn(async move {
        loop {
            match sweep(&store) {
                Ok(n) if n > 0 => tracing::info!("sweep removed {n} expired records"),
                Ok(_) => {}
                Err(e) => tracing::warn!("sweep failed: {e:#}"),
            }
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

/// One pass. Returns how many records were removed.
pub fn sweep(store: &Store) -> anyhow::Result<usize> {
    let today = today();
    let mut removed = 0;
    // expired slots
    let expired: Vec<[u8; 32]> = {
        let r = store.db.begin_read()?;
        let t = r.open_table(SLOTS)?;
        let mut v = Vec::new();
        for item in t.iter()? {
            let (k, val) = item?;
            if let Some(m) = SlotMeta::decode(val.value()) {
                if m.expiry_day != 0 && m.expiry_day < today {
                    v.push(k.value().try_into().unwrap());
                }
            }
        }
        v
    };
    if !expired.is_empty() {
        let w = store.db.begin_write()?;
        {
            let mut slots = w.open_table(SLOTS)?;
            let mut envs = w.open_table(ENVELOPES)?;
            let mut subs = w.open_table(SUBS)?;
            let mut acks = w.open_table(ACKS)?;
            for a in &expired {
                slots.remove(a.as_slice())?;
                removed += 1;
                let lo = key_seq(a, 0);
                let hi = [a.as_slice(), &[0xffu8; 32]].concat();
                let ek: Vec<Vec<u8>> = envs
                    .range(lo.as_slice()..=hi.as_slice())?
                    .map(|i| i.map(|(k, _)| k.value().to_vec()))
                    .collect::<Result<_, _>>()?;
                for k in ek {
                    envs.remove(k.as_slice())?;
                    removed += 1;
                }
                let sk: Vec<Vec<u8>> = subs
                    .range(lo.as_slice()..=hi.as_slice())?
                    .map(|i| i.map(|(k, _)| k.value().to_vec()))
                    .collect::<Result<_, _>>()?;
                for k in sk {
                    subs.remove(k.as_slice())?;
                    removed += 1;
                }
                let ak: Vec<Vec<u8>> = acks
                    .range(lo.as_slice()..=hi.as_slice())?
                    .map(|i| i.map(|(k, _)| k.value().to_vec()))
                    .collect::<Result<_, _>>()?;
                for k in ak {
                    acks.remove(k.as_slice())?;
                    removed += 1;
                }
            }
        }
        w.commit()?;
    }
    // expired blobs
    let dead: Vec<Vec<u8>> = {
        let r = store.db.begin_read()?;
        let t = r.open_table(BLOB_EXPIRY)?;
        let mut v = Vec::new();
        for item in t.iter()? {
            let (k, day) = item?;
            if day.value() < today {
                v.push(k.value().to_vec());
            }
        }
        v
    };
    if !dead.is_empty() {
        let w = store.db.begin_write()?;
        {
            let mut blobs = w.open_table(BLOBS)?;
            let mut exp = w.open_table(BLOB_EXPIRY)?;
            for k in &dead {
                blobs.remove(k.as_slice())?;
                exp.remove(k.as_slice())?;
                removed += 1;
            }
        }
        w.commit()?;
    }
    // pending deliveries for absent Envoys: value starts with the day
    removed += prune_by_day(store, PENDING, today.saturating_sub(PENDING_DAYS))?;
    // Envoy queues: QUEUE_META has no day; queues are bounded by count and
    // by their handle's lifetime, and a queue whose handle is gone is dropped
    let orphan: Vec<Vec<u8>> = {
        let r = store.db.begin_read()?;
        let meta = r.open_table(QUEUE_META)?;
        let handles = r.open_table(crate::store::HANDLES)?;
        let mut v = Vec::new();
        for item in meta.iter()? {
            let (k, _) = item?;
            let handle = &k.value()[32..64];
            if handles.get(handle)?.is_none() {
                v.push(k.value().to_vec());
            }
        }
        v
    };
    if !orphan.is_empty() {
        let w = store.db.begin_write()?;
        {
            let mut meta = w.open_table(QUEUE_META)?;
            let mut q = w.open_table(QUEUES)?;
            for k in &orphan {
                meta.remove(k.as_slice())?;
                let handle: [u8; 32] = k[32..64].try_into().unwrap();
                let lo = key_seq(&handle, 0);
                let hi = key_seq(&handle, u64::MAX);
                let keys: Vec<Vec<u8>> = q
                    .range(lo.as_slice()..=hi.as_slice())?
                    .map(|i| i.map(|(k, _)| k.value().to_vec()))
                    .collect::<Result<_, _>>()?;
                for qk in keys {
                    q.remove(qk.as_slice())?;
                }
                removed += 1;
            }
        }
        w.commit()?;
    }
    let _ = QUEUE_DAYS;
    Ok(removed)
}

/// Remove rows whose value begins with a little-endian day older than `before`.
fn prune_by_day(
    store: &Store,
    table: redb::TableDefinition<&[u8], &[u8]>,
    before: u32,
) -> anyhow::Result<usize> {
    let dead: Vec<Vec<u8>> = {
        let r = store.db.begin_read()?;
        let t = r.open_table(table)?;
        let mut v = Vec::new();
        for item in t.iter()? {
            let (k, val) = item?;
            let day = u32::from_le_bytes(val.value()[..4].try_into().unwrap_or([0; 4]));
            if day < before {
                v.push(k.value().to_vec());
            }
        }
        v
    };
    if dead.is_empty() {
        return Ok(0);
    }
    let w = store.db.begin_write()?;
    {
        let mut t = w.open_table(table)?;
        for k in &dead {
            t.remove(k.as_slice())?;
        }
    }
    w.commit()?;
    Ok(dead.len())
}
