use crate::{
    adapters::sdk::quote_submission::SdkQuoteSubmission,
    aggregator_quote::{self, QuoteStore, SolBuyQuoteRequest},
    application::{
        legacy_quote_submission, ordinary_sol_buy, ordinary_submission,
        ports::{
            ordinary_journal::TradeJournalMode,
            ordinary_sol_buy::{
                JournaledSolBuyOutcome, OrdinarySolBuyBackend, OrdinarySolBuyPipeline,
                OrdinarySolBuyWallet, SolBuyExecutionFuture, SolBuyExecutionOutcome, SolBuyIntent,
            },
            quote_cache::{QuoteBinding, QuoteCache},
        },
        quote_submission_types::{QuoteSubmissionCommand, QuoteSubmissionResult},
        trade_admission::AdmittedTrade,
    },
    domain::{
        quote::{BoundQuote, QuoteEstimate},
        trade::Address,
    },
    persistence::TradePersistence,
    sdk_client::SdkTradeClient,
    wallets::WalletRegistry,
};
use anyhow::Context;
use std::sync::Arc;

pub(crate) struct SdkOrdinarySolBuyBackend {
    wallets: Arc<WalletRegistry>,
    quotes: Arc<QuoteStore>,
    persistence: Arc<TradePersistence>,
    observation_reader:
        Option<Arc<crate::adapters::rpc::ordinary_observation::SingleShotStatusReader>>,
}

impl SdkOrdinarySolBuyBackend {
    pub(crate) fn new(
        wallets: Arc<WalletRegistry>,
        quotes: Arc<QuoteStore>,
        persistence: Arc<TradePersistence>,
    ) -> Self {
        Self {
            wallets,
            quotes,
            persistence,
            observation_reader: None,
        }
    }
    pub(crate) fn with_observation_reader(
        mut self,
        reader: Arc<crate::adapters::rpc::ordinary_observation::SingleShotStatusReader>,
    ) -> Self {
        self.observation_reader = Some(reader);
        self
    }
}

struct ResolvedSolBuyWallet {
    wallet_id: String,
    client: Arc<SdkTradeClient>,
    quotes: Arc<QuoteStore>,
    persistence: Arc<TradePersistence>,
    observation_reader:
        Option<Arc<crate::adapters::rpc::ordinary_observation::SingleShotStatusReader>>,
}

impl OrdinarySolBuyBackend for SdkOrdinarySolBuyBackend {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        self.wallets.get(wallet_id).map(|client| {
            Box::new(ResolvedSolBuyWallet {
                wallet_id: wallet_id.into(),
                client,
                quotes: self.quotes.clone(),
                persistence: self.persistence.clone(),
                observation_reader: self.observation_reader.clone(),
            }) as Box<dyn OrdinarySolBuyWallet>
        })
    }
}

impl OrdinarySolBuyWallet for ResolvedSolBuyWallet {
    fn buy<'a>(
        &'a self,
        execution: &'a AdmittedTrade,
        owner: Option<&'a crate::application::ports::ordinary_claims::ExecutionOwner>,
    ) -> SolBuyExecutionFuture<'a> {
        Box::pin(async move {
            if !execution.command().execution.mode.simulate() {
                owner
                    .context("ordinary SOL execution owner missing")?
                    .ensure_binding(execution)?;
            }
            execution.reservation().ensure_wallet(
                &self.wallet_id,
                Address::from_bytes(self.client.wallet().to_bytes()),
            )?;
            if !execution.command().execution.mode.simulate()
                && execution.reservation().mode() == TradeJournalMode::Durable
            {
                let owner = owner.context("ordinary durable execution owner missing")?;
                let Some(journal) = self.persistence.ordinary_execution_claims() else {
                    return Ok(SolBuyExecutionOutcome::Journaled(JournaledSolBuyOutcome {
                        record: None,
                        signature: None,
                        error: Some("ordinary durable journal unavailable; do not resubmit".into()),
                    }));
                };
                if let Some(reader) = &self.observation_reader {
                    return Ok(ordinary_submission::execute(
                        &journal,
                        self,
                        &crate::adapters::sdk::observed_quote::ObservedQuote::new(
                            &self.client,
                            reader,
                        ),
                        execution,
                        owner,
                    )
                    .await);
                }
                return Ok(ordinary_submission::execute(
                    &journal,
                    self,
                    &SdkQuoteSubmission(&self.client),
                    execution,
                    owner,
                )
                .await);
            }
            ordinary_sol_buy::execute(self, execution.command())
                .await
                .map(SolBuyExecutionOutcome::Legacy)
        })
    }
}

pub(crate) fn quote_request(input: SolBuyIntent) -> SolBuyQuoteRequest {
    SolBuyQuoteRequest {
        wallet_id: input.wallet_id,
        mint: input.mint.to_string(),
        amount_lamports: input.amount_lamports,
        slippage_bps: input.slippage_bps,
    }
}

impl OrdinarySolBuyPipeline for ResolvedSolBuyWallet {
    type Request = SolBuyQuoteRequest;

    fn map_request(&self, input: SolBuyIntent) -> Self::Request {
        quote_request(input)
    }
    fn validate_request(&self, request: &Self::Request) -> anyhow::Result<()> {
        aggregator_quote::validate_request(request)
    }
    async fn quote(&self, request: &Self::Request) -> anyhow::Result<QuoteEstimate> {
        Ok(aggregator_quote::quote(request).await?.into())
    }
    fn bind(&self, request: &Self::Request, quote: QuoteEstimate) -> anyhow::Result<BoundQuote> {
        self.quotes.bind(
            QuoteBinding {
                wallet_id: &request.wallet_id,
                wallet_pubkey: self.client.wallet().to_string(),
                slippage_bps: request.slippage_bps,
            },
            quote,
        )
    }
    async fn submit(
        &self,
        bound: &BoundQuote,
        request: &QuoteSubmissionCommand,
    ) -> anyhow::Result<QuoteSubmissionResult> {
        legacy_quote_submission::execute(&SdkQuoteSubmission(&self.client), bound, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn construction_and_absent_wallet_lookup_keep_existing_registry_and_quote_store() {
        let state = crate::http_contract_tests::state().await;
        let backend = SdkOrdinarySolBuyBackend::new(
            state.wallets.clone(),
            state.aggregator_quotes.clone(),
            state.persistence.clone(),
        );
        assert!(Arc::ptr_eq(&backend.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&backend.quotes, &state.aggregator_quotes));
        assert!(Arc::ptr_eq(&backend.persistence, &state.persistence));
        assert!(backend.resolve("fixture-wallet").is_none());
        assert!(backend.resolve("missing").is_none());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
