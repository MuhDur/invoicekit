// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-090 TTL-aware cache used by the SMP/SML lookup pipeline.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

/// A small `HashMap`-backed cache with per-entry TTL expiry.
/// Single-threaded by design; the pipeline wraps it in a
/// `Mutex` so tests stay deterministic.
pub struct TtlCache<K: Eq + Hash, V> {
    map: HashMap<K, Entry<V>>,
}

struct Entry<V> {
    value: V,
    expires_at: Instant,
}

impl<K: Eq + Hash, V: Clone> Default for TtlCache<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash, V: Clone> TtlCache<K, V> {
    /// Build an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Number of entries in the cache (including any that have
    /// expired but have not been pruned yet).
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when [`Self::len`] is zero.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Insert a value with an absolute expiry instant.
    pub fn insert(&mut self, key: K, value: V, expires_at: Instant) {
        self.map.insert(key, Entry { value, expires_at });
    }

    /// Read a value, returning `None` when the entry is absent
    /// or has expired.
    pub fn get(&mut self, key: &K, now: Instant) -> Option<V> {
        let entry = self.map.get(key)?;
        if now >= entry.expires_at {
            self.map.remove(key);
            return None;
        }
        Some(entry.value.clone())
    }

    /// Drop every entry whose expiry has passed.
    pub fn prune(&mut self, now: Instant) {
        self.map.retain(|_, entry| now < entry.expires_at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn insert_then_get_returns_value_before_expiry() {
        let mut cache: TtlCache<&'static str, u32> = TtlCache::new();
        let now = Instant::now();
        cache.insert("k", 42, now + Duration::from_secs(60));
        assert_eq!(cache.get(&"k", now), Some(42));
    }

    #[test]
    fn get_returns_none_after_expiry() {
        let mut cache: TtlCache<&'static str, u32> = TtlCache::new();
        let now = Instant::now();
        cache.insert("k", 42, now);
        assert_eq!(cache.get(&"k", now + Duration::from_secs(1)), None);
        assert!(cache.is_empty());
    }

    #[test]
    fn prune_drops_only_expired_entries() {
        let mut cache: TtlCache<&'static str, u32> = TtlCache::new();
        let now = Instant::now();
        cache.insert("alive", 1, now + Duration::from_secs(60));
        cache.insert("dead", 2, now);
        cache.prune(now + Duration::from_secs(1));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&"alive", now + Duration::from_secs(2)), Some(1));
    }
}
