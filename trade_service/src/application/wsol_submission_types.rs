use crate::domain::wsol_progress::WsolState;
use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsolDirection {
    Wrap,
    Unwrap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WsolQuoteSnapshot {
    pub quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub owner_type: String,
    pub owner_id: String,
    pub direction: WsolDirection,
    pub amount_lamports: u64,
    pub source_balance_lamports: u64,
    pub ata_exists: bool,
    pub estimated_network_fee_lamports: u64,
    pub temporary_rent_lamports: u64,
    pub rent_refund_lamports: u64,
    pub expires_at_ms: u64,
}

impl WsolQuoteSnapshot {
    pub(crate) fn validate_binding(
        &self,
        wallet_id: &str,
        wallet_pubkey: &str,
        owner_type: &str,
        owner_id: &str,
        now_ms: u64,
    ) -> Result<()> {
        if now_ms >= self.expires_at_ms {
            bail!("conversion quote expired");
        }
        if self.wallet_id != wallet_id
            || self.wallet_pubkey != wallet_pubkey
            || self.owner_type != owner_type
            || self.owner_id != owner_id
        {
            bail!("conversion quote does not belong to this wallet and owner");
        }
        if self.amount_lamports == 0 || self.amount_lamports > self.source_balance_lamports {
            bail!("conversion quote amount is invalid for source balance");
        }
        Ok(())
    }
    pub(crate) fn validate_current_balance(&self, current: u64) -> Result<()> {
        if current != self.source_balance_lamports {
            bail!("conversion source balance changed; request a new quote");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WsolSubmissionInput {
    pub quote_id: String,
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub simulate: bool,
    pub confirmed: bool,
    pub idempotency_key: String,
}

impl WsolSubmissionInput {
    pub(crate) fn validate_quote(&self, quote: &WsolQuoteSnapshot) -> Result<()> {
        if self.idempotency_key.trim().is_empty() || self.idempotency_key.len() > 128 {
            bail!("idempotency_key must be 1..128 characters");
        }
        if self.quote_id != quote.quote_id
            || self.wallet_id != quote.wallet_id
            || self.owner_type != quote.owner_type
            || self.owner_id != quote.owner_id
        {
            bail!("submission does not match conversion quote");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsolSubmissionRecord {
    pub submission_id: String,
    pub idempotency_key: String,
    pub fingerprint: String,
    pub quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub owner_type: String,
    pub owner_id: String,
    pub status: String,
    pub signature: Option<String>,
    pub signed_transaction: Option<Vec<u8>>,
    pub error: Option<String>,
}

impl WsolSubmissionRecord {
    pub(crate) fn validate_status_record(&self, requested_id: &str) -> Result<WsolState> {
        if self.submission_id != requested_id
            || [
                &self.submission_id,
                &self.fingerprint,
                &self.quote_id,
                &self.wallet_id,
                &self.wallet_pubkey,
                &self.owner_type,
                &self.owner_id,
            ]
            .iter()
            .any(|value| value.is_empty())
            || self.idempotency_key.trim().is_empty()
            || self.idempotency_key.len() > 128
        {
            bail!("WSOL status record does not match the requested operation identity");
        }
        self.validate_persisted_state()?;
        WsolState::parse(&self.status)
    }

    pub(crate) fn validate_status_readback(&self, current: &Self) -> Result<()> {
        self.validate_readback(current)?;
        let before = self.validate_status_record(&self.submission_id)?;
        let after = current.validate_status_record(&self.submission_id)?;
        crate::domain::wsol_progress::validate_read_progress(before, after)?;
        if matches!(before, WsolState::Confirmed | WsolState::Failed)
            && (self.signature != current.signature
                || self.signed_transaction != current.signed_transaction
                || self.error != current.error)
        {
            bail!("WSOL terminal status evidence or error changed during readback");
        }
        Ok(())
    }

    pub(crate) fn validate_request(
        &self,
        input: &WsolSubmissionInput,
        quote: &WsolQuoteSnapshot,
    ) -> Result<()> {
        input.validate_quote(quote)?;
        if self.submission_id.is_empty()
            || self.fingerprint.is_empty()
            || self.idempotency_key != input.idempotency_key
            || self.quote_id != quote.quote_id
            || self.wallet_id != quote.wallet_id
            || self.wallet_pubkey != quote.wallet_pubkey
            || self.owner_type != quote.owner_type
            || self.owner_id != quote.owner_id
        {
            bail!("WSOL reservation does not match the original request and quote");
        }
        self.validate_persisted_state()
    }

    fn clean_unsigned(&self) -> bool {
        self.status == "reserved"
            && self.signature.is_none()
            && self.signed_transaction.is_none()
            && self.error.is_none()
    }

    pub(crate) fn validate_unsigned_reservation(&self, current: &Self) -> Result<()> {
        if !self.same_identity(current) || !self.clean_unsigned() || !current.clean_unsigned() {
            bail!("WSOL operation requires the original clean unsigned reservation");
        }
        Ok(())
    }

    pub(super) fn validate_persisted_state(&self) -> Result<()> {
        match WsolState::parse(&self.status)? {
            WsolState::Reserved => self.validate_unsigned_reservation(self),
            WsolState::Failed if self.signature.is_none() && self.signed_transaction.is_none() => {
                Ok(())
            }
            WsolState::Signed
            | WsolState::Unknown
            | WsolState::Submitted
            | WsolState::Confirmed
            | WsolState::Failed => WsolSignedOperation::from_record(self)?.validate(self),
        }
    }

    fn same_identity(&self, current: &Self) -> bool {
        self.submission_id == current.submission_id
            && self.idempotency_key == current.idempotency_key
            && self.fingerprint == current.fingerprint
            && self.quote_id == current.quote_id
            && self.wallet_id == current.wallet_id
            && self.wallet_pubkey == current.wallet_pubkey
            && self.owner_type == current.owner_type
            && self.owner_id == current.owner_id
    }

    pub(crate) fn validate_readback(&self, current: &Self) -> Result<()> {
        if !self.same_identity(current) {
            bail!("WSOL submission identity changed during readback");
        }
        // A reservation may have advanced concurrently, but existing evidence cannot change.
        if self.signature.is_some() || self.signed_transaction.is_some() {
            WsolSignedOperation::from_record(self)?.validate(current)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct WsolSignedOperation<'a> {
    pub record: &'a WsolSubmissionRecord,
    pub signature: &'a str,
    pub signed_bytes: &'a [u8],
}
impl<'a> WsolSignedOperation<'a> {
    pub(crate) fn validate_reservation(&self, current: &WsolSubmissionRecord) -> Result<()> {
        if self.signature.is_empty()
            || self.signed_bytes.is_empty()
            || !self.record.same_identity(current)
            || self.record.status != "reserved"
            || current.status != "reserved"
            || self.record.signature.is_some()
            || self.record.signed_transaction.is_some()
            || current.signature.is_some()
            || current.signed_transaction.is_some()
            || self.record.error.is_some()
            || current.error.is_some()
        {
            bail!("WSOL signed evidence requires the original unsigned reservation");
        }
        Ok(())
    }

    pub(crate) fn from_record(record: &'a WsolSubmissionRecord) -> Result<Self> {
        Ok(Self {
            record,
            signature: record.signature.as_deref().ok_or_else(|| {
                anyhow::anyhow!("WSOL outcome requires persisted signed evidence")
            })?,
            signed_bytes: record.signed_transaction.as_deref().ok_or_else(|| {
                anyhow::anyhow!("WSOL outcome requires persisted signed evidence")
            })?,
        })
    }
    pub(crate) fn validate(&self, current: &WsolSubmissionRecord) -> Result<()> {
        let expected = self.record;
        if self.signature.is_empty()
            || self.signed_bytes.is_empty()
            || !expected.same_identity(current)
            || current.signature.as_deref() != Some(self.signature)
            || current.signed_transaction.as_deref() != Some(self.signed_bytes)
            || expected
                .signature
                .as_deref()
                .is_some_and(|value| value != self.signature)
            || expected
                .signed_transaction
                .as_deref()
                .is_some_and(|value| value != self.signed_bytes)
        {
            bail!("WSOL signed operation identity or evidence mismatch");
        }
        Ok(())
    }
}

pub(crate) struct WsolSubmissionOutcome {
    pub record: WsolSubmissionRecord,
    pub duplicate: bool,
}

pub(crate) struct WsolSimulationOutcome {
    pub quote_id: String,
    pub network_fee_lamports: u64,
    pub units_consumed: Option<u64>,
}
