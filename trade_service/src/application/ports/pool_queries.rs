use crate::domain::{route::PoolRouteHint, trade::Address};
use anyhow::Result;
use std::future::Future;

pub(crate) trait PoolReadBackend: Send + Sync {
    fn resolve(&self, mint: Address) -> impl Future<Output = Result<PoolRouteHint>> + Send;
    fn warmup(&self, hint: &PoolRouteHint) -> impl Future<Output = Result<()>> + Send;
    fn sol_hop(&self, quote_mint: Address) -> impl Future<Output = Result<Address>> + Send;
}

pub(crate) trait ParamCacheWriter: Send + Sync {
    fn record(&self, hint: &PoolRouteHint) -> String;
}
