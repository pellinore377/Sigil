use std::collections::VecDeque;

// Reusable authenticated image data: callers receive their own Blob handle and URL.
// Releasing a row's URL therefore never invalidates another row or viewer.
pub(crate) struct MediaCache<T> {
    entries: VecDeque<(String, usize, T)>,
    bytes: usize,
    budget: usize,
    pub(crate) generation: u64,
    active: bool,
}
impl<T: Clone> MediaCache<T> {
    pub(crate) const fn new(budget: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            budget,
            generation: 0,
            active: true,
        }
    }
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.generation = self.generation.wrapping_add(1);
    }
    pub(crate) fn set_active(&mut self, active: bool) {
        self.active = active;
        if !active {
            self.clear();
        }
    }
    pub(crate) fn get(&mut self, key: &str) -> Option<T> {
        if !self.active {
            return None;
        }
        let at = self.entries.iter().position(|entry| entry.0 == key)?;
        let entry = self.entries.remove(at)?;
        let value = entry.2.clone();
        self.entries.push_back(entry);
        Some(value)
    }
    pub(crate) fn insert(&mut self, key: String, bytes: usize, value: T, generation: u64) {
        if !self.active || generation != self.generation || bytes > self.budget {
            return;
        }
        if let Some(at) = self.entries.iter().position(|entry| entry.0 == key) {
            self.bytes -= self.entries.remove(at).unwrap().1;
        }
        self.entries.push_back((key, bytes, value));
        self.bytes += bytes;
        while self.bytes > self.budget || self.entries.len() > 64 {
            self.bytes -= self.entries.pop_front().unwrap().1;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revisiting_promotes_the_blob_without_unbounded_retention() {
        let mut cache = MediaCache::new(8);
        cache.insert("a".into(), 4, 1, 0);
        cache.insert("b".into(), 4, 2, 0);
        assert_eq!(cache.get("a"), Some(1));
        cache.insert("c".into(), 4, 3, 0);
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("a"), Some(1));
        assert_eq!(cache.bytes, 8);
        for i in 0..100 {
            cache.insert(i.to_string(), 0, i, 0);
        }
        assert_eq!(cache.entries.len(), 64);
    }
    #[test]
    fn account_or_background_clear_rejects_inflight_plaintext() {
        let mut cache = MediaCache::new(8);
        let epoch = cache.generation;
        cache.insert("a".into(), 4, 1, epoch);
        cache.set_active(false);
        cache.insert("b".into(), 4, 2, epoch);
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.bytes, 0);
        cache.set_active(true);
        cache.insert("b".into(), 4, 2, epoch);
        assert_eq!(cache.get("b"), None);
        cache.insert("b".into(), 4, 2, cache.generation);
        assert_eq!(cache.get("b"), Some(2));
        cache.clear();
        assert_eq!(cache.get("b"), None);
    }
}
