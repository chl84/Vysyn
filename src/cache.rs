use std::{collections::VecDeque, hash::Hash, sync::Arc};

/// Small byte-bounded LRU. Three neighboring images usually occupy this cache.
pub struct Cache<K, V> {
    entries: VecDeque<(K, Arc<V>, u64)>,
    used: u64,
    limit: u64,
}

impl<K: Eq + Hash, V> Cache<K, V> {
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
        if let Some((_, _, bytes)) = self.entries.pop_front() {
            self.used -= bytes;
            true
        } else {
            false
        }
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
}
