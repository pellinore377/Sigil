//! Subscriptions and delivery to Envoy streams. In memory: which Envoy
//! streams are connected and the recent requests-read nonces per stream.
//! On disk: subscriptions, and deliveries for Envoys that are away.

use crate::store::{key2, key_seq, today, Store, PENDING, SUBS};
use dashmap::DashMap;
use sigil_protocol::wire::Frame;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::mpsc;

pub struct Delivery {
    streams: DashMap<[u8; 32], mpsc::Sender<Frame>>,
    nonces: DashMap<[u8; 32], Vec<([u8; 32], Instant)>>,
    pending_seq: AtomicU64,
}

impl Delivery {
    pub fn new() -> Self {
        Delivery {
            streams: DashMap::new(),
            nonces: DashMap::new(),
            pending_seq: AtomicU64::new(1),
        }
    }

    pub fn subscribe(
        &self,
        store: &Store,
        address: &[u8; 32],
        handle: &[u8; 32],
        envoy: &[u8; 32],
    ) -> anyhow::Result<()> {
        let w = store.db.begin_write()?;
        w.open_table(SUBS)?
            .insert(key2(address, handle).as_slice(), envoy.as_slice())?;
        w.commit()?;
        Ok(())
    }

    pub fn unsubscribe(
        &self,
        store: &Store,
        address: &[u8; 32],
        handle: &[u8; 32],
    ) -> anyhow::Result<()> {
        let w = store.db.begin_write()?;
        w.open_table(SUBS)?
            .remove(key2(address, handle).as_slice())?;
        w.commit()?;
        Ok(())
    }

    pub fn subscribers(
        &self,
        store: &Store,
        address: &[u8; 32],
    ) -> anyhow::Result<Vec<([u8; 32], [u8; 32])>> {
        let r = store.db.begin_read()?;
        let t = r.open_table(SUBS)?;
        let mut out = Vec::new();
        let lo = key2(address, &[0u8; 32]);
        let hi = key2(address, &[0xffu8; 32]);
        for item in t.range(lo.as_slice()..=hi.as_slice())? {
            let (k, v) = item?;
            let handle: [u8; 32] = k.value()[32..64].try_into().unwrap();
            let envoy: [u8; 32] = v.value().try_into().unwrap_or([0; 32]);
            out.push((handle, envoy));
        }
        Ok(out)
    }

    /// Deliver an envelope to every subscriber of `address`.
    pub async fn deliver(
        &self,
        store: &Store,
        address: &[u8; 32],
        slot_seq: u64,
        envelope: &[u8],
    ) -> anyhow::Result<()> {
        for (handle, envoy) in self.subscribers(store, address)? {
            let frame = Frame::Deliver {
                wake_handle: handle,
                queue_seq: 0,
                slot_seq,
                envelope: envelope.to_vec(),
            };
            let sent = match self.streams.get(&envoy) {
                Some(tx) => tx.try_send(frame.clone()).is_ok(),
                None => false,
            };
            if !sent {
                let seq = self.pending_seq.fetch_add(1, Ordering::Relaxed);
                let w = store.db.begin_write()?;
                let mut v = today().to_le_bytes().to_vec();
                v.extend_from_slice(&frame.encode());
                w.open_table(PENDING)?
                    .insert(key_seq(&envoy, seq).as_slice(), v.as_slice())?;
                w.commit()?;
            }
        }
        Ok(())
    }

    /// An Envoy stream connected: register it and drain what was held.
    pub fn attach(
        &self,
        store: &Store,
        envoy: [u8; 32],
        tx: mpsc::Sender<Frame>,
    ) -> anyhow::Result<()> {
        let lo = key_seq(&envoy, 0);
        let hi = key_seq(&envoy, u64::MAX);
        let mut held = Vec::new();
        {
            let r = store.db.begin_read()?;
            let t = r.open_table(PENDING)?;
            for item in t.range(lo.as_slice()..=hi.as_slice())? {
                let (k, v) = item?;
                held.push((k.value().to_vec(), v.value().to_vec()));
            }
        }
        let w = store.db.begin_write()?;
        {
            let mut t = w.open_table(PENDING)?;
            for (k, v) in &held {
                t.remove(k.as_slice())?;
                let day = u32::from_le_bytes(v[..4].try_into().unwrap());
                if today().saturating_sub(day) <= 1 {
                    if let Ok(f) = Frame::decode(&v[4..]) {
                        let _ = tx.try_send(f);
                    }
                }
            }
        }
        w.commit()?;
        self.streams.insert(envoy, tx);
        Ok(())
    }

    pub fn detach(&self, envoy: &[u8; 32]) {
        self.streams.remove(envoy);
        self.nonces.remove(envoy);
    }

    pub fn new_nonce(&self, envoy: &[u8; 32]) -> [u8; 32] {
        let n: [u8; 32] = rand::random();
        let mut e = self.nonces.entry(*envoy).or_default();
        e.retain(|(_, t)| t.elapsed().as_secs() < 60);
        e.push((n, Instant::now()));
        n
    }

    pub fn recent_nonces(&self, envoy: &[u8; 32]) -> Vec<[u8; 32]> {
        self.nonces
            .get(envoy)
            .map(|v| {
                v.iter()
                    .filter(|(_, t)| t.elapsed().as_secs() < 60)
                    .map(|(n, _)| *n)
                    .collect()
            })
            .unwrap_or_default()
    }
}
