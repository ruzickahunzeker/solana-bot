use crate::{
    application::ports::pool_queries::ParamCacheWriter, domain::route::PoolRouteHint,
    param_cache::ParamCache, state::cache_key,
};
use std::sync::Arc;

impl ParamCacheWriter for Arc<ParamCache> {
    fn record(&self, hint: &PoolRouteHint) -> String {
        let mint = hint.mint.to_string();
        let pool_id = hint.pool_id.map(|id| id.to_string());
        let key = cache_key(hint.pool, &mint, pool_id.as_deref());
        self.insert(key.clone(), hint.pool, mint, pool_id);
        key
    }
}
