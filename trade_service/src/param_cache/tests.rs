use super::*;
use crate::{
    application::ports::pool_queries::ParamCacheWriter,
    domain::{route::PoolRouteHint, trade::Address},
};

fn hint(index: usize) -> PoolRouteHint {
    let mut bytes = [0x31; 32];
    bytes[..8].copy_from_slice(&(index as u64).to_le_bytes());
    let mut pool = [0x49; 32];
    pool[..8].copy_from_slice(&(index as u64).to_le_bytes());
    PoolRouteHint {
        pool: Pool::RaydiumCpmm,
        mint: Address::from_bytes(bytes),
        quote_mint: Address::from_bytes([0x72; 32]),
        pool_id: Some(Address::from_bytes(pool)),
    }
}

#[test]
fn param_cache_capacity_keeps_original_fields_and_evicts_only_on_new_key_pressure() {
    let cache = ParamCache::default();
    let mut keys = Vec::new();
    for index in 0..MAX_CANDIDATE_ENTRIES {
        let route = hint(index);
        let before = now_ms();
        let key = cache.record(&route);
        let entry = cache.get(&key).unwrap();
        assert_eq!(entry.pool, route.pool);
        assert_eq!(entry.mint, route.mint.to_string());
        assert_eq!(entry.pool_id, route.pool_id.map(|value| value.to_string()));
        assert!(entry.updated_at_ms >= before && entry.updated_at_ms <= now_ms());
        cache.entries.get_mut(&key).unwrap().updated_at_ms = 100;
        keys.push(key);
    }
    assert_eq!(
        keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
        MAX_CANDIDATE_ENTRIES
    );
    assert_eq!(cache.entries.len(), MAX_CANDIDATE_ENTRIES);
    let mut ordered = keys.clone();
    ordered.sort();
    let oldest = ordered[0].clone();
    cache.entries.get_mut(&oldest).unwrap().updated_at_ms = 50;

    // An existing key is refreshed without throwing away an unrelated candidate.
    let existing = cache.record(&hint(MAX_CANDIDATE_ENTRIES - 1));
    assert_eq!(existing, keys[MAX_CANDIDATE_ENTRIES - 1]);
    assert_eq!(cache.entries.len(), MAX_CANDIDATE_ENTRIES);
    assert!(keys.iter().all(|key| cache.get(key).is_some()));
    // Pick an old entry distinct from the refreshed key, without assuming clock resolution.
    let victim = ordered
        .iter()
        .find(|key| **key != existing)
        .unwrap()
        .clone();
    cache.entries.get_mut(&victim).unwrap().updated_at_ms = 0;
    let inserted = cache.record(&hint(MAX_CANDIDATE_ENTRIES));
    assert_eq!(cache.entries.len(), MAX_CANDIDATE_ENTRIES);
    assert!(cache.get(&victim).is_none());
    assert!(cache.get(&inserted).is_some());
    assert!(keys
        .iter()
        .filter(|key| **key != victim)
        .all(|key| cache.get(key).is_some()));

    // Equal wall-clock timestamps have deterministic key ordering; no new TTL is imposed.
    for mut entry in cache.entries.iter_mut() {
        entry.updated_at_ms = 10;
    }
    let tied = cache
        .entries
        .iter()
        .map(|entry| entry.key().clone())
        .min()
        .unwrap();
    let next = cache.record(&hint(MAX_CANDIDATE_ENTRIES + 1));
    assert!(cache.get(&tied).is_none());
    assert!(cache.get(&next).is_some());
    assert_eq!(cache.entries.len(), MAX_CANDIDATE_ENTRIES);
}

#[test]
fn param_cache_capacity_is_shared_by_concurrent_writers_and_releases_original_instance() {
    let cache = std::sync::Arc::new(ParamCache::default());
    let weak = std::sync::Arc::downgrade(&cache);
    std::thread::scope(|scope| {
        for writer in 0..4 {
            let cache = cache.clone();
            scope.spawn(move || {
                for offset in 0..MAX_CANDIDATE_ENTRIES / 2 {
                    let route = hint(writer * MAX_CANDIDATE_ENTRIES + offset);
                    let key = cache.record(&route);
                    {
                        // DashMap::len samples shards separately; hold the original admission
                        // gate so an eviction/insertion cannot straddle the count snapshot.
                        let _snapshot = cache.writes.lock().unwrap();
                        assert!(cache.entries.len() <= MAX_CANDIDATE_ENTRIES);
                    }
                    if let Some(entry) = cache.get(&key) {
                        assert_eq!(entry.pool, route.pool);
                        assert_eq!(entry.mint, route.mint.to_string());
                        assert_eq!(entry.pool_id, route.pool_id.map(|value| value.to_string()));
                    }
                }
            });
        }
    });
    assert_eq!(cache.entries.len(), MAX_CANDIDATE_ENTRIES);
    assert_eq!(std::sync::Arc::strong_count(&cache), 1);
    drop(cache);
    assert!(weak.upgrade().is_none());
}
