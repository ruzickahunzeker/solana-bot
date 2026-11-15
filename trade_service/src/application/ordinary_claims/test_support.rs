use crate::{
    adapters::persistence::{
        ordinary_claims::RuntimeExecutionClaims, trade_admission::AdmissionPayloadV1,
    },
    application::{
        trade_admission::{self, AdmissionDecision, AdmittedTrade},
        trade_record::TradeRecordV1,
    },
    config::{TradePersistenceConfig, TradePersistenceMode},
    domain::trade::{Address, USDC_MINT},
    models::TradeRequest,
    persistence::TradePersistence,
    AppState,
};
use std::sync::Arc;

pub(crate) struct Fixture {
    pub(crate) state: AppState,
    pub(crate) execution: Arc<AdmittedTrade>,
    pub(crate) record: TradeRecordV1,
}

impl Fixture {
    pub(crate) fn claims(&self) -> RuntimeExecutionClaims {
        RuntimeExecutionClaims::new(
            self.state.persistence.clone(),
            self.state.trades.clone(),
            self.state.idempotency.clone(),
        )
    }
}

pub(crate) async fn fixture(mode: TradePersistenceMode) -> Fixture {
    let mut state = crate::http_contract_tests::state().await;
    if mode != TradePersistenceMode::Memory {
        state.persistence = Arc::new(
            TradePersistence::new(&TradePersistenceConfig {
                mode,
                database_url: "sqlite::memory:".into(),
                busy_timeout_ms: 100,
            })
            .await
            .unwrap(),
        );
    }
    admit(state).await
}

pub(crate) async fn admit(state: AppState) -> Fixture {
    let request: TradeRequest = serde_json::from_value(serde_json::json!({
        "wallet_id":"fixture-wallet", "side":"buy", "pool":"pump-amm",
        "mint":Address::from_bytes([2; 32]).to_string(), "quote_mint": USDC_MINT,
        "amount_sol":0.001, "simulate":false, "idempotency_key":" claim-literal-key "
    }))
    .unwrap();
    let fixture = admit_request(state, request).await;
    assert!(fixture.execution.command().aggregator_sol);
    fixture
}

pub(crate) async fn admit_request(state: AppState, request: TradeRequest) -> Fixture {
    admit_request_for_pubkey(state, request, Address::from_bytes([1; 32]).to_string()).await
}

pub(crate) async fn admit_request_for_pubkey(
    state: AppState,
    request: TradeRequest,
    pubkey: String,
) -> Fixture {
    let normalized = crate::validation::validate_trade(&request, &state.config).unwrap();
    let command = request
        .to_command(
            "fixture-wallet".into(),
            &normalized,
            state.config.trade_command_defaults(),
        )
        .unwrap();
    let payload =
        AdmissionPayloadV1::new("fixture-wallet", &pubkey, &request, &normalized, &command);
    let decision = trade_admission::accept(
        state.persistence.as_ref(),
        state.trades.as_ref(),
        state.idempotency.as_ref(),
        &payload,
        crate::state::new_request_id(),
    )
    .await
    .unwrap();
    let AdmissionDecision::Created {
        record, execution, ..
    } = decision
    else {
        panic!("new actual reservation required")
    };
    Fixture {
        state,
        record,
        execution: Arc::new(execution),
    }
}
