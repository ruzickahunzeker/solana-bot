use super::*;
use crate::{
    application::{
        ordinary_submission,
        ports::{ordinary_claims::ExecutionOwner, ordinary_sol_buy::SolBuyExecutionFuture},
    },
    persistence::TradePersistence,
};

struct ControlledBackend {
    trace: Arc<Trace>,
    evidence: Arc<Evidence>,
    persistence: Arc<TradePersistence>,
    finished: Arc<tokio::sync::Notify>,
    return_gate: Option<Arc<ReturnGate>>,
}
struct ControlledWallet {
    legacy: LegacyWallet,
    persistence: Arc<TradePersistence>,
    finished: Arc<tokio::sync::Notify>,
    return_gate: Option<Arc<ReturnGate>>,
}

#[derive(Default)]
struct ReturnGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl Drop for ControlledWallet {
    fn drop(&mut self) {
        self.finished.notify_one();
    }
}

impl OrdinarySolBuyBackend for ControlledBackend {
    fn resolve(&self, wallet_id: &str) -> Option<Box<dyn OrdinarySolBuyWallet>> {
        self.trace
            .aggregate_resolves
            .lock()
            .unwrap()
            .push(wallet_id.into());
        Some(Box::new(ControlledWallet {
            legacy: LegacyWallet {
                trace: self.trace.clone(),
                evidence: self.evidence.clone(),
            },
            persistence: self.persistence.clone(),
            finished: self.finished.clone(),
            return_gate: self.return_gate.clone(),
        }))
    }
}
impl OrdinarySolBuyWallet for ControlledWallet {
    fn buy<'a>(
        &'a self,
        execution: &'a crate::application::trade_admission::AdmittedTrade,
        owner: Option<&'a ExecutionOwner>,
    ) -> SolBuyExecutionFuture<'a> {
        Box::pin(async move {
            let owner = owner.expect("original durable owner");
            owner.ensure_binding(execution)?;
            execution
                .reservation()
                .ensure_wallet("alpha", Address::from_bytes([1; 32]))?;
            self.legacy
                .trace
                .aggregate_commands
                .lock()
                .unwrap()
                .push(execution.command().clone());
            let journal = self
                .persistence
                .ordinary_execution_claims()
                .expect("test durable journal");
            let outcome = ordinary_submission::execute(
                &journal,
                &self.legacy,
                &self.legacy,
                execution,
                owner,
            )
            .await;
            if let Some(gate) = &self.return_gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            Ok(outcome)
        })
    }
}

async fn setup(receipt: Receipt) -> (AppState, Arc<Trace>, Arc<Evidence>) {
    let (state, trace, evidence, _) = setup_controlled(receipt, None).await;
    (state, trace, evidence)
}

async fn setup_controlled(
    receipt: Receipt,
    return_gate: Option<Arc<ReturnGate>>,
) -> (
    AppState,
    Arc<Trace>,
    Arc<Evidence>,
    Arc<tokio::sync::Notify>,
) {
    let (mut state, trace, evidence) =
        with_legacy_backend(TradePersistenceMode::Durable, receipt).await;
    let finished = Arc::new(tokio::sync::Notify::new());
    state.ordinary_sol_buy = Arc::new(ControlledBackend {
        trace: trace.clone(),
        evidence: evidence.clone(),
        persistence: state.persistence.clone(),
        finished: finished.clone(),
        return_gate,
    });
    (state, trace, evidence, finished)
}

#[tokio::test]
async fn ordinary_observation_http_finality_is_visible_without_get_triggering_a_new_lookup() {
    use crate::domain::reconciliation::{ChainCommitment, ChainObservation};
    for (receipt, error, expected) in [
        (Receipt::Match, None, "confirmed"),
        (Receipt::Error("synthetic send timeout"), None, "confirmed"),
        (Receipt::Match, Some("finalized execution error"), "failed"),
    ] {
        let (state, trace, evidence, finished) = setup_controlled(receipt, None).await;
        *evidence.observation.lock().unwrap() = Some(ChainObservation {
            signature: ORIGINAL.into(),
            commitment: ChainCommitment::Finalized,
            slot: 123,
            error: error.map(str::to_owned),
        });
        let app = http::router(state.clone());
        let request = aggregate_request();
        let response = send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await;
        let status = response.status();
        assert!(matches!(status, StatusCode::OK | StatusCode::ACCEPTED));
        let first = json_body(response, status).await;
        assert_eq!(first.as_object().unwrap().len(), 8);
        tokio::time::timeout(std::time::Duration::from_secs(2), finished.notified())
            .await
            .unwrap();
        let path = format!("/v1/trades/{}", first["request_id"].as_str().unwrap());
        for _ in 0..2 {
            let record = json_body(
                send(&app, "GET", &path, LOCAL, Some(TOKEN), None).await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(record["status"], expected);
            assert_eq!(record["signature"], ORIGINAL);
            assert!(!record.to_string().contains("final_observation_json"));
            assert!(!record.to_string().contains("signed_transaction"));
        }
        let repeated = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(request),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(repeated["request_id"], first["request_id"]);
        assert_eq!(repeated["status"], expected);
        assert_eq!(repeated["signature"], ORIGINAL);
        assert_eq!(repeated["duplicate"], true);
        assert_eq!(trace.aggregate_commands.lock().unwrap().len(), 1);
        let events = evidence.events.lock().unwrap();
        for event in ["sign", "broadcast", "lookup_signature"] {
            assert_eq!(events.iter().filter(|value| **value == event).count(), 1);
        }
    }
}

#[tokio::test]
async fn ordinary_observation_http_write_failure_preserves_submitted_signature_without_retry() {
    use crate::domain::reconciliation::{ChainCommitment, ChainObservation};
    let (state, trace, evidence, finished) = setup_controlled(Receipt::Match, None).await;
    *evidence.observation.lock().unwrap() = Some(ChainObservation {
        signature: ORIGINAL.into(),
        commitment: ChainCommitment::Finalized,
        slot: 123,
        error: None,
    });
    let pool = state.persistence.pool_for_tests();
    sqlx::raw_sql("CREATE TRIGGER deny_final_observation BEFORE UPDATE OF final_observation_json ON ordinary_execution_claims BEGIN SELECT RAISE(ABORT,'synthetic observation write failure'); END;")
        .execute(&pool).await.unwrap();
    let app = http::router(state.clone());
    let request = aggregate_request();
    let first = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), finished.notified())
        .await
        .unwrap();
    let path = format!("/v1/trades/{}", first["request_id"].as_str().unwrap());
    let stored = json_body(
        send(&app, "GET", &path, LOCAL, Some(TOKEN), None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(stored["status"], "submitted");
    assert_eq!(stored["signature"], ORIGINAL);
    let finality: Option<String> =
        sqlx::query_scalar("SELECT final_observation_json FROM ordinary_execution_claims")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(finality.is_none());
    let repeated = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(repeated["request_id"], first["request_id"]);
    assert_eq!(repeated["duplicate"], true);
    assert_eq!(trace.aggregate_commands.lock().unwrap().len(), 1);
    let events = evidence.events.lock().unwrap();
    for event in ["sign", "broadcast", "lookup_signature"] {
        assert_eq!(events.iter().filter(|value| **value == event).count(), 1);
    }
}

#[tokio::test]
async fn journaled_http_preserves_wire_and_original_signature_without_exposing_owner_or_bytes() {
    for (receipt, status) in [
        (Receipt::Match, "submitted"),
        (Receipt::Different, "unknown"),
        (
            Receipt::Error("sendTransaction failed: https://rpc.example.invalid/private-token"),
            "unknown",
        ),
    ] {
        let (state, trace, evidence) = setup(receipt).await;
        let app = http::router(state.clone());
        let request = aggregate_request();
        let first = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(request.clone()),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(first.as_object().unwrap().len(), 8);
        assert_eq!(first["status"], status);
        assert_eq!(first["signature"], ORIGINAL);
        let saved: (String, Vec<u8>, String) = sqlx::query_as(
            "SELECT signed_signature,signed_transaction,owner_nonce FROM ordinary_execution_claims",
        )
        .fetch_one(&state.persistence.pool_for_tests())
        .await
        .unwrap();
        assert_eq!(saved.0, ORIGINAL);
        assert_eq!(saved.1, ORIGINAL_BYTES);
        assert_eq!(saved.2.len(), 32);
        assert!(!first.to_string().contains(&saved.2));
        assert!(!first.to_string().contains("private-token"));
        let record = terminal(&app, first["request_id"].as_str().unwrap()).await;
        assert_eq!(record["status"], status);
        assert_eq!(record["signature"], ORIGINAL);
        let duplicate = json_body(
            send(
                &app,
                "POST",
                "/v1/trades",
                LOCAL,
                Some(TOKEN),
                Some(request),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(duplicate["request_id"], first["request_id"]);
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(trace.aggregate_commands.lock().unwrap().len(), 1);
        let events = evidence.events.lock().unwrap();
        assert_eq!(events.iter().filter(|event| **event == "sign").count(), 1);
        assert_eq!(
            events.iter().filter(|event| **event == "broadcast").count(),
            1
        );
        assert!(!events.contains(&"submit"));
    }
}

#[tokio::test]
async fn signed_blob_write_failure_does_not_send_or_blindly_mirror_unpersisted_signature() {
    let (mut state, trace, evidence) = setup(Receipt::Match).await;
    state.edit_fixture_config(|config| config.trade_response_wait_ms = 30);
    let pool = state.persistence.pool_for_tests();
    sqlx::raw_sql("CREATE TRIGGER deny_signed_blob BEFORE UPDATE OF signed_transaction ON ordinary_execution_claims WHEN NEW.signed_transaction IS NOT NULL BEGIN SELECT RAISE(ABORT,'synthetic signed write failure'); END;").execute(&pool).await.unwrap();
    let app = http::router(state.clone());
    let request = aggregate_request();
    let first = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request.clone()),
        )
        .await,
        StatusCode::ACCEPTED,
    )
    .await;
    let id = first["request_id"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if state.trades.get(id).is_some_and(|record| {
                record.status == crate::domain::trade::TradeStatus::Unknown
                    && record.signature.as_deref() == Some(ORIGINAL)
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let stored = state.persistence.get(id).await.unwrap().unwrap();
    assert_eq!(stored.status, crate::domain::trade::TradeStatus::BuildingTx);
    assert!(stored.signature.is_none());
    let signed: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT signed_transaction FROM ordinary_execution_claims")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(signed.is_none());
    let duplicate = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(request),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(duplicate["request_id"], first["request_id"]);
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(trace.aggregate_commands.lock().unwrap().len(), 1);
    let events = evidence.events.lock().unwrap();
    assert_eq!(events.iter().filter(|event| **event == "sign").count(), 1);
    assert!(!events.contains(&"broadcast"));
}

#[tokio::test]
async fn executor_never_overwrites_a_stronger_parent_snapshot_after_journal_broadcast() {
    let gate = Arc::new(ReturnGate::default());
    let (state, trace, evidence, finished) =
        setup_controlled(Receipt::Match, Some(gate.clone())).await;
    sqlx::raw_sql("CREATE TRIGGER stronger_parent AFTER UPDATE OF status ON trades WHEN NEW.status='submitted' BEGIN UPDATE trades SET status='confirmed',error=NULL WHERE request_id=NEW.request_id; END;")
        .execute(&state.persistence.pool_for_tests()).await.unwrap();
    sqlx::raw_sql("CREATE TABLE confirmed_parent_writes(request_id TEXT NOT NULL); CREATE TRIGGER track_confirmed_parent_writes AFTER UPDATE ON trades WHEN OLD.status='confirmed' BEGIN INSERT INTO confirmed_parent_writes(request_id) VALUES(NEW.request_id); END;")
        .execute(&state.persistence.pool_for_tests()).await.unwrap();
    let app = http::router(state.clone());
    let first = json_body(
        send(
            &app,
            "POST",
            "/v1/trades",
            LOCAL,
            Some(TOKEN),
            Some(aggregate_request()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(first["status"], "confirmed");
    assert_eq!(first["signature"], ORIGINAL);
    tokio::time::timeout(std::time::Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    let id = first["request_id"].as_str().unwrap();
    assert_eq!(
        state.trades.get(id).unwrap().status,
        crate::domain::trade::TradeStatus::BuildingTx
    );
    let persisted = state.persistence.get(id).await.unwrap().unwrap();
    assert_eq!(
        persisted.status,
        crate::domain::trade::TradeStatus::Confirmed
    );
    assert_eq!(persisted.signature.as_deref(), Some(ORIGINAL));
    gate.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), finished.notified())
        .await
        .unwrap();
    assert_eq!(
        state
            .trades
            .get(first["request_id"].as_str().unwrap())
            .unwrap()
            .status,
        crate::domain::trade::TradeStatus::Confirmed
    );
    let persisted = state.persistence.get(id).await.unwrap().unwrap();
    assert_eq!(
        persisted.status,
        crate::domain::trade::TradeStatus::Confirmed
    );
    assert_eq!(persisted.signature.as_deref(), Some(ORIGINAL));
    assert_eq!(trace.aggregate_commands.lock().unwrap().len(), 1);
    let later_writes: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM confirmed_parent_writes WHERE request_id=?")
            .bind(id)
            .fetch_one(&state.persistence.pool_for_tests())
            .await
            .unwrap();
    assert_eq!(later_writes, 0);
    let events = evidence.events.lock().unwrap();
    assert_eq!(events.iter().filter(|event| **event == "sign").count(), 1);
    assert_eq!(
        events.iter().filter(|event| **event == "broadcast").count(),
        1
    );
}
