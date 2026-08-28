//! Fan-out of pushed events to every connected client (bounded, slow clients dropped).
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<Mutex<Vec<(u64, Sender<String>)>>>,
    next: Arc<std::sync::atomic::AtomicU64>,
}

pub struct Subscription {
    hub: Hub,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub.inner.lock().retain(|(id, _)| *id != self.id);
    }
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn subscribe(&self, tx: Sender<String>) -> Subscription {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.lock().push((id, tx));
        Subscription { hub: self.clone(), id }
    }
    pub fn broadcast(&self, event: serde_json::Value) {
        let line = event.to_string();
        let mut dead = Vec::new();
        for (id, tx) in self.inner.lock().iter() {
            if tx.try_send(line.clone()).is_err() {
                dead.push(*id);
            }
        }
        if !dead.is_empty() {
            self.inner.lock().retain(|(id, _)| !dead.contains(id));
        }
    }
}
