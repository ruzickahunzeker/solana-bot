use super::*;
use crate::{
    application::{pool_queries::PoolQueryService, ports::pool_queries::PoolReadBackend},
    domain::trade::{Address, Pool},
    param_cache::ParamCache,
};

#[tokio::test]
async fn param_cache_port_execution_retains_original_authority_without_fixture_handle() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let mut f = fixture(ExecutionMethod::Sdk, Side::Buy, false, mode).await;
        let original = Arc::downgrade(&f.state.params);
        f.state.params = Arc::default();
        assert!(original.upgrade().is_some());
        f.execute().await;
        assert_eq!(f.status(), TradeStatus::Submitted);
        assert_eq!(f.calls(), 1);
        let key = cache_key(
            f.command.route.pool,
            &f.command.route.mint.to_string(),
            None,
        );
        let entry = original.upgrade().unwrap().get(&key).unwrap();
        assert_eq!(entry.pool, f.command.route.pool);
        assert_eq!(entry.mint, f.command.route.mint.to_string());
        assert_eq!(entry.pool_id, None);
        assert!(f.state.params.get(&key).is_none());
        drop(f);
        assert!(original.upgrade().is_none());
    }
}

struct ReadBackend(PoolRouteHint);
impl PoolReadBackend for ReadBackend {
    async fn resolve(&self, mint: Address) -> anyhow::Result<PoolRouteHint> {
        assert_eq!(mint, self.0.mint);
        Ok(self.0.clone())
    }
    async fn warmup(&self, hint: &PoolRouteHint) -> anyhow::Result<()> {
        assert_eq!(hint.mint, self.0.mint);
        Ok(())
    }
    async fn sol_hop(&self, _: Address) -> anyhow::Result<Address> {
        panic!("cache test must not discover a route")
    }
}

#[tokio::test]
async fn param_cache_capacity_does_not_change_pool_results_or_original_trade_records() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let mut f = fixture(ExecutionMethod::Sdk, Side::Buy, false, mode).await;
        let hint = PoolRouteHint {
            pool: f.command.route.pool,
            mint: f.command.route.mint,
            quote_mint: f.command.route.quote_mint,
            pool_id: f.command.route.pool_id,
        };
        let original = Arc::downgrade(&f.state.params);
        f.state.pool_queries = Arc::new(PoolQueryService::new(
            ReadBackend(hint.clone()),
            f.state.params.clone(),
        ));
        let key = f.state.pool_queries.warmup(hint.clone()).await.unwrap();
        assert!(key.as_str() < "zz-capacity-fixture-0000");
        for index in 0..2048 {
            f.state.params.insert(
                format!("zz-capacity-fixture-{index:04}"),
                Pool::Pump,
                format!("fixture-mint-{index}"),
                None,
            );
        }
        assert!(f.state.params.get(&key).is_none());
        let resolved = f.state.pool_queries.resolve(hint.mint).await.unwrap();
        assert_eq!(resolved.pool, hint.pool);
        assert_eq!(resolved.mint, hint.mint);
        assert_eq!(resolved.quote_mint, hint.quote_mint);
        assert_eq!(resolved.pool_id, hint.pool_id);
        assert_eq!(f.state.pool_queries.warmup(hint).await.unwrap(), key);
        f.execute().await;
        assert_eq!(f.status(), TradeStatus::Submitted);
        assert_eq!(f.calls(), 1);
        assert!(f.state.params.get(&key).is_some());
        drop(f);
        assert!(original.upgrade().is_none());
    }
}

#[tokio::test]
async fn param_cache_port_pool_service_and_execution_share_keys_and_release_original_cache() {
    let mut f = fixture(
        ExecutionMethod::Sdk,
        Side::Buy,
        false,
        TradePersistenceMode::Memory,
    )
    .await;
    let hint = PoolRouteHint {
        pool: f.command.route.pool,
        mint: f.command.route.mint,
        quote_mint: f.command.route.quote_mint,
        pool_id: f.command.route.pool_id,
    };
    let weak = Arc::downgrade(&f.state.params);
    f.state.pool_queries = Arc::new(PoolQueryService::new(
        ReadBackend(hint.clone()),
        f.state.params.clone(),
    ));
    f.state.params = Arc::new(ParamCache::default());
    let key = f.state.pool_queries.warmup(hint.clone()).await.unwrap();
    assert_eq!(key, cache_key(hint.pool, &hint.mint.to_string(), None));
    assert!(weak.upgrade().unwrap().get(&key).is_some());
    f.execute().await;
    assert_eq!(f.calls(), 1);
    let cached = weak.upgrade().unwrap().get(&key).unwrap();
    assert_eq!(cached.pool, hint.pool);
    assert_eq!(cached.mint, hint.mint.to_string());
    assert_eq!(cached.pool_id, None);
    assert!(f.state.params.get(&key).is_none());
    let queries = f.state.pool_queries.clone();
    drop(f);
    assert!(weak.upgrade().is_some());
    drop(queries);
    assert!(weak.upgrade().is_none());
}
