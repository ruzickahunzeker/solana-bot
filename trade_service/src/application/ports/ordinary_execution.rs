use crate::{
    application::{trade_command::TradeCommand, trade_execution::TradeExecutionResult},
    domain::blockhash::Blockhash,
};
use std::{future::Future, pin::Pin};

pub(crate) type TradeExecutionFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<TradeExecutionResult>> + Send + 'a>>;

#[derive(Clone, Copy)]
pub(crate) struct SdkExecutionContext {
    pub blockhash: Blockhash,
    pub wait_tx_confirmed: bool,
    pub mev_submit_verify_ms: u64,
}

/// Retains the single resolved wallet, including across persistence awaits.
/// No raw signer, RPC, SDK parameter or generic transaction-signing capability.
pub(crate) trait OrdinaryExecutionWallet: Send + Sync {
    fn sdk<'a>(
        &'a self,
        command: &'a TradeCommand,
        context: SdkExecutionContext,
    ) -> TradeExecutionFuture<'a>;

    fn pumpapi<'a>(
        &'a self,
        command: &'a TradeCommand,
        wait_tx_confirmed: bool,
    ) -> TradeExecutionFuture<'a>;
}

pub(crate) trait OrdinaryExecutionBackend: Send + Sync {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinaryExecutionWallet>>;
}
