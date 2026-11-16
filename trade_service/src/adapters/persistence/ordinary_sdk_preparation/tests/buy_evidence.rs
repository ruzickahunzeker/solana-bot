use super::*;
use crate::{
    adapters::sdk::prepared_candidate::test_support::buy,
    application::{
        ordinary_observation_scan::PendingOrdinaryObservations,
        ports::ordinary_reconciliation::RecoveredOrdinaryObservations,
    },
    domain::reconciliation::{ChainCommitment, ChainObservation},
};
use solana_sdk::signature::Signer;

struct BuyCase {
    f: Fixture,
    store: SqliteOrdinaryExecutionClaims,
    signature: String,
}

async fn historical(native: bool, stage: &str) -> BuyCase {
    let source = buy(native).await;
    let base = fixture(TradePersistenceMode::Durable).await;
    let f = admit_request_for_pubkey(
        base.state,
        source.request.clone(),
        source.key.pubkey().to_string(),
    )
    .await;
    assert!(f.claims().claim(&f.execution).await.is_err());
    let signed = source.signed();
    let parent_status = match stage {
        "signed" | "broadcasting" => TradeStatus::Submitting,
        "submitted" => TradeStatus::Submitted,
        "unknown" => TradeStatus::Unknown,
        _ => panic!("fixed historical signed stage"),
    };
    let parent = f
        .state
        .trades
        .update(
            &f.record.request_id,
            parent_status,
            Some(signed.signature.clone()),
            None,
        )
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    let pool = f.state.persistence.pool_for_tests();
    // Public-test historical import only: no ExecutionOwner, signing permit or sender exists.
    sqlx::query("INSERT INTO ordinary_execution_claims(request_id,schema_version,operation,request_hash,created_at_ms,owner_nonce,submission_stage,sdk_prepared_json,unsigned_message,unsigned_sha256,signed_signature,signed_transaction,signed_sha256) VALUES(?,1,'ordinary-sdk-trade',?,42,?,?,?,?,?,?,?,?)")
        .bind(&f.record.request_id).bind(f.execution.reservation().request_hash())
        .bind(uuid::Uuid::new_v4().simple().to_string()).bind(stage)
        .bind(source.record.encode(&source.message).unwrap()).bind(&source.message).bind(sha(&source.message))
        .bind(&signed.signature).bind(&signed.bytes).bind(sha(&signed.bytes)).execute(&pool).await.unwrap();
    BuyCase {
        f,
        store: SqliteOrdinaryExecutionClaims::new(pool),
        signature: signed.signature,
    }
}

async fn snapshot(c: &BuyCase) -> (Vec<Option<String>>, serde_json::Value) {
    let row = sqlx::query("SELECT submission_stage,owner_nonce,sdk_prepared_json,hex(unsigned_message),unsigned_sha256,bound_quote_json,preflight_json,signed_signature,hex(signed_transaction),signed_sha256,submission_error,final_observation_json FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id).fetch_one(&c.store.pool).await.unwrap();
    (
        (0..12).map(|i| row.try_get(i).unwrap()).collect(),
        serde_json::to_value(
            c.f.state
                .persistence
                .get(&c.f.record.request_id)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn sdk_buy_evidence_history_captures_both_assets_in_four_signed_stages_without_an_execution_owner(
) {
    for native in [false, true] {
        for stage in ["signed", "broadcasting", "submitted", "unknown"] {
            let c = historical(native, stage).await;
            let before = snapshot(&c).await;
            let captured = c
                .store
                .capture_observation(&c.f.record.request_id)
                .await
                .unwrap();
            assert_eq!(captured.context.signature(), c.signature);
            assert_eq!(captured.context.operation(), ClaimOperation::SdkTrade);
            assert_eq!(
                captured.context.wallet_pubkey(),
                c.f.execution.reservation().wallet_pubkey()
            );
            c.store.read_observation(&captured.context).await.unwrap();
            assert!(c
                .store
                .pending_observations(None, i64::MAX, 64)
                .await
                .unwrap()
                .entries
                .is_empty());
            assert!(c.f.claims().claim(&c.f.execution).await.is_err());
            assert_eq!(snapshot(&c).await, before);
        }
    }
}

#[tokio::test]
async fn sdk_buy_evidence_history_rejects_changed_parent_hash_json_message_and_signature_without_mutation(
) {
    for field in [
        "wallet",
        "side",
        "request_hash",
        "request_json",
        "signature",
        "unsigned_digest",
        "signed_digest",
        "metadata",
    ] {
        let c = historical(false, "signed").await;
        let row = sqlx::query("SELECT request_json FROM trades WHERE request_id=?")
            .bind(&c.f.record.request_id)
            .fetch_one(&c.store.pool)
            .await
            .unwrap();
        let mut request: crate::models::TradeRequest =
            serde_json::from_str(row.get("request_json")).unwrap();
        request.amount_quote_units = Some(101);
        let (table, column, value) = match field {
            "wallet" => (
                "trades",
                "wallet_pubkey",
                crate::domain::trade::Address::from_bytes([91; 32]).to_string(),
            ),
            "side" => ("trades", "side", "sell".into()),
            "request_hash" => (
                "ordinary_execution_claims",
                "request_hash",
                "changed".into(),
            ),
            "request_json" => (
                "trades",
                "request_json",
                serde_json::to_string(&request).unwrap(),
            ),
            "signature" => (
                "ordinary_execution_claims",
                "signed_signature",
                "changed".into(),
            ),
            "unsigned_digest" => (
                "ordinary_execution_claims",
                "unsigned_sha256",
                "changed".into(),
            ),
            "signed_digest" => (
                "ordinary_execution_claims",
                "signed_sha256",
                "changed".into(),
            ),
            "metadata" => {
                let json: String = sqlx::query_scalar(
                    "SELECT sdk_prepared_json FROM ordinary_execution_claims WHERE request_id=?",
                )
                .bind(&c.f.record.request_id)
                .fetch_one(&c.store.pool)
                .await
                .unwrap();
                (
                    "ordinary_execution_claims",
                    "sdk_prepared_json",
                    format!(" {json}"),
                )
            }
            _ => unreachable!(),
        };
        sqlx::query(&format!("UPDATE {table} SET {column}=? WHERE request_id=?"))
            .bind(value)
            .bind(&c.f.record.request_id)
            .execute(&c.store.pool)
            .await
            .unwrap();
        let before = snapshot(&c).await;
        assert!(
            c.store
                .capture_observation(&c.f.record.request_id)
                .await
                .is_err(),
            "{field}"
        );
        assert_eq!(snapshot(&c).await, before, "{field}");
    }
}

#[tokio::test]
async fn sdk_buy_evidence_history_finality_keeps_original_nonce_and_signed_bytes_without_enabling_a_claim(
) {
    for error in [None, Some("synthetic finalized failure")] {
        let c = historical(true, "unknown").await;
        let before = snapshot(&c).await;
        let captured = c
            .store
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        let result = c
            .store
            .record_captured_finality(
                &captured.context,
                &ChainObservation {
                    signature: c.signature.clone(),
                    slot: 900,
                    commitment: ChainCommitment::Finalized,
                    error: error.map(str::to_owned),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            result.status,
            if error.is_some() {
                TradeStatus::Failed
            } else {
                TradeStatus::Confirmed
            }
        );
        assert_eq!(result.signature.as_deref(), Some(c.signature.as_str()));
        let after = snapshot(&c).await;
        assert_eq!(&after.0[..11], &before.0[..11]);
        assert!(after.0[11].is_some());
        assert!(c.f.claims().claim(&c.f.execution).await.is_err());
        assert_eq!(snapshot(&c).await, after);
    }
}

#[tokio::test]
async fn sdk_buy_evidence_history_context_rejects_rewritten_record_and_nonfinal_or_foreign_observations(
) {
    let c = historical(false, "submitted").await;
    let captured = c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    let before = snapshot(&c).await;
    for (signature, slot, commitment) in [
        (c.signature.clone(), 900, ChainCommitment::Confirmed),
        ("foreign".into(), 900, ChainCommitment::Finalized),
        (c.signature.clone(), 0, ChainCommitment::Finalized),
    ] {
        assert!(c
            .store
            .record_captured_finality(
                &captured.context,
                &ChainObservation {
                    signature,
                    slot,
                    commitment,
                    error: None
                }
            )
            .await
            .is_err());
        assert_eq!(snapshot(&c).await, before);
    }
    sqlx::query("UPDATE ordinary_execution_claims SET owner_nonce=? WHERE request_id=?")
        .bind(uuid::Uuid::new_v4().simple().to_string())
        .bind(&c.f.record.request_id)
        .execute(&c.store.pool)
        .await
        .unwrap();
    let changed = snapshot(&c).await;
    assert!(c.store.read_observation(&captured.context).await.is_err());
    assert_eq!(snapshot(&c).await, changed);
}
