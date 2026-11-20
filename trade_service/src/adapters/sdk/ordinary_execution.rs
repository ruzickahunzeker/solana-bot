use crate::{
    adapters::{pumpapi::pumpapi_trade, sdk::trade_command::sdk_trade},
    application::{
        ports::ordinary_execution::{
            JournaledSdkFuture, OrdinaryExecutionBackend, OrdinaryExecutionWallet,
            SdkExecutionContext, TradeExecutionFuture,
        },
        trade_command::TradeCommand,
    },
    config::PumpApiConfig,
    domain::trade::Side,
    persistence::TradePersistence,
    sdk_client::SdkTradeClient,
    wallets::WalletRegistry,
};
use std::sync::Arc;

pub(crate) struct SdkOrdinaryExecutionBackend {
    wallets: Arc<WalletRegistry>,
    pumpapi: PumpApiConfig,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    journal: Arc<TradePersistence>,
}

impl SdkOrdinaryExecutionBackend {
    pub(crate) fn new(
        wallets: Arc<WalletRegistry>,
        pumpapi: PumpApiConfig,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
        journal: Arc<TradePersistence>,
    ) -> Self {
        Self {
            wallets,
            pumpapi,
            read_rpc,
            journal,
        }
    }
}

struct ResolvedSdkWallet {
    client: Arc<SdkTradeClient>,
    pumpapi: PumpApiConfig,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    journal: Arc<TradePersistence>,
}

impl OrdinaryExecutionBackend for SdkOrdinaryExecutionBackend {
    fn supports_prepared_sdk(&self) -> bool {
        true
    }
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        self.wallets.get(wallet_id).map(|client| {
            Box::new(ResolvedSdkWallet {
                client,
                pumpapi: self.pumpapi.clone(),
                read_rpc: self.read_rpc.clone(),
                journal: self.journal.clone(),
            }) as Box<dyn OrdinaryExecutionWallet>
        })
    }
}

impl OrdinaryExecutionWallet for ResolvedSdkWallet {
    fn prepare_original_sdk<'a>(
        &'a self,
        execution: &'a crate::application::trade_admission::AdmittedTrade,
        context: SdkExecutionContext,
    ) -> crate::application::ports::ordinary_execution::PreparedSdkFuture<'a> {
        Box::pin(async move {
            let journal = self
                .journal
                .ordinary_execution_claims()
                .ok_or_else(|| anyhow::anyhow!("original prepared SDK journal unavailable"))?;
            self.client
                .prepare_original_pump(execution, context, self.read_rpc.clone(), journal)
                .await
        })
    }
    fn sdk_journaled<'a>(
        &'a self,
        execution: &'a crate::application::trade_admission::AdmittedTrade,
        owner: crate::application::ports::ordinary_claims::ExecutionOwner,
        context: SdkExecutionContext,
    ) -> JournaledSdkFuture<'a> {
        Box::pin(async move {
            let Some(journal) = self.journal.ordinary_execution_claims() else {
                return crate::application::trade_execution::JournaledTradeOutcome {
                    record: None,
                    signature: None,
                    error: Some(
                        "original durable SDK journal unavailable; operation was not retried"
                            .into(),
                    ),
                };
            };
            self.client
                .execute_original_damm(&journal, execution, owner, context, self.read_rpc.clone())
                .await
        })
    }
    fn sdk<'a>(
        &'a self,
        command: &'a TradeCommand,
        context: SdkExecutionContext,
    ) -> TradeExecutionFuture<'a> {
        Box::pin(async move {
            let trade = sdk_trade(
                command,
                context.blockhash,
                context.wait_tx_confirmed,
                context.mev_submit_verify_ms,
            );
            match command.amount.side() {
                Side::Buy => self.client.buy(trade, || self.read_rpc.clone()).await,
                Side::Sell => self.client.sell(trade, || self.read_rpc.clone()).await,
            }
        })
    }

    fn pumpapi<'a>(
        &'a self,
        command: &'a TradeCommand,
        wait_tx_confirmed: bool,
    ) -> TradeExecutionFuture<'a> {
        Box::pin(async move {
            let trade = pumpapi_trade(command, self.pumpapi.clone(), wait_tx_confirmed);
            self.client.pumpapi_trade(trade).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backend_construction_and_absent_wallet_resolution_do_not_initialize_sdk() {
        let state = crate::http_contract_tests::state().await;
        let backend = SdkOrdinaryExecutionBackend::new(
            state.wallets.clone(),
            state.config.pumpapi.clone(),
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap()),
            state.persistence.clone(),
        );
        assert!(backend.resolve("fixture-wallet").is_none());
        assert!(backend.resolve("absent").is_none());
        assert_eq!(state.wallets.wallet_count(), 0);
    }

    #[tokio::test]
    async fn preparation_backend_retains_the_supplied_shared_runtime_without_a_second_instance() {
        let state = crate::http_contract_tests::state().await;
        let runtime =
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
        let weak = Arc::downgrade(&runtime);
        let backend = SdkOrdinaryExecutionBackend::new(
            state.wallets.clone(),
            state.config.pumpapi.clone(),
            runtime.clone(),
            state.persistence.clone(),
        );
        assert!(Arc::ptr_eq(&runtime, &backend.read_rpc));
        drop(runtime);
        assert!(weak.upgrade().is_some());
        assert!(backend.resolve("absent").is_none());
        drop(backend);
        assert!(weak.upgrade().is_none());
    }
}
