use super::*;
use crate::{
    application::{
        ports::{
            ordinary_execution::{
                OrdinaryExecutionBackend, OrdinaryExecutionWallet, SdkExecutionContext,
                TradeExecutionFuture,
            },
            wallet_identity::{LoadedWalletDirectory, LoadedWalletIdentity},
        },
        trade_command::TradeCommand,
        trade_execution::TradeExecutionResult,
    },
    domain::{blockhash::Blockhash, trade::Address},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
struct Trace {
    dropped: AtomicUsize,
    entered: Notify,
    release: Notify,
    calls: AtomicUsize,
}
struct Identity(Arc<Trace>);
impl Drop for Identity {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl LoadedWalletIdentity for Identity {
    fn pubkey(&self) -> Address {
        Address::from_bytes([9; 32])
    }
}
struct Directory(Arc<Trace>);
impl LoadedWalletDirectory for Directory {
    fn resolve_id(&self, requested: Option<&str>) -> String {
        requested.unwrap_or("loaded").into()
    }
    fn lookup(&self, id: &str) -> Option<Box<dyn LoadedWalletIdentity>> {
        assert_eq!(id, "loaded");
        Some(Box::new(Identity(self.0.clone())))
    }
}
struct Backend(Arc<Trace>);
impl OrdinaryExecutionBackend for Backend {
    fn resolve(&self, id: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        assert_eq!(id, "loaded");
        Some(Box::new(Backend(self.0.clone())))
    }
}
impl OrdinaryExecutionWallet for Backend {
    fn sdk<'a>(
        &'a self,
        command: &'a TradeCommand,
        _: SdkExecutionContext,
    ) -> TradeExecutionFuture<'a> {
        Box::pin(async move {
            assert_eq!(command.wallet_id, "loaded");
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            self.0.entered.notify_one();
            self.0.release.notified().await;
            Ok(TradeExecutionResult {
                success: true,
                signature: Some("synthetic-intake-result".into()),
                error: None,
            })
        })
    }
    fn pumpapi<'a>(&'a self, _: &'a TradeCommand, _: bool) -> TradeExecutionFuture<'a> {
        Box::pin(async { panic!("no PumpAPI execution in this compatibility fixture") })
    }
}

#[tokio::test]
async fn ordinary_intake_http_retains_wallet_identity_lease_through_dispatch_and_response_wait() {
    let mut state = state().await;
    let config = Arc::get_mut(&mut state.config).unwrap();
    config.trade_response_wait_ms = 5000;
    config.blockhash_max_age_ms = u64::MAX;
    state.blockhash_cache = Arc::new(BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([9; 32]),
        crate::state::now_ms(),
    ));
    let trace = Arc::new(Trace::default());
    state.wallet_identity = Arc::new(Directory(trace.clone()));
    state.ordinary_execution = Arc::new(Backend(trace.clone()));
    let app = http::router(state);
    let task = tokio::spawn(async move {
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(json!({
                "side":"buy","pool":"pump","mint":"11111111111111111111111111111111",
                "amount_lamports":17,"simulate":false,"idempotency_key":"intake-lease"
            })),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), trace.entered.notified())
        .await
        .unwrap();
    assert!(!task.is_finished());
    assert_eq!(trace.dropped.load(Ordering::SeqCst), 0);
    trace.release.notify_one();
    let response = tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    let body = json_body(response, StatusCode::OK).await;
    assert_eq!(body["status"], "submitted");
    assert_eq!(body["signature"], "synthetic-intake-result");
    assert_eq!(trace.calls.load(Ordering::SeqCst), 1);
    assert_eq!(trace.dropped.load(Ordering::SeqCst), 1);
}
