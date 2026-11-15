use crate::{
    application::{
        ports::ordinary_claims::ExecutionOwner,
        trade_admission::AdmittedTrade,
        trade_command::TradeCommand,
        trade_execution::{JournaledTradeOutcome, TradeExecutionResult},
    },
    domain::blockhash::Blockhash,
};
use std::{future::Future, pin::Pin};

pub(crate) type TradeExecutionFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<TradeExecutionResult>> + Send + 'a>>;

pub(crate) type JournaledSdkFuture<'a> =
    Pin<Box<dyn Future<Output = JournaledTradeOutcome> + Send + 'a>>;

#[derive(Clone, Copy)]
pub(crate) struct SdkExecutionContext {
    pub blockhash: Blockhash,
    pub wait_tx_confirmed: bool,
    pub mev_submit_verify_ms: u64,
}

/// Retains the single resolved wallet, including across persistence awaits.
/// No raw signer, RPC, SDK parameter or generic transaction-signing capability.
pub(crate) trait OrdinaryExecutionWallet: Send + Sync {
    fn sdk_journaled<'a>(
        &'a self,
        execution: &'a AdmittedTrade,
        owner: ExecutionOwner,
        context: SdkExecutionContext,
    ) -> JournaledSdkFuture<'a> {
        Box::pin(async move {
            let _ = (execution, owner, context);
            JournaledTradeOutcome {
                record: None,
                signature: None,
                error: Some(
                    "original SDK journaled backend is unavailable; do not retry the operation"
                        .into(),
                ),
            }
        })
    }

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
