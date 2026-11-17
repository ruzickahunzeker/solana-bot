use std::sync::Arc;

use crate::{
    adapters::{
        persistence::submission_journal::SqliteSubmissionJournal,
        sdk::quote_submission::{PreparedQuoteTransaction, SdkQuoteSubmission},
    },
    application::ports::{
        quote_execution::QuoteSubmissionBackend,
        quote_submit::{QuoteSubmissionRuntime, QuoteSubmissionWallet},
    },
    domain::{
        quote::{BoundQuote, QuotePreflight},
        submission::SignedSubmission,
        trade::Address,
    },
    persistence::TradePersistence,
    sdk_client::SdkTradeClient,
    wallets::WalletRegistry,
};

pub(crate) struct SdkQuoteSubmissionRuntime {
    wallets: Arc<WalletRegistry>,
    persistence: Arc<TradePersistence>,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}

impl SdkQuoteSubmissionRuntime {
    pub(crate) fn new(
        wallets: Arc<WalletRegistry>,
        persistence: Arc<TradePersistence>,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> Self {
        Self {
            wallets,
            persistence,
            read_rpc,
        }
    }
}

pub(crate) struct ResolvedQuoteSubmissionWallet {
    client: Arc<SdkTradeClient>,
    read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}

impl QuoteSubmissionRuntime for SdkQuoteSubmissionRuntime {
    type Wallet = ResolvedQuoteSubmissionWallet;
    type Journal = SqliteSubmissionJournal;

    fn resolve(&self, wallet_id: &str) -> Option<Self::Wallet> {
        self.wallets
            .get(wallet_id)
            .map(|client| ResolvedQuoteSubmissionWallet {
                client,
                read_rpc: self.read_rpc.clone(),
            })
    }

    fn journal(&self) -> Option<Self::Journal> {
        self.persistence.submission_journal()
    }
}

impl QuoteSubmissionWallet for ResolvedQuoteSubmissionWallet {
    fn pubkey(&self) -> Address {
        Address::from_bytes(self.client.wallet().to_bytes())
    }
}

impl QuoteSubmissionBackend for ResolvedQuoteSubmissionWallet {
    type Prepared = PreparedQuoteTransaction;

    fn ensure_live_allowed(&self) -> anyhow::Result<()> {
        SdkQuoteSubmission(&self.client, &self.read_rpc).ensure_live_allowed()
    }

    async fn prepare(
        &self,
        bound: &BoundQuote,
    ) -> anyhow::Result<(Self::Prepared, QuotePreflight)> {
        SdkQuoteSubmission(&self.client, &self.read_rpc)
            .prepare(bound)
            .await
    }

    fn sign(&self, prepared: Self::Prepared) -> anyhow::Result<SignedSubmission> {
        SdkQuoteSubmission(&self.client, &self.read_rpc).sign(prepared)
    }

    async fn broadcast(&self, signed: &SignedSubmission) -> anyhow::Result<String> {
        SdkQuoteSubmission(&self.client, &self.read_rpc)
            .broadcast(signed)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quote_submit_runtime_reuses_empty_registry_and_memory_persistence_without_sdk_init() {
        let state = crate::http_contract_tests::state().await;
        let read_rpc =
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
        let runtime = SdkQuoteSubmissionRuntime::new(
            state.wallets.clone(),
            state.persistence.clone(),
            read_rpc.clone(),
        );
        assert!(Arc::ptr_eq(&runtime.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&runtime.persistence, &state.persistence));
        assert!(Arc::ptr_eq(&runtime.read_rpc, &read_rpc));
        assert!(runtime.resolve("missing").is_none());
        assert!(runtime.journal().is_none());
        assert_eq!(state.wallets.wallet_count(), 0);
    }

    #[tokio::test]
    async fn quote_submit_runtime_acquires_existing_durable_journal_without_recovering_on_requests()
    {
        use crate::{
            application::ports::submission_journal::SubmissionJournal,
            config::{TradePersistenceConfig, TradePersistenceMode},
            domain::submission::{Reservation, SubmissionIntent, SubmissionKey},
        };
        let state = crate::http_contract_tests::state().await;
        let persistence = Arc::new(
            TradePersistence::new(&TradePersistenceConfig {
                mode: TradePersistenceMode::Durable,
                database_url: "sqlite::memory:".into(),
                busy_timeout_ms: 1000,
            })
            .await
            .unwrap(),
        );
        let read_rpc =
            Arc::new(crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap());
        let runtime = SdkQuoteSubmissionRuntime::new(
            state.wallets.clone(),
            persistence.clone(),
            read_rpc.clone(),
        );
        let intent = SubmissionIntent {
            key: SubmissionKey::new("fixture-wallet", "fixture-key").unwrap(),
            wallet_pubkey: Address::from_bytes([1; 32]).to_string(),
            quote_id: "fixture-quote".into(),
            quote: serde_json::json!({}),
        };
        let journal = runtime.journal().unwrap();
        assert!(matches!(
            journal
                .reserve(&intent, &serde_json::json!({}))
                .await
                .unwrap(),
            Reservation::Created(_)
        ));
        drop(journal);
        let record = runtime
            .journal()
            .unwrap()
            .get(&intent.key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            crate::domain::submission::SubmissionStatus::Reserved
        );
        assert!(Arc::ptr_eq(&runtime.persistence, &persistence));
        assert!(runtime.resolve("fixture-wallet").is_none());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
