use crate::{
    adapters::{
        persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
        sdk::damm_sell_message::candidate_fixture_for_wallet,
    },
    application::{
        ordinary_claims::test_support::admit_request_for_pubkey,
        ports::ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
    },
    domain::{submission::SignedSubmission, trade::TradeStatus},
    persistence::TradePersistence,
};
use serde_json::json;
use solana_sdk::{
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use std::sync::Arc;

pub(crate) struct SignedObservationFixture {
    pub(crate) request_id: String,
    pub(crate) signature: String,
}

/// Returns public observation identity only; the temporary signer/owner/candidate are destroyed.
pub(crate) async fn signed_observation(
    persistence: Arc<TradePersistence>,
    stage: &str,
    created_at_ms: i64,
) -> SignedObservationFixture {
    assert!(["signed", "broadcasting", "submitted", "unknown"].contains(&stage));
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = persistence;
    let key = Keypair::new();
    let candidate = candidate_fixture_for_wallet(key.pubkey());
    let d = candidate.metadata().details();
    let address = |bytes| solana_sdk::pubkey::Pubkey::new_from_array(bytes).to_string();
    let request = serde_json::from_value(json!({
        "wallet_id":"fixture-wallet", "side":"sell", "pool":"meteora-damm-v2",
        "mint":address(d.mint), "quote_mint":address(d.quote_mint), "pool_id":address(d.pool.pool),
        "sell_percent":d.sell_percent, "slippage_bps":d.slippage_bps,
        "tip_sol":f64::from_bits(d.tip_sol_bits), "use_mev":d.use_mev, "mev_provider":d.requested_mev_provider,
        "simulate":false, "execution_method":"sdk", "idempotency_key":format!(" scan-key-{} ",uuid::Uuid::new_v4())
    })).unwrap();
    let f = admit_request_for_pubkey(state, request, key.pubkey().to_string()).await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("a new original temporary SDK owner is required")
    };
    let parent = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    let pool = f.state.persistence.pool_for_tests();
    let store = SqliteOrdinaryExecutionClaims::new(pool.clone());
    let message = candidate.message_bytes();
    store
        .prepare_sdk_candidate(&f.execution, &owner, candidate.metadata(), &message)
        .await
        .unwrap();
    store
        .begin_sdk_signing(&f.execution, &owner, candidate.metadata(), &message)
        .await
        .unwrap();
    let transaction = VersionedTransaction::try_new(
        bincode::deserialize::<solana_sdk::message::VersionedMessage>(&message).unwrap(),
        &[&key],
    )
    .unwrap();
    let signed = SignedSubmission {
        signature: transaction.signatures[0].to_string(),
        bytes: bincode::serialize(&transaction).unwrap(),
    };
    store
        .record_sdk_signed(&f.execution, &owner, &signed)
        .await
        .unwrap();
    if stage != "signed" {
        store
            .take_sdk_broadcast(&f.execution, &owner, &signed)
            .await
            .unwrap();
        if stage != "broadcasting" {
            store
                .record_sdk_broadcast(
                    &f.execution,
                    &owner,
                    &signed,
                    stage == "submitted",
                    (stage == "unknown").then_some("synthetic transport uncertainty"),
                )
                .await
                .unwrap();
        }
    }
    sqlx::query("UPDATE ordinary_execution_claims SET created_at_ms=? WHERE request_id=?")
        .bind(created_at_ms)
        .bind(&f.record.request_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE trades SET updated_at_ms=? WHERE request_id=?")
        .bind(created_at_ms)
        .bind(&f.record.request_id)
        .execute(&pool)
        .await
        .unwrap();
    SignedObservationFixture {
        request_id: f.record.request_id,
        signature: signed.signature,
    }
}
