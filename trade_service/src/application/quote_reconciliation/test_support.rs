//! Temporary SQLite and observation-only fake runtime; no signer or broadcast backend.
use super::*;
use crate::{
    adapters::persistence::submission_journal::SqliteSubmissionJournal,
    application::ports::{
        reconciliation::{ReconciliationJournal, SignatureStatusReader},
        submission_journal::SubmissionJournal,
    },
    domain::{
        reconciliation::{ChainCommitment, ChainObservation},
        submission::{Reservation, SignedSubmission, SubmissionIntent, SubmissionRecord},
        trade::Address,
    },
};
use anyhow::{bail, Result};
use serde_json::json;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Barrier;

pub(crate) const SIGNATURE: &str = "synthetic-signature";
pub(crate) const WALLET: &str = "fixture-wallet";
pub(crate) const KEY: &str = "fixture-key";
pub(crate) const QUOTE: &str = "fixture-quote";

#[derive(Clone)]
pub(crate) enum Answer {
    Found(ChainCommitment, Option<String>),
    Missing,
    Error,
    Pending,
    Mismatch,
}
#[derive(Clone, Default)]
pub(crate) struct Faults {
    pub no_journal: bool,
    pub lookup_at: Option<usize>,
    pub observe: bool,
    pub replace_wallet: bool,
    pub remove_wallet: bool,
}
pub(crate) struct Trace {
    events: Mutex<Vec<&'static str>>,
    lookups: AtomicUsize,
    pub faults: Mutex<Faults>,
    pub wallets: Mutex<BTreeMap<String, Address>>,
    pub resolved_ids: Mutex<Vec<String>>,
    pub lookup_keys: Mutex<Vec<SubmissionKey>>,
    pub signatures: Mutex<Vec<String>>,
    pub answer: Mutex<Answer>,
    pub barrier: Mutex<Option<Arc<Barrier>>>,
}
impl Trace {
    pub fn push(&self, event: &'static str) {
        self.events.lock().unwrap().push(event);
    }
    pub fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
        self.lookups.store(0, Ordering::SeqCst);
    }
    pub fn faults(&self) -> Faults {
        self.faults.lock().unwrap().clone()
    }
}
pub(crate) struct Runtime {
    pub trace: Arc<Trace>,
    pub pool: SqlitePool,
}
pub(crate) struct Wallet {
    trace: Arc<Trace>,
    address: Address,
}
pub(crate) struct Reader {
    trace: Arc<Trace>,
    answer: Answer,
}
pub(crate) struct Journal {
    trace: Arc<Trace>,
    inner: SqliteSubmissionJournal,
}
impl QuoteReconciliationRuntime for Runtime {
    type Wallet = Wallet;
    type Journal = Journal;

    fn resolve(&self, id: &str) -> Option<Wallet> {
        self.trace.push("resolve");
        self.trace.resolved_ids.lock().unwrap().push(id.into());
        self.trace
            .wallets
            .lock()
            .unwrap()
            .get(id)
            .map(|address| Wallet {
                trace: self.trace.clone(),
                address: *address,
            })
    }
    fn journal(&self) -> Option<Journal> {
        self.trace.push("journal");
        (!self.trace.faults().no_journal).then(|| Journal {
            trace: self.trace.clone(),
            inner: SqliteSubmissionJournal::new(self.pool.clone()),
        })
    }
}
impl QuoteReconciliationWallet for Wallet {
    type Reader = Reader;

    fn reader(&self) -> Reader {
        self.trace.push("reader");
        Reader {
            trace: self.trace.clone(),
            answer: self.trace.answer.lock().unwrap().clone(),
        }
    }
    fn pubkey(&self) -> Address {
        self.trace.push("pubkey");
        self.address
    }
}
impl Drop for Wallet {
    fn drop(&mut self) {
        self.trace.push("drop");
    }
}
impl SignatureStatusReader for Reader {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        self.trace.push("rpc");
        self.trace.signatures.lock().unwrap().push(signature.into());
        let faults = self.trace.faults();
        if faults.replace_wallet {
            self.trace
                .wallets
                .lock()
                .unwrap()
                .insert(WALLET.into(), Address::from_bytes([9; 32]));
        }
        if faults.remove_wallet {
            self.trace.wallets.lock().unwrap().remove(WALLET);
        }
        let barrier = self.trace.barrier.lock().unwrap().clone();
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        match &self.answer {
            Answer::Found(commitment, error) => Ok(Some(ChainObservation {
                signature: signature.into(),
                commitment: *commitment,
                slot: 100 + u64::try_from(commitment.rank()).unwrap(),
                error: error.clone(),
            })),
            Answer::Missing => Ok(None),
            Answer::Error => bail!("synthetic lookup failure https://rpc.example.invalid/private"),
            Answer::Pending => std::future::pending().await,
            Answer::Mismatch => Ok(Some(ChainObservation {
                signature: "wrong-signature".into(),
                commitment: ChainCommitment::Finalized,
                slot: 999,
                error: None,
            })),
        }
    }
}
impl ReconciliationJournal for Journal {
    async fn lookup(&self, key: &SubmissionKey) -> Result<Option<SubmissionRecord>> {
        self.trace.push("lookup");
        self.trace.lookup_keys.lock().unwrap().push(key.clone());
        let nth = self.trace.lookups.fetch_add(1, Ordering::SeqCst) + 1;
        if self.trace.faults().lookup_at == Some(nth) {
            bail!("synthetic journal read failure https://db.example.invalid/private");
        }
        self.inner.lookup(key).await
    }
    async fn record_observation(&self, id: &str, observation: &ChainObservation) -> Result<bool> {
        self.trace.push("observe");
        if self.trace.faults().observe {
            bail!("synthetic observation write failure https://db.example.invalid/private");
        }
        self.inner.record_observation(id, observation).await
    }
}

pub(crate) struct Harness {
    pub service: Arc<dyn QuoteReconciliations>,
    pub trace: Arc<Trace>,
    pub pool: SqlitePool,
    pub submission_id: String,
}
impl Harness {
    pub fn journal(&self) -> SqliteSubmissionJournal {
        SqliteSubmissionJournal::new(self.pool.clone())
    }
    pub async fn query(&self) -> Result<ReconciliationResult, SubmissionError> {
        self.service.reconcile(QUOTE, WALLET, KEY).await
    }
}

pub(crate) async fn harness(signed: bool) -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let journal = SqliteSubmissionJournal::new(pool.clone());
    journal.initialize().await.unwrap();
    let address = Address::from_bytes([1; 32]);
    let intent = SubmissionIntent {
        key: SubmissionKey::new(WALLET, KEY).unwrap(),
        wallet_pubkey: address.to_string(),
        quote_id: QUOTE.into(),
        quote: json!({"amount_lamports":9_007_199_254_740_993_u64}),
    };
    let Reservation::Created(record) = journal
        .reserve(&intent, &json!({"fixture":true}))
        .await
        .unwrap()
    else {
        panic!("new reservation required")
    };
    if signed {
        journal
            .record_signed(
                &record.submission_id,
                &SignedSubmission {
                    signature: SIGNATURE.into(),
                    bytes: vec![1, 2, 3],
                },
            )
            .await
            .unwrap();
    }
    let trace = Arc::new(Trace {
        events: Mutex::default(),
        lookups: AtomicUsize::new(0),
        faults: Mutex::default(),
        wallets: Mutex::new(BTreeMap::from([(WALLET.into(), address)])),
        resolved_ids: Mutex::default(),
        lookup_keys: Mutex::default(),
        signatures: Mutex::default(),
        answer: Mutex::new(Answer::Found(ChainCommitment::Confirmed, None)),
        barrier: Mutex::default(),
    });
    let service = Arc::new(QuoteReconciliationService::new(Runtime {
        trace: trace.clone(),
        pool: pool.clone(),
    }));
    Harness {
        service,
        trace,
        pool,
        submission_id: record.submission_id,
    }
}
