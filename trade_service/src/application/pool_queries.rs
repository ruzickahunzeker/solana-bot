use crate::{
    application::ports::pool_queries::{ParamCacheWriter, PoolReadBackend},
    domain::{route::PoolRouteHint, trade::Address},
};
use anyhow::Result;
use std::{future::Future, pin::Pin};

pub type PoolQueryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait PoolQueries: Send + Sync {
    fn resolve(&self, mint: Address) -> PoolQueryFuture<'_, PoolRouteHint>;
    fn warmup(&self, hint: PoolRouteHint) -> PoolQueryFuture<'_, String>;
    fn sol_hop(&self, quote_mint: Address) -> PoolQueryFuture<'_, Address>;
}

pub(crate) struct PoolQueryService<B, C> {
    backend: B,
    cache: C,
}

impl<B: PoolReadBackend, C: ParamCacheWriter> PoolQueryService<B, C> {
    pub fn new(backend: B, cache: C) -> Self {
        Self { backend, cache }
    }
}

impl<B: PoolReadBackend, C: ParamCacheWriter> PoolQueries for PoolQueryService<B, C> {
    fn resolve(&self, mint: Address) -> PoolQueryFuture<'_, PoolRouteHint> {
        Box::pin(async move {
            let hint = self.backend.resolve(mint).await?;
            self.cache.record(&hint);
            Ok(hint)
        })
    }

    fn warmup(&self, hint: PoolRouteHint) -> PoolQueryFuture<'_, String> {
        Box::pin(async move {
            self.backend.warmup(&hint).await?;
            Ok(self.cache.record(&hint))
        })
    }

    fn sol_hop(&self, quote_mint: Address) -> PoolQueryFuture<'_, Address> {
        Box::pin(self.backend.sol_hop(quote_mint))
    }
}
