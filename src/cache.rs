use std::{collections::VecDeque, sync::Arc};

/// Small byte-bounded LRU. Three neighboring images usually occupy this cache.
pub struct Cache<K, V> {
    entries: VecDeque<(K, Arc<V>, u64)>,
    used: u64,
    limit: u64,
}

impl<K: Eq, V> Cache<K, V> {
    pub fn new(limit: u64) -> Self {
        Self {
            entries: VecDeque::new(),
            used: 0,
            limit,
        }
    }
    pub fn get(&mut self, key: &K) -> Option<Arc<V>> {
        let index = self.entries.iter().position(|(k, _, _)| k == key)?;
        let entry = self.entries.remove(index)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }
    pub fn insert(&mut self, key: K, value: Arc<V>, bytes: u64) {
        if let Some(index) = self.entries.iter().position(|(k, _, _)| k == &key)
            && let Some((_, _, bytes)) = self.entries.remove(index)
        {
            self.used -= bytes;
        }
        if bytes > self.limit {
            return;
        }
        while self.used > self.limit - bytes || self.entries.len() >= 32 {
            self.evict_one();
        }
        self.used += bytes;
        self.entries.push_back((key, value, bytes));
    }
    pub fn evict_one(&mut self) -> bool {
        self.pop_lru().is_some()
    }
    /// Transfer the oldest entry out of the cache, allowing resource reuse.
    pub fn pop_lru(&mut self) -> Option<Arc<V>> {
        let (_, value, bytes) = self.entries.pop_front()?;
        self.used -= bytes;
        Some(value)
    }
    /// Release the oldest value which is not held by a foreground consumer.
    pub fn evict_unshared(&mut self) -> bool {
        let Some(index) = self
            .entries
            .iter()
            .position(|(_, value, _)| Arc::strong_count(value) == 1)
        else {
            return false;
        };
        let (_, _, bytes) = self.entries.remove(index).expect("existing cache entry");
        self.used -= bytes;
        true
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.used = 0;
    }
    pub fn used(&self) -> u64 {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lru_is_byte_bounded_and_updates_recency() {
        let mut c = Cache::new(8);
        c.insert(1, Arc::new(1), 4);
        c.insert(2, Arc::new(2), 4);
        assert_eq!(*c.get(&1).unwrap(), 1);
        c.insert(3, Arc::new(3), 4);
        assert!(c.get(&2).is_none());
        c.insert(3, Arc::new(30), 4);
        assert_eq!(c.used(), 8);
        c.insert(4, Arc::new(4), 9);
        assert!(c.get(&4).is_none());
        c.clear();
        assert_eq!(c.used(), 0);
    }

    #[test]
    fn memory_pressure_preserves_pinned_values_and_transfers_resources() {
        let mut cache = Cache::new(12);
        let current = Arc::new(1);
        let reusable = Arc::new(2);
        cache.insert(1, current.clone(), 4);
        cache.insert(2, reusable.clone(), 4);
        assert!(!cache.evict_unshared());
        let entry = cache.pop_lru().unwrap();
        assert!(Arc::ptr_eq(&entry, &current));
        assert_eq!(cache.used(), 4);
        drop(reusable);
        assert!(cache.evict_unshared());
        assert_eq!(cache.used(), 0);
    }
}
