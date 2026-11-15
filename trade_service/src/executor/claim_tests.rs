use super::*;
use crate::{
    application::{
        ordinary_claims::test_support::{fixture, Fixture},
        ports::ordinary_sol_buy::{OrdinarySolBuyBackend, OrdinarySolBuyWallet},
    },
    config::TradePersistenceMode,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Notify;

#[derive(Default)]
struct Calls {
    resolve: AtomicUsize,
    buy: AtomicUsize,
}
struct Backend(Arc<Calls>);
struct Wallet(Arc<Calls>);

impl OrdinarySolBuyBackend for Backend {
    fn resolve(&self, _: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        self.0.resolve.fetch_add(1, Ordering::SeqCst);
        Some(Box::new(Wallet(self.0.clone())))
    }
}
impl OrdinarySolBuyWallet for Wallet {
    fn buy<'a>(
        &'a self,
        _: &'a AdmittedTrade,
        _owner: Option<&'a ExecutionOwner>,
    ) -> crate::application::ports::ordinary_sol_buy::SolBuyExecutionFuture<'a> {
        Box::pin(async move {
            self.0.buy.fetch_add(1, Ordering::SeqCst);
            Ok(
                crate::application::ports::ordinary_sol_buy::SolBuyExecutionOutcome::Legacy(
                    ExecuteResult {
                        success: true,
                        signature: Some("fake-original-claim-receipt".into()),
                        error: None,
                    },
                ),
            )
        })
    }
}
fn install(f: &mut Fixture) -> Arc<Calls> {
    let calls = Arc::new(Calls::default());
    f.state.ordinary_sol_buy = Arc::new(Backend(calls.clone()));
    calls
}
fn assert_calls(calls: &Calls, expected: usize) {
    assert_eq!(calls.resolve.load(Ordering::SeqCst), expected);
    assert_eq!(calls.buy.load(Ordering::SeqCst), expected);
}

#[tokio::test]
async fn repeated_claimed_execution_does_not_rebuild_or_overwrite_submitted_order() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let mut f = fixture(mode).await;
        let calls = install(&mut f);
        execute_with_claims(f.state.clone(), &f.execution, &f.claims()).await;
        let before = f.state.trades.get(&f.record.request_id).unwrap();
        assert_eq!(before.status, TradeStatus::Submitted);
        execute_with_claims(f.state.clone(), &f.execution, &f.claims()).await;
        assert_eq!(
            serde_json::to_value(f.state.trades.get(&f.record.request_id).unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
        assert_calls(&calls, 1);
    }
}

#[tokio::test]
async fn concurrent_executor_entries_only_allow_one_backend_invocation() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let mut f = fixture(mode).await;
        let calls = install(&mut f);
        let claims = Arc::new(f.claims());
        let mut tasks = vec![];
        for _ in 0..16 {
            let state = f.state.clone();
            let execution = f.execution.clone();
            let claims = claims.clone();
            tasks.push(tokio::spawn(async move {
                execute_with_claims(state, &execution, claims.as_ref()).await;
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_calls(&calls, 1);
        assert_eq!(
            f.state.trades.get(&f.record.request_id).unwrap().status,
            TradeStatus::Submitted
        );
    }
}

struct LostAck(RuntimeExecutionClaims);
impl OrdinaryExecutionClaimer for LostAck {
    async fn claim(&self, execution: &AdmittedTrade) -> anyhow::Result<ExecutionClaim> {
        let result = self.0.claim(execution).await?;
        if result.is_acquired() {
            anyhow::bail!("synthetic lost claim acknowledgement");
        }
        Ok(result)
    }
}

#[tokio::test]
async fn lost_claim_acknowledgement_stops_without_reclaim_fallback_or_status_changes() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let mut f = fixture(mode).await;
        let calls = install(&mut f);
        execute_with_claims(f.state.clone(), &f.execution, &LostAck(f.claims())).await;
        assert_eq!(
            f.state.trades.get(&f.record.request_id).unwrap().status,
            TradeStatus::Accepted
        );
        execute_with_claims(f.state.clone(), &f.execution, &f.claims()).await;
        assert_eq!(
            f.state.trades.get(&f.record.request_id).unwrap().status,
            TradeStatus::Accepted
        );
        assert_calls(&calls, 0);
        assert_eq!(
            f.claims().claim(&f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

struct HeldClaim {
    inner: RuntimeExecutionClaims,
    committed: Notify,
}
impl OrdinaryExecutionClaimer for HeldClaim {
    async fn claim(&self, execution: &AdmittedTrade) -> anyhow::Result<ExecutionClaim> {
        let result = self.inner.claim(execution).await?;
        self.committed.notify_one();
        std::future::pending::<()>().await;
        Ok(result)
    }
}

#[tokio::test]
async fn cancelled_executor_after_claim_never_releases_or_replays_the_original_operation() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        let mut f = fixture(mode).await;
        let calls = install(&mut f);
        let gate = Arc::new(HeldClaim {
            inner: f.claims(),
            committed: Notify::new(),
        });
        let state = f.state.clone();
        let execution = f.execution.clone();
        let task_gate = gate.clone();
        let task = tokio::spawn(async move {
            execute_with_claims(state, &execution, task_gate.as_ref()).await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), gate.committed.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        execute_with_claims(f.state.clone(), &f.execution, &f.claims()).await;
        assert_calls(&calls, 0);
        assert_eq!(
            f.state.trades.get(&f.record.request_id).unwrap().status,
            TradeStatus::Accepted
        );
    }
}

#[tokio::test]
async fn unavailable_durable_claim_storage_stops_before_policy_or_backend_mutations() {
    let mut f = fixture(TradePersistenceMode::Durable).await;
    let calls = install(&mut f);
    f.state.persistence.pool_for_tests().close().await;
    execute_with_claims(f.state.clone(), &f.execution, &f.claims()).await;
    assert_calls(&calls, 0);
    assert_eq!(
        f.state.trades.get(&f.record.request_id).unwrap().status,
        TradeStatus::Accepted
    );
}

#[tokio::test]
async fn terminal_parent_without_a_claim_is_not_reset_to_building_or_executed() {
    for mode in [
        TradePersistenceMode::Memory,
        TradePersistenceMode::Shadow,
        TradePersistenceMode::Durable,
    ] {
        for status in [
            TradeStatus::Submitted,
            TradeStatus::Failed,
            TradeStatus::Unknown,
        ] {
            let mut f = fixture(mode).await;
            let calls = install(&mut f);
            let record = f
                .state
                .trades
                .update(
                    &f.record.request_id,
                    status,
                    Some("original-terminal-signature".into()),
                    Some("original result".into()),
                )
                .unwrap();
            f.state.persistence.update(&record).await.unwrap();
            execute_with_claims(f.state.clone(), &f.execution, &f.claims()).await;
            assert_eq!(
                serde_json::to_value(f.state.trades.get(&f.record.request_id).unwrap()).unwrap(),
                serde_json::to_value(record).unwrap()
            );
            assert_calls(&calls, 0);
        }
    }
}

struct MustNotClaimSimulation;
impl OrdinaryExecutionClaimer for MustNotClaimSimulation {
    async fn claim(&self, _: &AdmittedTrade) -> anyhow::Result<ExecutionClaim> {
        panic!("simulation must not consume a live claim")
    }
}

#[tokio::test]
async fn ordinary_aggregate_simulation_does_not_consume_a_live_execution_claim() {
    let mut f = fixture(TradePersistenceMode::Memory).await;
    let calls = install(&mut f);
    let mut command = f.execution.command().clone();
    command.execution.mode = crate::domain::trade::ExecutionMode::Simulate;
    let input = AdmittedTrade::unreserved_fixture(&f.record, command);
    execute_with_claims(f.state.clone(), &input, &MustNotClaimSimulation).await;
    assert_calls(&calls, 1);
    assert_eq!(
        f.state.trades.get(&f.record.request_id).unwrap().status,
        TradeStatus::Simulated
    );
}
