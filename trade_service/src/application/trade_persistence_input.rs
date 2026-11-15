//! Immutable legacy storage bytes, not wire DTOs, routes or execution authority.

pub(crate) struct StoredRequestV1 {
    request_hash: String,
    request_json: String,
}

impl StoredRequestV1 {
    pub(crate) fn new(request_hash: String, request_json: String) -> Self {
        Self {
            request_hash,
            request_json,
        }
    }
    pub(crate) fn hash(&self) -> &str {
        &self.request_hash
    }
    pub(crate) fn json(&self) -> &str {
        &self.request_json
    }
    pub(crate) fn into_parts(self) -> (String, String) {
        (self.request_hash, self.request_json)
    }
}

pub(crate) struct ValidatedTradeInput {
    wallet_id: String,
    idempotency_key: String,
    aggregator_sol: bool,
    snapshot: StoredRequestV1,
}

impl ValidatedTradeInput {
    pub(crate) fn new(
        wallet_id: String,
        idempotency_key: String,
        aggregator_sol: bool,
        snapshot: StoredRequestV1,
    ) -> Self {
        Self {
            wallet_id,
            idempotency_key,
            aggregator_sol,
            snapshot,
        }
    }
    pub(crate) fn ensure_wallet(&self, wallet_id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.wallet_id == wallet_id,
            "trade snapshot wallet does not match record"
        );
        Ok(())
    }
    pub(crate) fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
    pub(crate) fn aggregator_sol(&self) -> bool {
        self.aggregator_sol
    }
    pub(crate) fn snapshot(&self) -> &StoredRequestV1 {
        &self.snapshot
    }
    pub(crate) fn into_storage(self) -> (String, StoredRequestV1) {
        (self.idempotency_key, self.snapshot)
    }
}

pub(crate) struct RejectedTradeInput {
    wallet_id: String,
    request_id: String,
    snapshot: StoredRequestV1,
}

impl RejectedTradeInput {
    pub(crate) fn new(wallet_id: String, request_id: String, snapshot: StoredRequestV1) -> Self {
        Self {
            wallet_id,
            request_id,
            snapshot,
        }
    }
    pub(crate) fn ensure_record(&self, wallet_id: &str, request_id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.wallet_id == wallet_id && self.request_id == request_id,
            "rejected trade snapshot does not match record"
        );
        Ok(())
    }
    pub(crate) fn into_storage(self) -> (String, StoredRequestV1) {
        (format!("__rejected__:{}", self.request_id), self.snapshot)
    }
}
