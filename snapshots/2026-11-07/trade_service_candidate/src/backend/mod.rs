//! Stable execution boundary between Trade Service business logic and SDK backends.
//!
//! SDK-specific parameter and account types must stay inside a backend implementation.

pub mod registry;
pub mod sdk;
pub mod types;

use anyhow::Result;
use async_trait::async_trait;
use types::{
    BackendCapabilities, QuoteRequest, QuoteResult, ResolvedRoute, RouteHint, SimulationResult,
    SubmissionResult, TradePlan,
};

#[async_trait]
pub trait ExecutionBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> BackendCapabilities;
    async fn resolve_pool(&self, hint: RouteHint) -> Result<ResolvedRoute>;
    async fn warmup(&self, hint: RouteHint) -> Result<()>;
    async fn quote_exact_in(&self, request: QuoteRequest) -> Result<QuoteResult>;
    async fn simulate(&self, plan: TradePlan) -> Result<SimulationResult>;
    async fn submit(&self, plan: TradePlan, idempotency_key: &str) -> Result<SubmissionResult>;
}
