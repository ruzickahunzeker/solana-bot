use anyhow::{ensure, Result};

use crate::application::{
    ordinary_claims::ClaimOperation, trade_admission::AdmittedTrade, trade_record::TradeRecordV1,
};

/// An immutable observation identity, never an execution owner or signing input.
pub(crate) struct OrdinaryObservationContext {
    operation: ClaimOperation,
    request_id: String,
    wallet_id: String,
    wallet_pubkey: String,
    idempotency_key: String,
    request_hash: String,
    signature: String,
    evidence_fingerprint: String,
}

impl std::fmt::Debug for OrdinaryObservationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OrdinaryObservationContext(<redacted>)")
    }
}

impl OrdinaryObservationContext {
    pub(crate) fn captured(
        record: &TradeRecordV1,
        idempotency_key: String,
        request_hash: String,
        evidence_fingerprint: String,
    ) -> Result<Self> {
        let signature = record.signature.clone().unwrap_or_default();
        ensure!(
            !record.request_id.is_empty()
                && !record.wallet_id.is_empty()
                && !record.wallet_pubkey.is_empty()
                && !idempotency_key.trim().is_empty()
                && !signature.is_empty()
                && [request_hash.as_str(), evidence_fingerprint.as_str()]
                    .iter()
                    .all(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())),
            "ordinary observation identity is incomplete"
        );
        Ok(Self {
            operation: ClaimOperation::SolBuy,
            request_id: record.request_id.clone(),
            wallet_id: record.wallet_id.clone(),
            wallet_pubkey: record.wallet_pubkey.clone(),
            idempotency_key,
            request_hash,
            signature,
            evidence_fingerprint,
        })
    }

    pub(crate) fn captured_sdk(
        record: &TradeRecordV1,
        idempotency_key: String,
        request_hash: String,
        evidence_fingerprint: String,
    ) -> Result<Self> {
        let mut value =
            Self::captured(record, idempotency_key, request_hash, evidence_fingerprint)?;
        value.operation = ClaimOperation::SdkTrade;
        Ok(value)
    }

    pub(crate) fn operation(&self) -> ClaimOperation {
        self.operation
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }
    pub(crate) fn wallet_id(&self) -> &str {
        &self.wallet_id
    }
    pub(crate) fn wallet_pubkey(&self) -> &str {
        &self.wallet_pubkey
    }
    pub(crate) fn signature(&self) -> &str {
        &self.signature
    }
    pub(crate) fn request_hash(&self) -> &str {
        &self.request_hash
    }

    pub(crate) fn ensure_record(&self, record: &TradeRecordV1) -> Result<()> {
        ensure!(
            record.request_id == self.request_id()
                && record.wallet_id == self.wallet_id()
                && record.wallet_pubkey == self.wallet_pubkey()
                && record.signature.as_deref() == Some(self.signature()),
            "ordinary observation record identity changed"
        );
        Ok(())
    }

    pub(crate) fn ensure_same(&self, other: &Self) -> Result<()> {
        ensure!(
            self.operation == other.operation
                && self.request_id == other.request_id
                && self.wallet_id == other.wallet_id
                && self.wallet_pubkey == other.wallet_pubkey
                && self.idempotency_key == other.idempotency_key
                && self.request_hash == other.request_hash
                && self.signature == other.signature
                && self.evidence_fingerprint == other.evidence_fingerprint,
            "ordinary observation immutable evidence changed"
        );
        Ok(())
    }

    pub(crate) fn ensure_execution(&self, execution: &AdmittedTrade) -> Result<()> {
        let anchor = execution.reservation();
        ensure!(
            self.request_id == anchor.request_id()
                && self.wallet_id == anchor.wallet_id()
                && self.wallet_pubkey == anchor.wallet_pubkey()
                && self.idempotency_key == anchor.idempotency_key()
                && self.request_hash == anchor.request_hash(),
            "ordinary observation does not match original reservation"
        );
        Ok(())
    }
}

pub(crate) struct CapturedOrdinaryObservation {
    pub(crate) context: OrdinaryObservationContext,
    pub(crate) record: TradeRecordV1,
}
