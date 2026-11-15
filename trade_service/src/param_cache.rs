use crate::domain::trade::Pool;
use crate::state::now_ms;
use dashmap::DashMap;

#[derive(Clone)]
pub struct ParamCacheEntry {
    pub pool: Pool,
    pub mint: String,
    pub pool_id: Option<String>,
    pub updated_at_ms: u128,
}

#[derive(Default)]
pub struct ParamCache {
    entries: DashMap<String, ParamCacheEntry>,
}

impl ParamCache {
    pub fn insert(&self, key: String, pool: Pool, mint: String, pool_id: Option<String>) {
        self.entries.insert(
            key,
            ParamCacheEntry {
                pool,
                mint,
                pool_id,
                updated_at_ms: now_ms(),
            },
        );
    }

    pub fn get(&self, key: &str) -> Option<ParamCacheEntry> {
        self.entries.get(key).map(|entry| entry.clone())
    }
}
