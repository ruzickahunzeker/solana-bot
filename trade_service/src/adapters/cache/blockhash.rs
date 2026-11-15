use crate::{
    application::ports::blockhash::CachedBlockhashRead, blockhash_cache::BlockhashCache,
    domain::blockhash::Blockhash,
};

impl CachedBlockhashRead for BlockhashCache {
    fn fresh_hash(&self, max_age_ms: u64) -> Option<Blockhash> {
        self.get_fresh(max_age_ms).map(|cached| cached.hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_runtime_cache_adapter_keeps_original_missing_stale_and_clock_rollback_rules() {
        let empty = BlockhashCache::default();
        assert_eq!(empty.fresh_hash(u64::MAX), None);
        let stale = BlockhashCache::with_snapshot_for_tests(Blockhash::from_bytes([3; 32]), 0);
        assert_eq!(stale.fresh_hash(0), None);
        let rollback =
            BlockhashCache::with_snapshot_for_tests(Blockhash::from_bytes([5; 32]), u128::MAX);
        assert_eq!(rollback.fresh_hash(0), Some(Blockhash::from_bytes([5; 32])));
        assert_eq!(
            rollback.fresh_hash(0),
            rollback.get_fresh(0).map(|cached| cached.hash)
        );
    }
}
