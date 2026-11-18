use std::sync::Arc;

use crate::{
    adapters::{
        persistence::quote_reconciliation::QuoteReconciliationJournal,
        rpc::{
            quote_status::RpcQuoteStatusReader,
            read_http::{ReadHttpTransport, ReqwestReadTransport},
            read_runtime::SharedReadRuntime,
        },
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
    read_rpc: Arc<SharedReadRuntime>,
}

impl SdkQuoteReconciliationRuntime {
    pub(crate) fn new(
        wallets: Arc<WalletRegistry>,
        persistence: Arc<TradePersistence>,
        read_rpc: Arc<SharedReadRuntime>,
    ) -> Self {
        Self {
            wallets,
            persistence,
            read_rpc,
        }
    }
}

pub(crate) struct ResolvedReconciliationWallet {
    client: Arc<SdkTradeClient>,
    read_rpc: Arc<SharedReadRuntime>,
}

pub(crate) struct QuoteStatusReader<T = ReqwestReadTransport> {
    inner: RpcQuoteStatusReader<T>,
}
impl<T: ReadHttpTransport> QuoteStatusReader<T> {
    fn from_selected(
        runtime: Arc<SharedReadRuntime<T>>,
        rpcs: Vec<Arc<solana_client::nonblocking::rpc_client::RpcClient>>,
    ) -> Self {
        Self {
            inner: RpcQuoteStatusReader::new(
                runtime,
                rpcs.into_iter().take(2).map(|rpc| rpc.url()).collect(),
            ),
        }
    }
}

impl QuoteReconciliationRuntime for SdkQuoteReconciliationRuntime {
    type Wallet = ResolvedReconciliationWallet;
    type Journal = QuoteReconciliationJournal;

    fn resolve(&self, wallet_id: &str) -> Option<Self::Wallet> {
        self.wallets
            .get(wallet_id)
            .map(|client| ResolvedReconciliationWallet {
                client,
                read_rpc: self.read_rpc.clone(),
            })
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
        QuoteStatusReader::from_selected(self.read_rpc.clone(), self.client.rpcs())
    }

    fn pubkey(&self) -> Address {
        Address::from_bytes(self.client.wallet().to_bytes())
    }
}

impl<T: ReadHttpTransport> SignatureStatusReader for QuoteStatusReader<T> {
    async fn lookup_signature(&self, signature: &str) -> anyhow::Result<Option<ChainObservation>> {
        self.inner.lookup_signature(signature).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quote_status_adapter_actual_reader_uses_original_two_url_snapshots_not_execution_clients(
    ) {
        use crate::adapters::rpc::read_runtime::quote_status_fixture as f;
        let old_a = f::Legacy::new(f::PRIMARY, f::value(serde_json::json!([null])));
        let old_b = f::Legacy::new(f::BACKUP, f::found());
        let old_c = f::Legacy::new(f::THIRD, f::found());
        let a = old_a.client();
        let b = old_b.client();
        let c = old_c.client();
        let weak = [Arc::downgrade(&a), Arc::downgrade(&b), Arc::downgrade(&c)];
        let runtime = f::runtime(vec![
            (
                f::PRIMARY,
                vec![f::Reply::Good(f::value(serde_json::json!([null])))],
            ),
            (f::BACKUP, vec![f::Reply::Good(f::found())]),
        ]);
        let runtime_weak = Arc::downgrade(&runtime);
        let reader = QuoteStatusReader::from_selected(runtime.clone(), vec![a, b, c]);
        assert!(weak.iter().all(|rpc| rpc.upgrade().is_none()));
        assert!(reader
            .lookup_signature(&f::signature())
            .await
            .unwrap()
            .is_some());
        assert!(old_a.calls.lock().unwrap().is_empty());
        assert!(old_b.calls.lock().unwrap().is_empty());
        assert!(old_c.calls.lock().unwrap().is_empty());
        let calls = runtime.test_transport().calls.lock().unwrap();
        assert_eq!(
            calls.iter().map(|call| call.0.as_str()).collect::<Vec<_>>(),
            [f::PRIMARY, f::BACKUP]
        );
        drop(calls);
        drop(runtime);
        assert!(runtime_weak.upgrade().is_some());
        drop(reader);
        assert!(runtime_weak.upgrade().is_none());
    }

    #[tokio::test]
    async fn quote_status_adapter_runtime_releases_same_read_registry_and_persistence_resources() {
        let state = crate::http_contract_tests::state().await;
        let read_rpc = Arc::new(SharedReadRuntime::new(&[]).unwrap());
        let read_weak = Arc::downgrade(&read_rpc);
        let wallets = Arc::downgrade(&state.wallets);
        let persistence = Arc::downgrade(&state.persistence);
        let runtime = SdkQuoteReconciliationRuntime::new(
            state.wallets.clone(),
            state.persistence.clone(),
            read_rpc.clone(),
        );
        assert!(Arc::ptr_eq(&runtime.read_rpc, &read_rpc));
        drop((read_rpc, state));
        assert!(read_weak.upgrade().is_some());
        assert!(wallets.upgrade().is_some());
        assert!(persistence.upgrade().is_some());
        drop(runtime);
        assert!(read_weak.upgrade().is_none());
        assert!(wallets.upgrade().is_none());
        assert!(persistence.upgrade().is_none());
    }

    #[tokio::test]
    async fn quote_reconciliation_runtime_reuses_empty_registry_without_sdk_or_journal_init() {
        let state = crate::http_contract_tests::state().await;
        let read_rpc = Arc::new(SharedReadRuntime::new(&[]).unwrap());
        let runtime = SdkQuoteReconciliationRuntime::new(
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
        let read_rpc = Arc::new(SharedReadRuntime::new(&[]).unwrap());
        let runtime = SdkQuoteReconciliationRuntime::new(
            state.wallets.clone(),
            persistence.clone(),
            read_rpc,
        );
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
