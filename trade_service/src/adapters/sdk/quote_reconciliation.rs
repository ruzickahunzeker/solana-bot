use std::sync::Arc;

use crate::{
    adapters::{
        persistence::quote_reconciliation::QuoteReconciliationJournal,
        rpc::signature_status::RpcSignatureStatusReader,
    },
    application::ports::{
        quote_reconciliation::{QuoteReconciliationRuntime, QuoteReconciliationWallet},
        reconciliation::SignatureStatusReader,
    },
    domain::{reconciliation::ChainObservation, trade::Address},
    persistence::TradePersistence,
    sdk_client::SdkTradeClient,
    wallets::WalletRegistry,
};

pub(crate) struct SdkQuoteReconciliationRuntime {
    wallets: Arc<WalletRegistry>,
    persistence: Arc<TradePersistence>,
}

impl SdkQuoteReconciliationRuntime {
    pub(crate) fn new(wallets: Arc<WalletRegistry>, persistence: Arc<TradePersistence>) -> Self {
        Self {
            wallets,
            persistence,
        }
    }
}

pub(crate) struct ResolvedReconciliationWallet {
    client: Arc<SdkTradeClient>,
}

pub(crate) struct QuoteStatusReader {
    inner: RpcSignatureStatusReader,
}

impl QuoteReconciliationRuntime for SdkQuoteReconciliationRuntime {
    type Wallet = ResolvedReconciliationWallet;
    type Journal = QuoteReconciliationJournal;

    fn resolve(&self, wallet_id: &str) -> Option<Self::Wallet> {
        self.wallets
            .get(wallet_id)
            .map(|client| ResolvedReconciliationWallet { client })
    }

    fn journal(&self) -> Option<Self::Journal> {
        self.persistence
            .submission_journal()
            .map(QuoteReconciliationJournal::new)
    }
}

impl QuoteReconciliationWallet for ResolvedReconciliationWallet {
    type Reader = QuoteStatusReader;

    fn reader(&self) -> Self::Reader {
        QuoteStatusReader {
            inner: RpcSignatureStatusReader {
                rpcs: self.client.rpcs(),
            },
        }
    }

    fn pubkey(&self) -> Address {
        Address::from_bytes(self.client.wallet().to_bytes())
    }
}

impl SignatureStatusReader for QuoteStatusReader {
    async fn lookup_signature(&self, signature: &str) -> anyhow::Result<Option<ChainObservation>> {
        self.inner.lookup_signature(signature).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quote_reconciliation_runtime_reuses_empty_registry_without_sdk_or_journal_init() {
        let state = crate::http_contract_tests::state().await;
        let runtime =
            SdkQuoteReconciliationRuntime::new(state.wallets.clone(), state.persistence.clone());
        assert!(Arc::ptr_eq(&runtime.wallets, &state.wallets));
        assert!(Arc::ptr_eq(&runtime.persistence, &state.persistence));
        assert!(runtime.resolve("missing").is_none());
        assert!(runtime.journal().is_none());
        assert_eq!(state.wallets.wallet_count(), 0);
    }

    #[tokio::test]
    async fn quote_reconciliation_runtime_journal_view_observes_existing_records_without_recovery()
    {
        use crate::{
            application::ports::{
                reconciliation::ReconciliationJournal, submission_journal::SubmissionJournal,
            },
            config::{TradePersistenceConfig, TradePersistenceMode},
            domain::{
                reconciliation::ChainCommitment,
                submission::{
                    Reservation, SignedSubmission, SubmissionIntent, SubmissionKey,
                    SubmissionStatus,
                },
            },
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
        let intent = SubmissionIntent {
            key: SubmissionKey::new("fixture-wallet", "fixture-key").unwrap(),
            wallet_pubkey: Address::from_bytes([1; 32]).to_string(),
            quote_id: "fixture-quote".into(),
            quote: serde_json::json!({}),
        };
        let journal = persistence.submission_journal().unwrap();
        let Reservation::Created(record) = journal
            .reserve(&intent, &serde_json::json!({}))
            .await
            .unwrap()
        else {
            panic!("expected new reservation")
        };
        journal
            .record_signed(
                &record.submission_id,
                &SignedSubmission {
                    signature: "synthetic-signature".into(),
                    bytes: vec![1, 2, 3],
                },
            )
            .await
            .unwrap();
        let runtime =
            SdkQuoteReconciliationRuntime::new(state.wallets.clone(), persistence.clone());
        for _ in 0..2 {
            let record = runtime
                .journal()
                .unwrap()
                .lookup(&intent.key)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(record.status, SubmissionStatus::Signed);
            assert_eq!(record.signature.as_deref(), Some("synthetic-signature"));
        }
        assert!(runtime
            .journal()
            .unwrap()
            .record_observation(
                &record.submission_id,
                &ChainObservation {
                    signature: "synthetic-signature".into(),
                    commitment: ChainCommitment::Finalized,
                    slot: 123,
                    error: None,
                }
            )
            .await
            .unwrap());
        let record = journal.get(&intent.key).await.unwrap().unwrap();
        assert_eq!(record.status, SubmissionStatus::Finalized);
        assert!(runtime.resolve("fixture-wallet").is_none());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}
