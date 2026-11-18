use crate::domain::trade::Pool;
use crate::state::now_ms;
use dashmap::DashMap;
use std::sync::Mutex;

const MAX_CANDIDATE_ENTRIES: usize = 1024;

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
    writes: Mutex<()>,
}

impl ParamCache {
    pub fn insert(&self, key: String, pool: Pool, mint: String, pool_id: Option<String>) {
        // Serialize admission, not readers: distinct-key writers must share one hard bound.
        let _write = self
            .writes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_CANDIDATE_ENTRIES {
            let oldest = self
                .entries
                .iter()
                .min_by(|a, b| {
                    a.updated_at_ms
                        .cmp(&b.updated_at_ms)
                        .then_with(|| a.key().cmp(b.key()))
                })
                .map(|entry| entry.key().clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
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

#[cfg(test)]
mod tests;
