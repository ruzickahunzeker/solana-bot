use super::*;
use crate::{
    application::ports::{
        quote_cache::QuoteBinding,
        quote_preflight::{QuotePreflightFuture, QuotePreflightWallet},
    },
    domain::{
        quote::{BoundQuote, QuoteEstimate},
        trade::Address,
    },
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

fn fixture_value() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap()
}
fn bound() -> BoundQuote {
    let mut value: BoundQuote = serde_json::from_value(fixture_value()["bound"].clone()).unwrap();
    value.wallet_pubkey = Address::from_bytes([7; 32]).to_string();
    value
}
struct Trace {
    events: Mutex<Vec<&'static str>>,
    ids: Mutex<Vec<String>>,
    reads: Mutex<Vec<(String, String, String)>>,
    address: Mutex<Address>,
    available: AtomicBool,
    remove_during_preflight: AtomicBool,
    prepare_error: AtomicBool,
    fail_read: Mutex<Option<usize>>,
    report: Mutex<QuotePreflight>,
}
impl Trace {
    fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
}
struct Backend(Arc<Trace>);
struct Wallet {
    trace: Arc<Trace>,
    address: Address,
}
struct Cache(Arc<Trace>);
impl QuotePreflightBackend for Backend {
    fn resolve(&self, id: &str) -> Option<Box<dyn QuotePreflightWallet>> {
        self.0.events.lock().unwrap().push("resolve");
        self.0.ids.lock().unwrap().push(id.into());
        self.0.available.load(Ordering::SeqCst).then(|| {
            Box::new(Wallet {
                trace: self.0.clone(),
                address: *self.0.address.lock().unwrap(),
            }) as Box<dyn QuotePreflightWallet>
        })
    }
}
impl QuotePreflightWallet for Wallet {
    fn pubkey(&self) -> Address {
        self.trace.events.lock().unwrap().push("pubkey");
        self.address
    }
    fn inspect<'a>(&'a self, quote: &'a BoundQuote) -> QuotePreflightFuture<'a> {
        Box::pin(async move {
            self.trace.events.lock().unwrap().push("preflight");
            assert_eq!(*quote, bound());
            if self.trace.remove_during_preflight.load(Ordering::SeqCst) {
                self.trace.available.store(false, Ordering::SeqCst);
                *self.trace.address.lock().unwrap() = Address::from_bytes([9; 32]);
            }
            tokio::task::yield_now().await;
            if self.trace.prepare_error.load(Ordering::SeqCst) {
                anyhow::bail!("synthetic preflight failure");
            }
            Ok(self.trace.report.lock().unwrap().clone())
        })
    }
}
impl Drop for Wallet {
    fn drop(&mut self) {
        self.trace.events.lock().unwrap().push("drop");
    }
}
impl QuoteCache for Cache {
    fn bind(&self, _: QuoteBinding<'_>, _: QuoteEstimate) -> anyhow::Result<BoundQuote> {
        panic!("preflight cannot write quote cache")
    }
    fn read(&self, id: &str, wallet: &str, payer: &str) -> anyhow::Result<BoundQuote> {
        self.0.events.lock().unwrap().push("read");
        let count = {
            let mut reads = self.0.reads.lock().unwrap();
            reads.push((id.into(), wallet.into(), payer.into()));
            reads.len()
        };
        if *self.0.fail_read.lock().unwrap() == Some(count) {
            anyhow::bail!("synthetic cache error {count}");
        }
        Ok(bound())
    }
}
fn fixture() -> (Arc<dyn QuotePreflights>, Arc<Trace>) {
    let trace = Arc::new(Trace {
        events: Mutex::default(),
        ids: Mutex::default(),
        reads: Mutex::default(),
        address: Mutex::new(Address::from_bytes([7; 32])),
        available: AtomicBool::new(true),
        remove_during_preflight: AtomicBool::new(false),
        prepare_error: AtomicBool::new(false),
        fail_read: Mutex::new(None),
        report: Mutex::new(serde_json::from_value(fixture_value()["preflight"].clone()).unwrap()),
    });
    let service = QuotePreflightService::new(
        Arc::new(Backend(trace.clone())),
        Arc::new(Cache(trace.clone())),
    );
    (Arc::new(service), trace)
}

#[tokio::test]
async fn preflight_service_returns_complete_report_only_after_second_cache_check() {
    let (service, trace) = fixture();
    assert!(trace.events().is_empty());
    let report = service
        .inspect("fixture-quote-v1", "fixture-wallet")
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(report).unwrap(),
        fixture_value()["preflight"]
    );
    assert_eq!(
        trace.events(),
        [
            "resolve",
            "pubkey",
            "read",
            "preflight",
            "pubkey",
            "read",
            "drop"
        ]
    );
    assert_eq!(&*trace.ids.lock().unwrap(), &["fixture-wallet"]);
    let reads = trace.reads.lock().unwrap();
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0], reads[1]);
}

#[tokio::test]
async fn preflight_service_missing_wallet_precedes_every_cache_and_preparation_error() {
    let (service, trace) = fixture();
    trace.available.store(false, Ordering::SeqCst);
    *trace.fail_read.lock().unwrap() = Some(1);
    trace.prepare_error.store(true, Ordering::SeqCst);
    assert!(matches!(
        service.inspect("missing", "").await,
        Err(QuotePreflightError::WalletMissing)
    ));
    assert_eq!(trace.events(), ["resolve"]);
    assert!(trace.reads.lock().unwrap().is_empty());
    assert_eq!(&*trace.ids.lock().unwrap(), &[""]);
}

#[tokio::test]
async fn preflight_service_first_cache_failure_never_prepares_and_drops_the_handle() {
    let (service, trace) = fixture();
    *trace.fail_read.lock().unwrap() = Some(1);
    trace.prepare_error.store(true, Ordering::SeqCst);
    let Err(QuotePreflightError::Cache(error)) = service.inspect("missing", "fixture-wallet").await
    else {
        panic!("expected first cache error")
    };
    assert_eq!(error.to_string(), "synthetic cache error 1");
    assert_eq!(trace.events(), ["resolve", "pubkey", "read", "drop"]);
}

#[tokio::test]
async fn preflight_service_backend_failure_skips_second_cache_and_preserves_error_priority() {
    let (service, trace) = fixture();
    trace.prepare_error.store(true, Ordering::SeqCst);
    *trace.fail_read.lock().unwrap() = Some(2);
    let Err(QuotePreflightError::Backend(error)) =
        service.inspect("fixture-quote-v1", "fixture-wallet").await
    else {
        panic!("expected preparation error")
    };
    assert_eq!(error.to_string(), "synthetic preflight failure");
    assert_eq!(
        trace.events(),
        ["resolve", "pubkey", "read", "preflight", "drop"]
    );
    assert_eq!(trace.reads.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn preflight_service_expiry_after_success_discards_report_and_returns_second_cache_error() {
    let (service, trace) = fixture();
    *trace.fail_read.lock().unwrap() = Some(2);
    let Err(QuotePreflightError::Cache(error)) =
        service.inspect("fixture-quote-v1", "fixture-wallet").await
    else {
        panic!("expected second cache error")
    };
    assert_eq!(error.to_string(), "synthetic cache error 2");
    assert_eq!(
        trace.events(),
        [
            "resolve",
            "pubkey",
            "read",
            "preflight",
            "pubkey",
            "read",
            "drop"
        ]
    );
}

#[tokio::test]
async fn preflight_service_retains_original_wallet_across_registry_removal_and_replacement() {
    let (service, trace) = fixture();
    trace.remove_during_preflight.store(true, Ordering::SeqCst);
    service
        .inspect("fixture-quote-v1", "fixture-wallet")
        .await
        .unwrap();
    assert_eq!(&*trace.ids.lock().unwrap(), &["fixture-wallet"]);
    assert!(!trace.available.load(Ordering::SeqCst));
    assert_eq!(*trace.address.lock().unwrap(), Address::from_bytes([9; 32]));
    let reads = trace.reads.lock().unwrap();
    assert_eq!(reads.len(), 2);
    for read in reads.iter() {
        assert_eq!(read.2, Address::from_bytes([7; 32]).to_string());
    }
}

#[tokio::test]
async fn preflight_service_preserves_literal_ids_and_nonexecutable_report_without_new_policy() {
    for (id, wallet) in [
        ("fixture-quote-v1", "fixture-wallet"),
        (" literal quote ", " literal wallet "),
        ("", ""),
    ] {
        let (service, trace) = fixture();
        let expected = {
            let mut report = trace.report.lock().unwrap();
            report.executable = false;
            report.local_simulation_ok = false;
            report.reason = "literal simulation failure".into();
            report.clone()
        };
        let observed = service.inspect(id, wallet).await.unwrap();
        assert_eq!(observed, expected);
        assert_eq!(&*trace.ids.lock().unwrap(), &[wallet]);
        for read in trace.reads.lock().unwrap().iter() {
            assert_eq!(read.0, id);
            assert_eq!(read.1, wallet);
        }
        assert_eq!(trace.reads.lock().unwrap().len(), 2);
    }
}
