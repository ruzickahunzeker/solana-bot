//! Offline-only runtime: synthetic signing/broadcast counters and temporary SQLite.
use super::*;
use crate::{
    adapters::persistence::submission_journal::SqliteSubmissionJournal,
    application::ports::{
        quote_cache::QuoteBinding,
        quote_execution::QuoteSubmissionBackend,
        quote_submit::{QuoteSubmissionRuntime, QuoteSubmissionWallet},
        submission_journal::SubmissionJournal,
    },
    domain::{
        quote::{BoundQuote, QuoteEstimate, QuotePreflight},
        submission::{
            Reservation, SignedSubmission, SubmissionIntent, SubmissionKey, SubmissionRecord,
            SubmissionStatus,
        },
        trade::Address,
    },
};
use anyhow::{bail, Result};
use serde_json::Value;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};
use tokio::sync::Barrier;

#[derive(Clone, Default)]
pub(crate) struct Faults {
    pub no_journal: bool,
    pub cache: bool,
    pub get_at: Option<usize>,
    pub reserve: bool,
    pub record_signed: bool,
    pub record_broadcast: bool,
    pub prepare: bool,
    pub sign: bool,
    pub timeout: bool,
    pub forbidden: bool,
    pub expire_after_cache: bool,
    pub replace_wallet: bool,
}

#[derive(Default)]
pub(crate) struct Trace {
    pub faults: Mutex<Faults>,
    pub wallets: Mutex<BTreeMap<String, Address>>,
    pub barrier: Mutex<Option<Arc<Barrier>>>,
    pub reads: Mutex<Vec<(String, String, String)>>,
    pub prepared: Mutex<Vec<BoundQuote>>,
    events: Mutex<Vec<&'static str>>,
    gets: AtomicUsize,
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
        self.gets.store(0, Ordering::SeqCst);
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
    pool: SqlitePool,
    address: Address,
}
pub(crate) struct Journal {
    trace: Arc<Trace>,
    inner: SqliteSubmissionJournal,
}
pub(crate) struct Cache {
    pub trace: Arc<Trace>,
    pub inner: Arc<crate::aggregator_quote::QuoteStore>,
}

impl QuoteSubmissionRuntime for Runtime {
    type Wallet = Wallet;
    type Journal = Journal;
    fn resolve(&self, id: &str) -> Option<Wallet> {
        self.trace.push("resolve");
        self.trace
            .wallets
            .lock()
            .unwrap()
            .get(id)
            .map(|address| Wallet {
                trace: self.trace.clone(),
                pool: self.pool.clone(),
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
impl Drop for Wallet {
    fn drop(&mut self) {
        self.trace.push("drop");
    }
}
impl QuoteSubmissionWallet for Wallet {
    fn pubkey(&self) -> Address {
        self.trace.push("pubkey");
        self.address
    }
}
impl QuoteSubmissionBackend for Wallet {
    type Prepared = ();
    fn ensure_live_allowed(&self) -> Result<()> {
        self.trace.push("live_allowed");
        if self.trace.faults().forbidden {
            bail!(crate::domain::execution_policy::BROADCAST_DISABLED_ERROR);
        }
        Ok(())
    }
    async fn prepare(&self, bound: &BoundQuote) -> Result<((), QuotePreflight)> {
        self.trace.push("prepare");
        self.trace.prepared.lock().unwrap().push(bound.clone());
        let faults = self.trace.faults();
        if faults.prepare {
            bail!("synthetic prepare failure https://rpc.example.invalid/private");
        }
        if faults.replace_wallet {
            self.trace
                .wallets
                .lock()
                .unwrap()
                .insert(bound.wallet_id.clone(), Address::from_bytes([9; 32]));
        }
        let barrier = self.trace.barrier.lock().unwrap().clone();
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        Ok(((), report(bound)))
    }
    fn sign(&self, _: ()) -> Result<SignedSubmission> {
        self.trace.push("sign");
        if self.trace.faults().sign {
            bail!("synthetic signer unavailable");
        }
        Ok(SignedSubmission {
            signature: "synthetic-signature".into(),
            bytes: vec![1, 2, 3],
        })
    }
    async fn broadcast(&self, signed: &SignedSubmission) -> Result<String> {
        let persisted: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM quote_submissions WHERE status='signed' AND signature=? AND signed_transaction=?",
        )
        .bind(&signed.signature)
        .bind(&signed.bytes)
        .fetch_one(&self.pool)
        .await?;
        assert!(
            persisted > 0,
            "synthetic broadcast requires persisted signed bytes"
        );
        self.trace.push("broadcast");
        if self.trace.faults().timeout {
            bail!("synthetic RPC timeout https://rpc.example.invalid/private");
        }
        Ok(signed.signature.clone())
    }
}
impl SubmissionJournal for Journal {
    async fn get(&self, key: &SubmissionKey) -> Result<Option<SubmissionRecord>> {
        self.trace.push("get");
        let nth = self.trace.gets.fetch_add(1, Ordering::SeqCst) + 1;
        if self.trace.faults().get_at == Some(nth) {
            bail!("synthetic journal unavailable");
        }
        self.inner.get(key).await
    }
    async fn reserve(&self, intent: &SubmissionIntent, report: &Value) -> Result<Reservation> {
        self.trace.push("reserve");
        if self.trace.faults().reserve {
            bail!("synthetic reservation failure");
        }
        self.inner.reserve(intent, report).await
    }
    async fn record_signed(&self, id: &str, signed: &SignedSubmission) -> Result<()> {
        self.trace.push("record_signed");
        if self.trace.faults().record_signed {
            bail!("synthetic signature write failure");
        }
        self.inner.record_signed(id, signed).await
    }
    async fn fail_before_sign(&self, id: &str, message: &str) -> Result<()> {
        self.trace.push("fail_before_sign");
        self.inner.fail_before_sign(id, message).await
    }
    async fn record_broadcast(
        &self,
        id: &str,
        status: SubmissionStatus,
        error: Option<&str>,
    ) -> Result<()> {
        self.trace.push("record_broadcast");
        if self.trace.faults().record_broadcast {
            bail!("synthetic broadcast write failure");
        }
        self.inner.record_broadcast(id, status, error).await
    }
}
impl QuoteCache for Cache {
    fn bind(&self, binding: QuoteBinding<'_>, quote: QuoteEstimate) -> Result<BoundQuote> {
        self.inner.bind(binding, quote)
    }
    fn read(&self, id: &str, wallet: &str, payer: &str) -> Result<BoundQuote> {
        self.trace.push("cache");
        self.trace
            .reads
            .lock()
            .unwrap()
            .push((id.into(), wallet.into(), payer.into()));
        if self.trace.faults().cache {
            bail!("quote not found or service restarted");
        }
        let mut bound = self.inner.read(id, wallet, payer)?;
        if self.trace.faults().expire_after_cache {
            bound.expires_at_ms = 0;
        }
        Ok(bound)
    }
}

pub(crate) fn report(bound: &BoundQuote) -> QuotePreflight {
    let value: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap();
    let mut result: QuotePreflight = serde_json::from_value(value["preflight"].clone()).unwrap();
    result.quote_id = bound.quote_id.clone();
    result.wallet_id = bound.wallet_id.clone();
    assert!(result.executable);
    result
}

pub(crate) struct Harness {
    pub service: Arc<dyn SolBuySubmissions>,
    pub trace: Arc<Trace>,
    pub cache: Arc<Cache>,
    pub pool: SqlitePool,
    pub bound: BoundQuote,
}
impl Harness {
    pub fn command(&self, simulate: bool) -> QuoteSubmissionCommand {
        QuoteSubmissionCommand {
            wallet_id: self.bound.wallet_id.clone(),
            idempotency_key: "fixture-key".into(),
            confirmed: !simulate,
            simulate,
        }
    }
    pub fn journal(&self) -> SqliteSubmissionJournal {
        SqliteSubmissionJournal::new(self.pool.clone())
    }
}

pub(crate) async fn harness() -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    harness_with_pool(pool).await
}

pub(crate) async fn harness_with_pool(pool: SqlitePool) -> Harness {
    SqliteSubmissionJournal::new(pool.clone())
        .initialize()
        .await
        .unwrap();
    let trace = Arc::new(Trace::default());
    let address = Address::from_bytes([1; 32]);
    let value: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap();
    let fixture: BoundQuote = serde_json::from_value(value["bound"].clone()).unwrap();
    trace
        .wallets
        .lock()
        .unwrap()
        .insert(fixture.wallet_id.clone(), address);
    let cache = Arc::new(Cache {
        trace: trace.clone(),
        inner: Arc::default(),
    });
    let bound = cache
        .bind(
            QuoteBinding {
                wallet_id: &fixture.wallet_id,
                wallet_pubkey: address.to_string(),
                slippage_bps: fixture.quote_slippage_bps,
            },
            fixture.quote,
        )
        .unwrap();
    let service = Arc::new(SolBuySubmissionService::new(
        Runtime {
            trace: trace.clone(),
            pool: pool.clone(),
        },
        cache.clone(),
    ));
    Harness {
        service,
        trace,
        cache,
        pool,
        bound,
    }
}
