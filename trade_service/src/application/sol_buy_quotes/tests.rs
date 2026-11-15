use super::*;
use crate::{application::ports::wallet_identity::LoadedWalletIdentity, domain::trade::Address};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    wallet_id: String,
    mint: String,
    amount_lamports: u64,
    slippage_bps: u16,
}
#[derive(Default)]
struct Trace {
    events: Mutex<Vec<&'static str>>,
    requests: Mutex<Vec<Request>>,
    lookups: Mutex<Vec<String>>,
    reads: Mutex<Vec<(String, String, String)>>,
    bound: Mutex<Option<BoundQuote>>,
    available: AtomicBool,
    remove_during_fetch: AtomicBool,
    failure: Mutex<Option<&'static str>>,
}
impl Trace {
    fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
    fn event(&self, name: &'static str) -> anyhow::Result<()> {
        self.events.lock().unwrap().push(name);
        if *self.failure.lock().unwrap() == Some(name) {
            anyhow::bail!("synthetic {name} failure");
        }
        Ok(())
    }
}
struct Source(Arc<Trace>);
struct Directory(Arc<Trace>);
struct Identity(Arc<Trace>);
struct Cache(Arc<Trace>);
fn bound() -> BoundQuote {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap();
    serde_json::from_value(fixture["bound"].clone()).unwrap()
}
impl SolBuyQuoteSource for Source {
    type Request = Request;
    fn map_request(&self, input: SolBuyQuoteInput<'_>) -> Request {
        self.0.events.lock().unwrap().push("map");
        let request = Request {
            wallet_id: input.wallet_id.into(),
            mint: input.mint.into(),
            amount_lamports: input.amount_lamports,
            slippage_bps: input.slippage_bps,
        };
        self.0.requests.lock().unwrap().push(request.clone());
        request
    }
    fn validate(&self, _: &Request) -> anyhow::Result<()> {
        self.0.event("validate")
    }
    async fn fetch(&self, _: &Request) -> anyhow::Result<crate::domain::quote::QuoteEstimate> {
        self.0.event("fetch")?;
        if self.0.remove_during_fetch.load(Ordering::SeqCst) {
            self.0.available.store(false, Ordering::SeqCst);
        }
        tokio::task::yield_now().await;
        Ok(bound().quote)
    }
}
impl LoadedWalletDirectory for Directory {
    fn resolve_id(&self, _: Option<&str>) -> String {
        panic!("explicit quote wallet must not resolve defaults")
    }
    fn lookup(&self, id: &str) -> Option<Box<dyn LoadedWalletIdentity>> {
        self.0.events.lock().unwrap().push("lookup");
        self.0.lookups.lock().unwrap().push(id.into());
        self.0
            .available
            .load(Ordering::SeqCst)
            .then(|| Box::new(Identity(self.0.clone())) as Box<dyn LoadedWalletIdentity>)
    }
}
impl LoadedWalletIdentity for Identity {
    fn pubkey(&self) -> Address {
        self.0.events.lock().unwrap().push("pubkey");
        Address::from_bytes([7; 32])
    }
}
impl Drop for Identity {
    fn drop(&mut self) {
        self.0.events.lock().unwrap().push("drop");
    }
}
impl QuoteCache for Cache {
    fn bind(
        &self,
        binding: QuoteBinding<'_>,
        quote: crate::domain::quote::QuoteEstimate,
    ) -> anyhow::Result<BoundQuote> {
        self.0.event("bind")?;
        let mut value = bound();
        value.wallet_id = binding.wallet_id.into();
        value.wallet_pubkey = binding.wallet_pubkey;
        value.quote_slippage_bps = binding.slippage_bps;
        value.quote = quote;
        *self.0.bound.lock().unwrap() = Some(value.clone());
        Ok(value)
    }
    fn read(&self, id: &str, wallet: &str, payer: &str) -> anyhow::Result<BoundQuote> {
        self.0.event("read")?;
        self.0
            .reads
            .lock()
            .unwrap()
            .push((id.into(), wallet.into(), payer.into()));
        Ok(self.0.bound.lock().unwrap().clone().unwrap_or_else(bound))
    }
}
fn fixture() -> (Arc<dyn SolBuyQuoteQueries>, Arc<Trace>) {
    let trace = Arc::new(Trace::default());
    trace.available.store(true, Ordering::SeqCst);
    let service = SolBuyQuoteService::new(
        Source(trace.clone()),
        Arc::new(Directory(trace.clone())),
        Arc::new(Cache(trace.clone())),
    );
    (Arc::new(service), trace)
}
fn input() -> SolBuyQuoteInput<'static> {
    SolBuyQuoteInput {
        wallet_id: " literal wallet ",
        mint: "literal-unvalidated-mint",
        amount_lamports: 9_007_199_254_740_993,
        slippage_bps: 1234,
    }
}

#[tokio::test]
async fn quote_queries_create_keeps_original_order_and_complete_integer_quote_values() {
    let (service, trace) = fixture();
    let result = service.create(input()).await.unwrap();
    assert_eq!(
        trace.events(),
        ["map", "validate", "lookup", "fetch", "pubkey", "bind", "drop"]
    );
    assert_eq!(result.quote, bound().quote);
    assert_eq!(result.quote.expected_output_units, u64::MAX);
    assert_eq!(result.wallet_id, input().wallet_id);
    assert_eq!(
        result.wallet_pubkey,
        Address::from_bytes([7; 32]).to_string()
    );
    assert_eq!(result.quote_slippage_bps, 1234);
    assert_eq!(
        &*trace.requests.lock().unwrap(),
        &[Request {
            wallet_id: input().wallet_id.into(),
            mint: input().mint.into(),
            amount_lamports: input().amount_lamports,
            slippage_bps: 1234
        }]
    );
    assert_eq!(&*trace.lookups.lock().unwrap(), &[input().wallet_id]);
}

#[tokio::test]
async fn quote_queries_failures_preserve_stage_kind_and_do_not_sample_payer_or_cache_early() {
    for (failure, expected) in [
        ("validate", vec!["map", "validate"]),
        ("wallet", vec!["map", "validate", "lookup"]),
        ("fetch", vec!["map", "validate", "lookup", "fetch", "drop"]),
        (
            "bind",
            vec![
                "map", "validate", "lookup", "fetch", "pubkey", "bind", "drop",
            ],
        ),
    ] {
        let (service, trace) = fixture();
        *trace.failure.lock().unwrap() = Some(failure);
        trace.available.store(failure != "wallet", Ordering::SeqCst);
        let error = service.create(input()).await.unwrap_err();
        let message = match error {
            QuoteQueryError::InvalidRequest(error) => {
                assert_eq!(failure, "validate");
                error.to_string()
            }
            QuoteQueryError::WalletMissing => {
                assert_eq!(failure, "wallet");
                "wallet_id not found".into()
            }
            QuoteQueryError::Backend(error) => {
                assert_eq!(failure, "fetch");
                error.to_string()
            }
            QuoteQueryError::CacheWrite(error) => {
                assert_eq!(failure, "bind");
                error.to_string()
            }
            other => panic!("unexpected stage: {other:?}"),
        };
        assert_eq!(
            message,
            if failure == "wallet" {
                "wallet_id not found".into()
            } else {
                format!("synthetic {failure} failure")
            }
        );
        assert_eq!(trace.events(), expected);
        assert!(trace.bound.lock().unwrap().is_none());
    }
}

#[test]
fn quote_queries_get_keeps_literal_wallet_lookup_before_read_and_skips_source_validation() {
    for wallet in ["", " literal wallet "] {
        let (service, trace) = fixture();
        service.get("literal-quote-id", wallet).unwrap();
        assert_eq!(trace.events(), ["lookup", "pubkey", "read", "drop"]);
        assert_eq!(&*trace.lookups.lock().unwrap(), &[wallet]);
        assert_eq!(
            &*trace.reads.lock().unwrap(),
            &[(
                "literal-quote-id".into(),
                wallet.into(),
                Address::from_bytes([7; 32]).to_string()
            )]
        );
        assert!(trace.requests.lock().unwrap().is_empty());
    }
}

#[test]
fn quote_queries_get_failure_priority_keeps_identity_lease_and_cache_error_separate() {
    for available in [false, true] {
        let (service, trace) = fixture();
        trace.available.store(available, Ordering::SeqCst);
        *trace.failure.lock().unwrap() = Some("read");
        match service.get("missing", "").unwrap_err() {
            QuoteQueryError::WalletMissing => assert!(!available),
            QuoteQueryError::CacheRead(error) => {
                assert!(available);
                assert_eq!(error.to_string(), "synthetic read failure");
            }
            other => panic!("unexpected get error: {other:?}"),
        }
        assert_eq!(
            trace.events(),
            if available {
                vec!["lookup", "pubkey", "read", "drop"]
            } else {
                vec!["lookup"]
            }
        );
    }
}

#[tokio::test]
async fn quote_queries_retains_loaded_identity_across_fetch_without_second_lookup() {
    let (service, trace) = fixture();
    assert!(trace.events().is_empty());
    trace.remove_during_fetch.store(true, Ordering::SeqCst);
    let result = service.create(input()).await.unwrap();
    assert_eq!(
        result.wallet_pubkey,
        Address::from_bytes([7; 32]).to_string()
    );
    assert!(!trace.available.load(Ordering::SeqCst));
    assert_eq!(&*trace.lookups.lock().unwrap(), &[input().wallet_id]);
    assert_eq!(
        trace.events(),
        ["map", "validate", "lookup", "fetch", "pubkey", "bind", "drop"]
    );
}
