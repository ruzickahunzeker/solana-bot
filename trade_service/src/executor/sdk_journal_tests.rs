use super::*;
use crate::{
    application::{
        ordinary_claims::test_support::{admit_request, fixture, Fixture},
        ports::ordinary_execution::{
            JournaledSdkFuture, OrdinaryExecutionBackend, OrdinaryExecutionWallet,
            TradeExecutionFuture,
        },
    },
    config::TradePersistenceMode,
    domain::{
        blockhash::Blockhash,
        execution_policy::BroadcastPolicy,
        trade::{Address, Pool},
    },
    persistence::TradePersistence,
    state::TradeStore,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Notify;

#[derive(Default)]
struct Trace {
    resolves: AtomicUsize,
    sdk: AtomicUsize,
    legacy: AtomicUsize,
    missing: AtomicBool,
    block: AtomicBool,
    started: Notify,
    release: Notify,
}
struct Backend {
    trace: Arc<Trace>,
    journal: Arc<TradePersistence>,
    index: Arc<TradeStore>,
    request_id: String,
}
struct Wallet {
    trace: Arc<Trace>,
    journal: Arc<TradePersistence>,
}
impl OrdinaryExecutionBackend for Backend {
    fn resolve(&self, id: &str) -> Option<Box<dyn OrdinaryExecutionWallet>> {
        assert_eq!(id, "fixture-wallet");
        assert_eq!(
            self.index.get(&self.request_id).unwrap().status,
            TradeStatus::BuildingTx
        );
        self.trace.resolves.fetch_add(1, Ordering::SeqCst);
        if self.trace.missing.load(Ordering::SeqCst) {
            None
        } else {
            Some(Box::new(Wallet {
                trace: self.trace.clone(),
                journal: self.journal.clone(),
            }))
        }
    }
}
impl OrdinaryExecutionWallet for Wallet {
    fn sdk_journaled<'a>(
        &'a self,
        execution: &'a AdmittedTrade,
        owner: ExecutionOwner,
        context: SdkExecutionContext,
    ) -> JournaledSdkFuture<'a> {
        Box::pin(async move {
            assert_eq!(context.blockhash.to_bytes(), [42; 32]);
            owner.ensure_binding(execution).unwrap();
            let journal = self.journal.ordinary_execution_claims().unwrap();
            assert_eq!(
                journal.claim(execution).await.unwrap(),
                ExecutionClaim::AlreadyHeld
            );
            assert_eq!(
                journal
                    .observe_sdk_submission(execution, &owner)
                    .await
                    .unwrap()
                    .status,
                TradeStatus::BuildingTx
            );
            self.trace.sdk.fetch_add(1, Ordering::SeqCst);
            self.trace.started.notify_one();
            if self.trace.block.load(Ordering::SeqCst) {
                self.trace.release.notified().await;
            }
            crate::application::ordinary_sdk_submission::reject_preparation(
                &journal,
                execution,
                owner,
                anyhow::anyhow!("synthetic SDK preparation failure, no signer used"),
            )
            .await
        })
    }
    fn sdk<'a>(
        &'a self,
        _command: &'a TradeCommand,
        _context: SdkExecutionContext,
    ) -> TradeExecutionFuture<'a> {
        Box::pin(async move {
            self.trace.legacy.fetch_add(1, Ordering::SeqCst);
            Ok(ExecuteResult {
                success: true,
                signature: Some("synthetic-legacy-signature".into()),
                error: None,
            })
        })
    }
    fn pumpapi<'a>(&'a self, _command: &'a TradeCommand, _wait: bool) -> TradeExecutionFuture<'a> {
        panic!("PumpAPI is not part of the SDK journal dispatch tests")
    }
}

async fn setup(
    mode: TradePersistenceMode,
    simulate: bool,
    buy: bool,
    pool: Pool,
) -> (Fixture, Arc<Trace>) {
    setup_payment(mode, simulate, buy, pool, true).await
}
async fn setup_payment(
    mode: TradePersistenceMode,
    simulate: bool,
    buy: bool,
    pool: Pool,
    native: bool,
) -> (Fixture, Arc<Trace>) {
    let state = fixture(mode).await.state;
    let request = serde_json::from_value(serde_json::json!({
        "wallet_id":"fixture-wallet", "side":if buy {"buy"} else {"sell"}, "pool":pool,
        "mint":Address::from_bytes([2;32]).to_string(), "quote_mint":if native {crate::domain::trade::WSOL_MINT} else {crate::domain::trade::USDC_MINT},
        "pool_id":(pool != Pool::Pump).then(|| Address::from_bytes([3;32]).to_string()),
        "amount_sol":(buy&&native).then_some(0.001), "amount_quote_units":(buy&&!native).then_some(100), "sell_percent":(!buy).then_some(100),
        "simulate":simulate, "execution_method":"sdk", "use_mev":false,
        "idempotency_key":" original-sdk-runtime-literal "
    })).unwrap();
    let mut f = admit_request(state, request).await;
    f.state.cached_blockhash = Arc::new(crate::BlockhashCache::with_snapshot_for_tests(
        Blockhash::from_bytes([42; 32]),
        crate::state::now_ms(),
    ));
    let trace = Arc::new(Trace::default());
    f.state.ordinary_execution = Arc::new(Backend {
        trace: trace.clone(),
        journal: f.state.persistence.clone(),
        index: f.state.trades.clone(),
        request_id: f.record.request_id.clone(),
    });
    (f, trace)
}
async fn run(f: &Fixture) {
    execute_with_claims(f.state.clone(), &f.execution, &f.claims()).await;
}

#[tokio::test]
async fn sdk_runtime_dispatch_uses_original_durable_owner_without_legacy_sdk_or_duplicate_execution(
) {
    let (f, trace) = setup(
        TradePersistenceMode::Durable,
        false,
        false,
        Pool::MeteoraDammV2,
    )
    .await;
    run(&f).await;
    let before = f
        .state
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.status, TradeStatus::Failed);
    assert!(before.signature.is_none());
    assert!(before.error.as_deref().unwrap().contains("no signer used"));
    run(&f).await;
    assert_eq!(trace.sdk.load(Ordering::SeqCst), 1);
    assert_eq!(trace.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(trace.legacy.load(Ordering::SeqCst), 0);
    let after = f
        .state
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    assert_eq!(
        f.state.trades.get(&f.record.request_id).unwrap().status,
        TradeStatus::Failed
    );
}

#[tokio::test]
async fn sdk_runtime_dispatch_does_not_divert_memory_shadow_simulation_buy_or_other_protocols() {
    for (mode, simulate, buy, pool) in [
        (
            TradePersistenceMode::Memory,
            false,
            false,
            Pool::MeteoraDammV2,
        ),
        (
            TradePersistenceMode::Shadow,
            false,
            false,
            Pool::MeteoraDammV2,
        ),
        (
            TradePersistenceMode::Durable,
            true,
            false,
            Pool::MeteoraDammV2,
        ),
        (TradePersistenceMode::Durable, false, true, Pool::Pump),
        (TradePersistenceMode::Durable, false, false, Pool::Pump),
    ] {
        let (f, trace) = setup(mode, simulate, buy, pool).await;
        run(&f).await;
        assert_eq!(trace.sdk.load(Ordering::SeqCst), 0);
        assert_eq!(trace.legacy.load(Ordering::SeqCst), 1);
        if mode == TradePersistenceMode::Durable {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ordinary_execution_claims WHERE request_id=?",
            )
            .bind(&f.record.request_id)
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap();
            assert_eq!(count, 0);
        }
    }
}

#[tokio::test]
async fn sdk_runtime_dispatch_stale_hash_preserves_rejected_status_in_the_original_journal() {
    let (mut f, trace) = setup(
        TradePersistenceMode::Durable,
        false,
        false,
        Pool::MeteoraDammV2,
    )
    .await;
    f.state.cached_blockhash = Arc::new(crate::BlockhashCache::default());
    run(&f).await;
    let record = f
        .state
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, TradeStatus::Rejected);
    assert_eq!(
        record.error.as_deref(),
        Some("blockhash cache is empty or stale")
    );
    assert!(record.signature.is_none());
    assert_eq!(trace.sdk.load(Ordering::SeqCst), 0);
    assert_eq!(trace.legacy.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
}

#[tokio::test]
async fn sdk_runtime_dispatch_missing_wallet_is_a_journaled_presign_failure() {
    let (f, trace) = setup(
        TradePersistenceMode::Durable,
        false,
        false,
        Pool::MeteoraDammV2,
    )
    .await;
    trace.missing.store(true, Ordering::SeqCst);
    run(&f).await;
    let record = f
        .state
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, TradeStatus::Failed);
    assert_eq!(
        record.error.as_deref(),
        Some("wallet_id fixture-wallet not found")
    );
    assert_eq!(trace.sdk.load(Ordering::SeqCst), 0);
    assert_eq!(trace.legacy.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
}

#[tokio::test]
async fn sdk_runtime_dispatch_failed_rejection_write_never_falls_back_or_overwrites_original() {
    let (mut f, trace) = setup(
        TradePersistenceMode::Durable,
        false,
        false,
        Pool::MeteoraDammV2,
    )
    .await;
    f.state.cached_blockhash = Arc::new(crate::BlockhashCache::default());
    sqlx::raw_sql("CREATE TRIGGER deny_sdk_rejection BEFORE UPDATE OF submission_stage ON ordinary_execution_claims WHEN NEW.submission_stage='rejected' BEGIN SELECT RAISE(ABORT,'synthetic rejection write failure'); END")
        .execute(&f.state.persistence.pool_for_tests()).await.unwrap();
    run(&f).await;
    let parent = f
        .state
        .persistence
        .get(&f.record.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent.status, TradeStatus::BuildingTx);
    assert!(parent.signature.is_none());
    assert_eq!(
        f.state.trades.get(&f.record.request_id).unwrap().status,
        TradeStatus::Unknown
    );
    assert_eq!(trace.sdk.load(Ordering::SeqCst), 0);
    assert_eq!(trace.legacy.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.claims().claim(&f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
}

#[tokio::test]
async fn sdk_runtime_dispatch_disabled_policy_stops_before_claim_or_wallet_resolution() {
    let (mut f, trace) = setup(
        TradePersistenceMode::Durable,
        false,
        false,
        Pool::MeteoraDammV2,
    )
    .await;
    f.state
        .edit_fixture_config(|config| config.security.broadcast = BroadcastPolicy::Disabled);
    run(&f).await;
    assert_eq!(trace.resolves.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TradeStatus::Rejected
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&f.record.request_id)
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn sdk_runtime_dispatch_racing_executors_resolve_and_enter_only_once() {
    let (f, trace) = setup(
        TradePersistenceMode::Durable,
        false,
        false,
        Pool::MeteoraDammV2,
    )
    .await;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let state = f.state.clone();
        let execution = f.execution.clone();
        tasks.push(tokio::spawn(async move {
            let claims = RuntimeExecutionClaims::new(
                state.persistence.clone(),
                state.trades.clone(),
                state.idempotency.clone(),
            );
            execute_with_claims(state, &execution, &claims).await;
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(trace.sdk.load(Ordering::SeqCst), 1);
    assert_eq!(trace.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(trace.legacy.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sdk_runtime_dispatch_cancellation_does_not_reclaim_or_restart_the_same_order() {
    let (f, trace) = setup(
        TradePersistenceMode::Durable,
        false,
        false,
        Pool::MeteoraDammV2,
    )
    .await;
    trace.block.store(true, Ordering::SeqCst);
    let state = f.state.clone();
    let execution = f.execution.clone();
    let task = tokio::spawn(async move {
        let claims = RuntimeExecutionClaims::new(
            state.persistence.clone(),
            state.trades.clone(),
            state.idempotency.clone(),
        );
        execute_with_claims(state, &execution, &claims).await;
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), trace.started.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    run(&f).await;
    assert_eq!(trace.sdk.load(Ordering::SeqCst), 1);
    assert_eq!(trace.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        TradeStatus::BuildingTx
    );
}

#[tokio::test]
async fn sdk_runtime_dispatch_buy_uses_the_original_owner_once_for_native_and_usdc_without_legacy()
{
    for native in [false, true] {
        let (f, trace) = setup_payment(
            TradePersistenceMode::Durable,
            false,
            true,
            Pool::MeteoraDammV2,
            native,
        )
        .await;
        run(&f).await;
        run(&f).await;
        assert_eq!(trace.sdk.load(Ordering::SeqCst), 1);
        assert_eq!(trace.resolves.load(Ordering::SeqCst), 1);
        assert_eq!(trace.legacy.load(Ordering::SeqCst), 0);
        let record = f
            .state
            .persistence
            .get(&f.record.request_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, TradeStatus::Failed);
        assert!(record.signature.is_none());
        assert!(record.error.as_deref().unwrap().contains("no signer used"));
        assert_eq!(
            f.claims().claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_runtime_dispatch_buy_disabled_policy_and_stale_hash_never_enter_sdk_or_fallback() {
    for native in [false, true] {
        for disabled in [false, true] {
            let (mut f, trace) = setup_payment(
                TradePersistenceMode::Durable,
                false,
                true,
                Pool::MeteoraDammV2,
                native,
            )
            .await;
            if disabled {
                f.state.edit_fixture_config(|config| {
                    config.security.broadcast = BroadcastPolicy::Disabled
                });
            } else {
                f.state.cached_blockhash = Arc::new(crate::BlockhashCache::default());
            }
            run(&f).await;
            // Preserve the original order: policy precedes resolution, hash follows resolution.
            assert_eq!(
                trace.resolves.load(Ordering::SeqCst),
                usize::from(!disabled)
            );
            assert_eq!(trace.sdk.load(Ordering::SeqCst), 0);
            assert_eq!(trace.legacy.load(Ordering::SeqCst), 0);
            let record = f
                .state
                .persistence
                .get(&f.record.request_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(record.status, TradeStatus::Rejected);
            assert!(record.signature.is_none());
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM ordinary_execution_claims WHERE request_id=?",
            )
            .bind(&f.record.request_id)
            .fetch_one(&f.state.persistence.pool_for_tests())
            .await
            .unwrap();
            assert_eq!(count, i64::from(!disabled));
            run(&f).await;
            assert_eq!(
                trace.resolves.load(Ordering::SeqCst),
                usize::from(!disabled)
            );
            assert_eq!(trace.sdk.load(Ordering::SeqCst), 0);
            assert_eq!(trace.legacy.load(Ordering::SeqCst), 0);
        }
    }
}
