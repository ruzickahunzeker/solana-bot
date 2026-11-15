use super::*;
use crate::{
    application::{
        ports::{
            quote_cache::{QuoteBinding, QuoteCache},
            quote_preflight::{QuotePreflightBackend, QuotePreflightFuture, QuotePreflightWallet},
        },
        quote_preflight::QuotePreflightService,
    },
    domain::{
        execution_policy::BroadcastPolicy,
        quote::{BoundQuote, QuoteEstimate, QuotePreflight},
        trade::Address,
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

#[derive(Default)]
struct Trace {
    events: Mutex<Vec<&'static str>>,
    ids: Mutex<Vec<String>>,
    reads: Mutex<Vec<(String, String, String)>>,
    wallets: Mutex<BTreeMap<String, Address>>,
    quotes: Mutex<Vec<BoundQuote>>,
    backend_error: AtomicBool,
    nonexecutable: AtomicBool,
    remove_during_prepare: AtomicBool,
    replace_during_prepare: AtomicBool,
    read_error: Mutex<Option<(usize, &'static str)>>,
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
struct Cache {
    trace: Arc<Trace>,
    inner: Arc<crate::aggregator_quote::QuoteStore>,
}
fn report(quote: &BoundQuote, nonexecutable: bool) -> QuotePreflight {
    let value: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap();
    let mut result: QuotePreflight = serde_json::from_value(value["preflight"].clone()).unwrap();
    result.quote_id = quote.quote_id.clone();
    result.wallet_id = quote.wallet_id.clone();
    if nonexecutable {
        result.executable = false;
        result.local_simulation_ok = false;
        result.reason = "synthetic simulation failure".into();
    }
    result
}
impl QuotePreflightBackend for Backend {
    fn resolve(&self, id: &str) -> Option<Box<dyn QuotePreflightWallet>> {
        self.0.events.lock().unwrap().push("resolve");
        self.0.ids.lock().unwrap().push(id.into());
        self.0.wallets.lock().unwrap().get(id).map(|address| {
            Box::new(Wallet {
                trace: self.0.clone(),
                address: *address,
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
            self.trace.quotes.lock().unwrap().push(quote.clone());
            if self.trace.remove_during_prepare.load(Ordering::SeqCst) {
                self.trace.wallets.lock().unwrap().remove(&quote.wallet_id);
            }
            if self.trace.replace_during_prepare.load(Ordering::SeqCst) {
                self.trace
                    .wallets
                    .lock()
                    .unwrap()
                    .insert(quote.wallet_id.clone(), Address::from_bytes([9; 32]));
            }
            tokio::task::yield_now().await;
            if self.trace.backend_error.load(Ordering::SeqCst) {
                anyhow::bail!("preflight RPC https://rpc.example.invalid/token timed out");
            }
            Ok(report(
                quote,
                self.trace.nonexecutable.load(Ordering::SeqCst),
            ))
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
        panic!("preflight must not create or refresh cached quotes")
    }
    fn read(&self, id: &str, wallet: &str, payer: &str) -> anyhow::Result<BoundQuote> {
        self.trace.events.lock().unwrap().push("read");
        let count = {
            let mut reads = self.trace.reads.lock().unwrap();
            reads.push((id.into(), wallet.into(), payer.into()));
            reads.len()
        };
        if let Some((index, error)) = *self.trace.read_error.lock().unwrap() {
            if index == count {
                anyhow::bail!("{error}");
            }
        }
        self.inner.read(id, wallet, payer)
    }
}
async fn with_preflight_source() -> (AppState, Arc<Trace>, BoundQuote) {
    let mut state = state().await;
    let trace = Arc::new(Trace::default());
    let quote = bound();
    let address = Address::from_bytes([7; 32]);
    trace
        .wallets
        .lock()
        .unwrap()
        .insert(quote.wallet_id.clone(), address);
    let cached = state
        .aggregator_quotes
        .bind(
            QuoteBinding {
                wallet_id: &quote.wallet_id,
                wallet_pubkey: address.to_string(),
                slippage_bps: quote.quote_slippage_bps,
            },
            quote.quote,
        )
        .unwrap();
    state.quote_preflights = Arc::new(QuotePreflightService::new(
        Arc::new(Backend(trace.clone())),
        Arc::new(Cache {
            trace: trace.clone(),
            inner: state.aggregator_quotes.clone(),
        }),
    ));
    (state, trace, cached)
}
fn path(quote: &BoundQuote) -> String {
    format!(
        "/v1/quotes/sol-buy/{}/preflight?wallet_id={}",
        quote.quote_id, quote.wallet_id
    )
}

fn bound() -> BoundQuote {
    let value: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap();
    serde_json::from_value(value["bound"].clone()).unwrap()
}

#[tokio::test]
async fn preflight_missing_loaded_wallet_baseline_precedes_cache_and_keeps_auth_and_query_rules() {
    for disabled in [false, true] {
        let mut state = state().await;
        if disabled {
            Arc::get_mut(&mut state.config).unwrap().security.broadcast = BroadcastPolicy::Disabled;
        }
        let quote = bound();
        let cached = state
            .aggregator_quotes
            .bind(
                QuoteBinding {
                    wallet_id: &quote.wallet_id,
                    wallet_pubkey: quote.wallet_pubkey.clone(),
                    slippage_bps: quote.quote_slippage_bps,
                },
                quote.quote,
            )
            .unwrap();
        let app = http::router(state.clone());
        for id in [cached.quote_id.as_str(), "missing"] {
            for wallet in ["fixture-wallet", ""] {
                error_body(
                    send(
                        &app,
                        "POST",
                        &format!("/v1/quotes/sol-buy/{id}/preflight?wallet_id={wallet}"),
                        LOCAL,
                        Some(TOKEN),
                        None,
                    )
                    .await,
                    StatusCode::BAD_REQUEST,
                    "wallet_id not found",
                )
                .await;
            }
        }
        let path = format!(
            "/v1/quotes/sol-buy/{}/preflight?wallet_id=fixture-wallet",
            cached.quote_id
        );
        assert_eq!(
            send(&app, "POST", &path, LOCAL, None, None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            send(
                &app,
                "POST",
                "/v1/quotes/sol-buy/missing/preflight",
                LOCAL,
                None,
                None
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(state.trades.recent(10).is_empty());
        assert_eq!(state.wallets.wallet_count(), 0);
    }
}

#[tokio::test]
async fn preflight_http_returns_complete_report_and_keeps_nonexecutable_results_read_only() {
    for disabled in [false, true] {
        for nonexecutable in [false, true] {
            let (mut state, trace, cached) = with_preflight_source().await;
            if disabled {
                Arc::get_mut(&mut state.config).unwrap().security.broadcast =
                    BroadcastPolicy::Disabled;
            }
            trace.nonexecutable.store(nonexecutable, Ordering::SeqCst);
            let response = json_body(
                send(
                    &http::router(state.clone()),
                    "POST",
                    &path(&cached),
                    LOCAL,
                    Some(TOKEN),
                    None,
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(
                response,
                serde_json::to_value(report(&cached, nonexecutable)).unwrap()
            );
            assert_eq!(response.as_object().unwrap().len(), 32);
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
            assert_eq!(&*trace.ids.lock().unwrap(), &[cached.wallet_id.clone()]);
            assert_eq!(&*trace.quotes.lock().unwrap(), &[cached]);
            assert_eq!(state.wallets.wallet_count(), 0);
            assert!(state.trades.recent(10).is_empty());
        }
    }
}

#[tokio::test]
async fn preflight_http_preserves_wallet_first_cache_and_late_expiry_status_priority() {
    for stage in ["wallet", "first-cache", "last-cache"] {
        let (state, trace, cached) = with_preflight_source().await;
        if stage == "wallet" {
            trace.wallets.lock().unwrap().clear();
        }
        *trace.read_error.lock().unwrap() =
            Some((if stage == "last-cache" { 2 } else { 1 }, "quote expired"));
        error_body(
            send(
                &http::router(state),
                "POST",
                &path(&cached),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            if stage == "wallet" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::GONE
            },
            if stage == "wallet" {
                "wallet_id not found"
            } else {
                "quote expired"
            },
        )
        .await;
        assert_eq!(
            trace.events(),
            match stage {
                "wallet" => vec!["resolve"],
                "first-cache" => vec!["resolve", "pubkey", "read", "drop"],
                _ => vec![
                    "resolve",
                    "pubkey",
                    "read",
                    "preflight",
                    "pubkey",
                    "read",
                    "drop"
                ],
            }
        );
    }
}

#[tokio::test]
async fn preflight_http_backend_failure_redacts_url_and_skips_second_cache_check() {
    let (state, trace, cached) = with_preflight_source().await;
    trace.backend_error.store(true, Ordering::SeqCst);
    *trace.read_error.lock().unwrap() = Some((2, "quote expired"));
    error_body(
        send(
            &http::router(state),
            "POST",
            &path(&cached),
            LOCAL,
            Some(TOKEN),
            None,
        )
        .await,
        StatusCode::BAD_GATEWAY,
        "preflight RPC <redacted-url> timed out",
    )
    .await;
    assert_eq!(
        trace.events(),
        ["resolve", "pubkey", "read", "preflight", "drop"]
    );
    assert_eq!(trace.reads.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn preflight_http_retains_original_wallet_for_recheck_after_registry_changes() {
    for remove in [false, true] {
        let (state, trace, cached) = with_preflight_source().await;
        trace.remove_during_prepare.store(remove, Ordering::SeqCst);
        trace
            .replace_during_prepare
            .store(!remove, Ordering::SeqCst);
        let response = json_body(
            send(
                &http::router(state),
                "POST",
                &path(&cached),
                LOCAL,
                Some(TOKEN),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(
            response,
            serde_json::to_value(report(&cached, false)).unwrap()
        );
        assert_eq!(&*trace.ids.lock().unwrap(), &[cached.wallet_id.clone()]);
        let reads = trace.reads.lock().unwrap();
        assert_eq!(reads.len(), 2);
        for read in reads.iter() {
            assert_eq!(read.2, cached.wallet_pubkey);
        }
        let wallets = trace.wallets.lock().unwrap();
        if remove {
            assert!(!wallets.contains_key(&cached.wallet_id));
        } else {
            assert_eq!(wallets[&cached.wallet_id], Address::from_bytes([9; 32]));
        }
    }
}

#[tokio::test]
async fn preflight_http_auth_query_extraction_and_post_only_route_never_reach_backend() {
    let (state, trace, cached) = with_preflight_source().await;
    let app = http::router(state);
    assert_eq!(
        send(&app, "POST", &path(&cached), LOCAL, None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "POST", &path(&cached), DENIED, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/quotes/sol-buy/missing/preflight",
            LOCAL,
            None,
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&app, "GET", &path(&cached), LOCAL, Some(TOKEN), None)
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert!(trace.events().is_empty());
    assert!(trace.ids.lock().unwrap().is_empty());
}
