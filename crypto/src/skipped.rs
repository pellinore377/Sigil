//! FIFO retention of at most 128 skipped keys. Only public identifiers move in
//! the age queue; keys retain the existing zeroizing ownership in the map.
use crate::{MessageKey, Secret32};
use std::collections::{BTreeMap, VecDeque};
pub(crate) const MAX: usize = 128;
pub(crate) struct Skipped<K> {
    keys: BTreeMap<K, MessageKey>,
    order: VecDeque<K>,
}
impl<K: Copy + Ord> Skipped<K> {
    pub fn new() -> Self {
        Self {
            keys: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.keys.len()
    }
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&K, &MessageKey)> {
        // Both containers are private and every mutation preserves this pairing.
        self.order.iter().map(|id| (id, &self.keys[id]))
    }
    pub fn candidate(&self) -> Self {
        let mut result = Self::new();
        for (&id, key) in self.iter() {
            result.insert(id, MessageKey(Secret32::from_bytes(*key.0 .0)));
        }
        result
    }
    pub fn insert(&mut self, id: K, key: MessageKey) -> Option<MessageKey> {
        if let Some(prior) = self.keys.get_mut(&id) {
            return Some(std::mem::replace(prior, key));
        }
        if self.keys.len() == MAX {
            if let Some(oldest) = self.order.pop_front() {
                self.keys.remove(&oldest);
            }
        }
        self.order.push_back(id);
        self.keys.insert(id, key)
    }
    pub fn remove(&mut self, id: &K) -> Option<MessageKey> {
        let key = self.keys.remove(id)?;
        self.order.retain(|candidate| candidate != id);
        Some(key)
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.order.retain(|id| {
            if keep(id) {
                true
            } else {
                self.keys.remove(id);
                false
            }
        });
    }
}
