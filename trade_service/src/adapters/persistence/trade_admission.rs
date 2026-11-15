use crate::{
    adapters::persistence::trade_snapshot,
    application::{
        ports::trade_admission::{RejectedAdmissionPayload, ValidatedAdmissionPayload},
        trade_command::TradeCommand,
        trade_persistence_input::{RejectedTradeInput, ValidatedTradeInput},
        trade_record::TradeRecordV1,
        validated_trade::ValidatedTrade as NormalizedTrade,
    },
    domain::trade::{ExecutionMethod, TradeStatus},
    models::TradeRequest,
};

pub(crate) struct AdmissionPayloadV1<'a> {
    wallet_id: &'a str,
    wallet_pubkey: &'a str,
    request: &'a TradeRequest,
    normalized: &'a NormalizedTrade,
    command: &'a TradeCommand,
}

impl<'a> AdmissionPayloadV1<'a> {
    pub(crate) fn new(
        wallet_id: &'a str,
        wallet_pubkey: &'a str,
        request: &'a TradeRequest,
        normalized: &'a NormalizedTrade,
        command: &'a TradeCommand,
    ) -> Self {
        Self {
            wallet_id,
            wallet_pubkey,
            request,
            normalized,
            command,
        }
    }
}

impl ValidatedAdmissionPayload for AdmissionPayloadV1<'_> {
    fn wallet_id(&self) -> &str {
        self.wallet_id
    }
    fn idempotency_key(&self) -> &str {
        &self.request.idempotency_key
    }
    fn aggregator_sol(&self) -> bool {
        self.normalized.aggregator_sol
    }
    fn fingerprint(&self) -> anyhow::Result<String> {
        trade_snapshot::request_hash(self.wallet_id, self.request, self.normalized)
    }
    fn new_record(&self, request_id: String) -> TradeRecordV1 {
        new_record(request_id, self.wallet_id, self.wallet_pubkey, self.request)
    }
    fn snapshot(&self) -> anyhow::Result<ValidatedTradeInput> {
        trade_snapshot::validated(self.wallet_id, self.request, self.normalized)
    }
    fn execution_command(&self) -> TradeCommand {
        self.command.clone()
    }
}

pub(crate) struct RejectionPayloadV1<'a> {
    wallet_id: &'a str,
    wallet_pubkey: &'a str,
    request: &'a TradeRequest,
}

impl<'a> RejectionPayloadV1<'a> {
    pub(crate) fn new(
        wallet_id: &'a str,
        wallet_pubkey: &'a str,
        request: &'a TradeRequest,
    ) -> Self {
        Self {
            wallet_id,
            wallet_pubkey,
            request,
        }
    }
}

impl RejectedAdmissionPayload for RejectionPayloadV1<'_> {
    fn new_record(&self, request_id: String) -> TradeRecordV1 {
        new_record(request_id, self.wallet_id, self.wallet_pubkey, self.request)
    }

    fn snapshot(&self, record: &TradeRecordV1) -> anyhow::Result<RejectedTradeInput> {
        // Preserve the old codec's binding to the resolved record, not raw DTO wallet metadata.
        trade_snapshot::rejected(&record.wallet_id, &record.request_id, self.request)
    }
}

fn new_record(
    request_id: String,
    wallet_id: &str,
    wallet_pubkey: &str,
    request: &TradeRequest,
) -> TradeRecordV1 {
    let now = crate::state::now_ms();
    TradeRecordV1 {
        request_id,
        wallet_id: wallet_id.into(),
        wallet_pubkey: wallet_pubkey.into(),
        side: request.side,
        pool: request.pool,
        mint: request.mint.clone(),
        pool_id: request.pool_id.clone(),
        execution_method: request.execution_method.unwrap_or(ExecutionMethod::Sdk),
        status: TradeStatus::Accepted,
        signature: None,
        error: None,
        duplicate: false,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejected_v1_factory_preserves_all_legacy_error_records_and_snapshot_bytes() {
        let state = crate::http_contract_tests::state().await;
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/contracts/trade_normalization_v1.json"
        ))
        .unwrap();
        let old_store = crate::state::TradeStore::default();
        let new_store = crate::state::TradeStore::default();
        for case in fixtures["rejected"].as_array().unwrap() {
            let mut value = serde_json::json!({"side":"buy","pool":"pump","mint":"11111111111111111111111111111111","idempotency_key":" literal-key ","wallet_id":"unresolved-request-metadata"});
            value
                .as_object_mut()
                .unwrap()
                .extend(case["patch"].as_object().unwrap().clone());
            let request: TradeRequest = serde_json::from_value(value).unwrap();
            let error = crate::validation::validate_trade(&request, &state.config)
                .err()
                .unwrap()
                .to_string();
            let legacy = old_store.rejected(
                &request,
                "resolved-wallet".into(),
                "public-identity".into(),
                error.clone(),
            );
            let payload = RejectionPayloadV1::new("resolved-wallet", "public-identity", &request);
            let actual = crate::application::trade_admission::reject(
                &new_store,
                &payload,
                legacy.request_id.clone(),
                error,
                crate::state::now_ms,
            );
            assert!(actual.updated_at_ms >= actual.created_at_ms);
            let mut expected = serde_json::to_value(&legacy).unwrap();
            let mut observed = serde_json::to_value(&actual).unwrap();
            for key in ["created_at_ms", "updated_at_ms"] {
                expected.as_object_mut().unwrap().remove(key);
                observed.as_object_mut().unwrap().remove(key);
            }
            assert_eq!(observed, expected, "{}", case["name"]);
            assert_eq!(
                new_store.get(&actual.request_id).unwrap().status,
                TradeStatus::Rejected
            );
            let (actual_key, actual_snapshot) = payload.snapshot(&actual).unwrap().into_storage();
            let (legacy_key, legacy_snapshot) =
                trade_snapshot::rejected(&legacy.wallet_id, &legacy.request_id, &request)
                    .unwrap()
                    .into_storage();
            assert_eq!(actual_key, legacy_key);
            assert_eq!(actual_snapshot.hash(), legacy_snapshot.hash());
            assert_eq!(actual_snapshot.json(), legacy_snapshot.json());
            assert_eq!(
                actual_snapshot.json(),
                serde_json::to_string(&request).unwrap()
            );
        }
        assert_eq!(fixtures["rejected"].as_array().unwrap().len(), 19);
    }

    #[tokio::test]
    async fn lazy_v1_factory_preserves_all_normalized_sample_records_hashes_and_snapshots() {
        let state = crate::http_contract_tests::state().await;
        let mut config = (*state.config).clone();
        config.stonkfun_via_sol.enabled = true;
        config.default_tip_sol = 0.0001;
        config.pumpapi.default_priority_fee_sol = 0.0002;
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/contracts/trade_normalization_v1.json"
        ))
        .unwrap();
        for case in fixtures["accepted"].as_array().unwrap() {
            let mut value = serde_json::json!({"side":"buy","pool":"pump","mint":"11111111111111111111111111111111","idempotency_key":" literal-key "});
            value
                .as_object_mut()
                .unwrap()
                .extend(case["patch"].as_object().unwrap().clone());
            let request: TradeRequest = serde_json::from_value(value).unwrap();
            let normalized = crate::validation::validate_trade(&request, &config).unwrap();
            let command = request
                .to_command(
                    "resolved-wallet".into(),
                    &normalized,
                    config.trade_command_defaults(),
                )
                .unwrap();
            let payload = AdmissionPayloadV1::new(
                "resolved-wallet",
                "public-identity",
                &request,
                &normalized,
                &command,
            );
            let original = crate::state::TradeStore::default().create_with_id(
                "tr_factory".into(),
                &request,
                "resolved-wallet".into(),
                "public-identity".into(),
            );
            let actual = payload.new_record("tr_factory".into());
            assert_eq!(actual.created_at_ms, actual.updated_at_ms);
            let mut expected = serde_json::to_value(original).unwrap();
            let mut observed = serde_json::to_value(actual).unwrap();
            for key in ["created_at_ms", "updated_at_ms"] {
                expected.as_object_mut().unwrap().remove(key);
                observed.as_object_mut().unwrap().remove(key);
            }
            assert_eq!(observed, expected, "{}", case["name"]);
            let snapshot = payload.snapshot().unwrap();
            assert_eq!(payload.fingerprint().unwrap(), snapshot.snapshot().hash());
            assert_eq!(
                snapshot.snapshot().json(),
                serde_json::to_string(&request).unwrap()
            );
            assert_eq!(payload.aggregator_sol(), normalized.aggregator_sol);
            assert_eq!(payload.idempotency_key(), request.idempotency_key);
        }
    }
}
