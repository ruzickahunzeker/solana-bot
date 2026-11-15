use super::wsol_submission_types::WsolSubmissionRecord;
use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsolFailureKind {
    Rejected,
    Conflict,
    Unavailable,
    Integrity,
    OutcomeUnknown,
}

impl WsolFailureKind {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Rejected => "wsol_rejected",
            Self::Conflict => "wsol_conflict",
            Self::Unavailable => "wsol_unavailable",
            Self::Integrity => "wsol_integrity_error",
            Self::OutcomeUnknown => "wsol_outcome_unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsolFailureStage {
    Unclassified,
    Admission,
    QuoteLookup,
    Reservation,
    UnsignedFailure,
    SignedPersistence,
    SimulationReceipt,
    PreBroadcastRead,
    BroadcastReceipt,
    FinalRead,
    StatusRead,
    StatusLookup,
    StatusObservation,
    StatusFinalRead,
    QuoteStore,
    QuoteMetadata,
    QuoteEstimate,
    QuoteSave,
    UnsignedAdmission,
    UnsignedQuote,
    UnsignedInput,
    UnsignedMetadata,
    UnsignedWallet,
    UnsignedPrepare,
    UnsignedRpc,
}

impl WsolFailureStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unclassified => "unclassified",
            Self::Admission => "admission",
            Self::QuoteLookup => "quote_lookup",
            Self::Reservation => "reservation",
            Self::UnsignedFailure => "unsigned_failure",
            Self::SignedPersistence => "signed_persistence",
            Self::SimulationReceipt => "simulation_receipt",
            Self::PreBroadcastRead => "pre_broadcast_read",
            Self::BroadcastReceipt => "broadcast_receipt",
            Self::FinalRead => "final_read",
            Self::StatusRead => "status_read",
            Self::StatusLookup => "status_lookup",
            Self::StatusObservation => "status_observation",
            Self::StatusFinalRead => "status_final_read",
            Self::QuoteStore => "quote_store",
            Self::QuoteMetadata => "quote_metadata",
            Self::QuoteEstimate => "quote_estimate",
            Self::QuoteSave => "quote_save",
            Self::UnsignedAdmission => "unsigned_admission",
            Self::UnsignedQuote => "unsigned_quote",
            Self::UnsignedInput => "unsigned_input",
            Self::UnsignedMetadata => "unsigned_metadata",
            Self::UnsignedWallet => "unsigned_wallet",
            Self::UnsignedPrepare => "unsigned_prepare",
            Self::UnsignedRpc => "unsigned_rpc",
        }
    }
}

#[derive(Debug)]
pub(crate) struct WsolFailure {
    pub kind: WsolFailureKind,
    pub stage: WsolFailureStage,
    pub submission_id: Option<String>,
    pub signature: Option<String>,
    message: String,
}

impl WsolFailure {
    fn marker(kind: WsolFailureKind, error: impl fmt::Display) -> Self {
        Self {
            kind,
            stage: WsolFailureStage::Admission,
            submission_id: None,
            signature: None,
            message: crate::sanitize::redact_urls(&error.to_string()),
        }
    }
    pub(crate) fn rejected(error: impl fmt::Display) -> Self {
        Self::marker(WsolFailureKind::Rejected, error)
    }
    pub(crate) fn conflict(error: impl fmt::Display) -> Self {
        Self::marker(WsolFailureKind::Conflict, error)
    }
    pub(crate) fn integrity(error: impl fmt::Display) -> Self {
        Self::marker(WsolFailureKind::Integrity, error)
    }
    pub(crate) fn unclassified(error: anyhow::Error) -> Self {
        Self {
            kind: WsolFailureKind::OutcomeUnknown,
            stage: WsolFailureStage::Unclassified,
            submission_id: None,
            signature: None,
            message: crate::sanitize::redact_urls(&error.to_string()),
        }
    }

    pub(crate) fn client_message(&self) -> String {
        if matches!(
            self.kind,
            WsolFailureKind::Unavailable
                | WsolFailureKind::Integrity
                | WsolFailureKind::OutcomeUnknown
        ) {
            format!("{}; WSOL operation requires reconciliation; do not automatically retry or use a new idempotency_key", self.message)
        } else {
            self.message.clone()
        }
    }
}
impl fmt::Display for WsolFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl Error for WsolFailure {}

pub(crate) struct WsolFailureScope {
    stage: WsolFailureStage,
    submission_id: Option<String>,
    signature: Option<String>,
}
impl WsolFailureScope {
    pub(crate) fn new() -> Self {
        Self {
            stage: WsolFailureStage::Admission,
            submission_id: None,
            signature: None,
        }
    }
    pub(crate) fn stage(&mut self, stage: WsolFailureStage) {
        self.stage = stage;
    }
    pub(crate) fn remember_record(&mut self, record: &WsolSubmissionRecord) {
        self.submission_id = Some(record.submission_id.clone());
    }
    pub(crate) fn remember_signature(&mut self, signature: &str) {
        if !signature.is_empty() {
            self.signature = Some(signature.into());
        }
    }
    pub(crate) fn classify(&self, error: anyhow::Error) -> anyhow::Error {
        let tagged = error
            .downcast_ref::<WsolFailure>()
            .map(|failure| failure.kind);
        let kind = match (self.stage, tagged) {
            (_, Some(WsolFailureKind::Integrity)) => WsolFailureKind::Integrity,
            (
                WsolFailureStage::QuoteMetadata
                | WsolFailureStage::QuoteEstimate
                | WsolFailureStage::UnsignedAdmission
                | WsolFailureStage::UnsignedQuote
                | WsolFailureStage::UnsignedInput
                | WsolFailureStage::UnsignedMetadata
                | WsolFailureStage::UnsignedWallet
                | WsolFailureStage::UnsignedPrepare
                | WsolFailureStage::UnsignedRpc,
                Some(WsolFailureKind::Rejected),
            ) => WsolFailureKind::Rejected,
            (
                WsolFailureStage::QuoteStore
                | WsolFailureStage::QuoteMetadata
                | WsolFailureStage::QuoteEstimate
                | WsolFailureStage::UnsignedAdmission
                | WsolFailureStage::UnsignedQuote
                | WsolFailureStage::UnsignedInput
                | WsolFailureStage::UnsignedMetadata
                | WsolFailureStage::UnsignedWallet
                | WsolFailureStage::UnsignedPrepare
                | WsolFailureStage::UnsignedRpc,
                _,
            ) => WsolFailureKind::Unavailable,
            (WsolFailureStage::Admission, _) => WsolFailureKind::Rejected,
            (WsolFailureStage::QuoteLookup, Some(WsolFailureKind::Rejected)) => {
                WsolFailureKind::Rejected
            }
            (WsolFailureStage::QuoteLookup, _) => WsolFailureKind::Unavailable,
            (WsolFailureStage::StatusRead, Some(WsolFailureKind::Rejected)) => {
                WsolFailureKind::Rejected
            }
            (
                WsolFailureStage::StatusRead
                | WsolFailureStage::StatusLookup
                | WsolFailureStage::StatusFinalRead,
                _,
            ) => WsolFailureKind::Unavailable,
            (
                WsolFailureStage::Reservation,
                Some(kind @ (WsolFailureKind::Rejected | WsolFailureKind::Conflict)),
            ) => kind,
            _ => WsolFailureKind::OutcomeUnknown,
        };
        WsolFailure {
            kind,
            stage: self.stage,
            submission_id: self.submission_id.clone(),
            signature: self.signature.clone(),
            message: crate::sanitize::redact_urls(&error.to_string()),
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn record() -> WsolSubmissionRecord {
        WsolSubmissionRecord {
            submission_id: "conversion-original".into(),
            idempotency_key: "must-not-appear-in-error-context".into(),
            fingerprint: "fingerprint".into(),
            quote_id: "quote".into(),
            wallet_id: "wallet".into(),
            wallet_pubkey: "public".into(),
            owner_type: "telegram".into(),
            owner_id: "private-owner".into(),
            status: "reserved".into(),
            signature: None,
            signed_transaction: None,
            error: None,
        }
    }

    #[test]
    fn wsol_failure_classification_uses_typed_rejections_not_error_text() {
        let mut scope = WsolFailureScope::new();
        scope.stage(WsolFailureStage::Reservation);
        let message = "idempotency key was used with different conversion parameters";
        let raw = scope.classify(anyhow!(message));
        assert_eq!(
            raw.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::OutcomeUnknown
        );
        let typed = scope.classify(
            anyhow::Error::new(WsolFailure::conflict(message)).context("storage admission"),
        );
        assert_eq!(
            typed.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Conflict
        );
        let rejected = scope.classify(WsolFailure::rejected("conversion quote expired").into());
        assert_eq!(
            rejected.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Rejected
        );
        for stage in [
            WsolFailureStage::UnsignedFailure,
            WsolFailureStage::SignedPersistence,
            WsolFailureStage::SimulationReceipt,
            WsolFailureStage::PreBroadcastRead,
            WsolFailureStage::BroadcastReceipt,
            WsolFailureStage::FinalRead,
        ] {
            scope.stage(stage);
            let late = scope.classify(WsolFailure::rejected(message).into());
            assert_eq!(
                late.downcast_ref::<WsolFailure>().unwrap().kind,
                WsolFailureKind::OutcomeUnknown,
                "{stage:?}"
            );
        }
    }

    #[test]
    fn wsol_failure_read_unavailability_and_integrity_never_become_parameter_errors() {
        let mut scope = WsolFailureScope::new();
        let admission = scope.classify(anyhow!("invalid input"));
        assert_eq!(
            admission.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Rejected
        );
        scope.stage(WsolFailureStage::QuoteLookup);
        let read = scope.classify(anyhow!("synthetic database read failure"));
        assert_eq!(
            read.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Unavailable
        );
        let missing = scope.classify(WsolFailure::rejected("conversion quote not found").into());
        assert_eq!(
            missing.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Rejected
        );
        let integrity = scope.classify(WsolFailure::integrity("mismatched receipt").into());
        assert_eq!(
            integrity.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Integrity
        );
    }

    #[test]
    fn wsol_failure_context_is_redacted_and_contains_only_remembered_public_identifiers() {
        let mut scope = WsolFailureScope::new();
        scope.stage(WsolFailureStage::SignedPersistence);
        let before = scope.classify(anyhow!(
            "synthetic failure https://fixture.invalid/private-api-key"
        ));
        let before = before.downcast_ref::<WsolFailure>().unwrap();
        assert!(before.submission_id.is_none() && before.signature.is_none());
        assert!(!format!("{before:?}").contains("private-api-key"));
        assert!(before.source().is_none());
        scope.remember_record(&record());
        scope.remember_signature("original-public-signature");
        let error = scope.classify(anyhow!("synthetic failure"));
        let error = error.downcast_ref::<WsolFailure>().unwrap();
        assert_eq!(error.submission_id.as_deref(), Some("conversion-original"));
        assert_eq!(
            error.signature.as_deref(),
            Some("original-public-signature")
        );
        assert!(!format!("{error:?}").contains("must-not-appear"));
        assert!(!format!("{error:?}").contains("private-owner"));
        assert!(error
            .client_message()
            .contains("do not automatically retry"));
        assert_eq!(error.to_string(), "synthetic failure");
        let fallback = WsolFailure::unclassified(anyhow!("synthetic failure"));
        assert_eq!(fallback.kind, WsolFailureKind::OutcomeUnknown);
        assert_eq!(fallback.stage, WsolFailureStage::Unclassified);
        assert!(fallback.submission_id.is_none() && fallback.signature.is_none());
    }

    #[test]
    fn wsol_failure_machine_codes_and_stages_have_stable_unique_values() {
        let kinds = [
            (WsolFailureKind::Rejected, "wsol_rejected"),
            (WsolFailureKind::Conflict, "wsol_conflict"),
            (WsolFailureKind::Unavailable, "wsol_unavailable"),
            (WsolFailureKind::Integrity, "wsol_integrity_error"),
            (WsolFailureKind::OutcomeUnknown, "wsol_outcome_unknown"),
        ];
        for (kind, code) in kinds {
            assert_eq!(kind.code(), code);
        }
        let stages = [
            (WsolFailureStage::Unclassified, "unclassified"),
            (WsolFailureStage::Admission, "admission"),
            (WsolFailureStage::QuoteLookup, "quote_lookup"),
            (WsolFailureStage::Reservation, "reservation"),
            (WsolFailureStage::UnsignedFailure, "unsigned_failure"),
            (WsolFailureStage::SignedPersistence, "signed_persistence"),
            (WsolFailureStage::SimulationReceipt, "simulation_receipt"),
            (WsolFailureStage::PreBroadcastRead, "pre_broadcast_read"),
            (WsolFailureStage::BroadcastReceipt, "broadcast_receipt"),
            (WsolFailureStage::FinalRead, "final_read"),
            (WsolFailureStage::StatusRead, "status_read"),
            (WsolFailureStage::StatusLookup, "status_lookup"),
            (WsolFailureStage::StatusObservation, "status_observation"),
            (WsolFailureStage::StatusFinalRead, "status_final_read"),
            (WsolFailureStage::QuoteStore, "quote_store"),
            (WsolFailureStage::QuoteMetadata, "quote_metadata"),
            (WsolFailureStage::QuoteEstimate, "quote_estimate"),
            (WsolFailureStage::QuoteSave, "quote_save"),
            (WsolFailureStage::UnsignedAdmission, "unsigned_admission"),
            (WsolFailureStage::UnsignedQuote, "unsigned_quote"),
            (WsolFailureStage::UnsignedInput, "unsigned_input"),
            (WsolFailureStage::UnsignedMetadata, "unsigned_metadata"),
            (WsolFailureStage::UnsignedWallet, "unsigned_wallet"),
            (WsolFailureStage::UnsignedPrepare, "unsigned_prepare"),
            (WsolFailureStage::UnsignedRpc, "unsigned_rpc"),
        ];
        for (stage, text) in stages {
            assert_eq!(stage.as_str(), text);
        }
    }

    #[test]
    fn wsol_status_boundary_errors_use_typed_rejection_and_never_downgrade_uncertainty() {
        use WsolFailureStage::*;
        for stage in [StatusRead, StatusLookup, StatusObservation, StatusFinalRead] {
            let mut scope = WsolFailureScope::new();
            scope.stage(stage);
            for tagged in [false, true] {
                let error = if tagged {
                    WsolFailure::rejected("conversion submission not found").into()
                } else {
                    anyhow!("conversion submission not found")
                };
                let classified = scope.classify(error);
                let failure = classified.downcast_ref::<WsolFailure>().unwrap();
                assert_eq!(
                    failure.kind,
                    if stage == StatusRead && tagged {
                        WsolFailureKind::Rejected
                    } else if stage == StatusObservation {
                        WsolFailureKind::OutcomeUnknown
                    } else {
                        WsolFailureKind::Unavailable
                    }
                );
                assert_eq!(failure.stage, stage);
                let integrity = scope.classify(WsolFailure::integrity("bad evidence").into());
                assert_eq!(
                    integrity.downcast_ref::<WsolFailure>().unwrap().kind,
                    WsolFailureKind::Integrity
                );
            }
        }
    }

    #[test]
    fn wsol_quote_unsigned_boundary_stage_classification_is_typed_and_save_ack_loss_never_downgrades(
    ) {
        use WsolFailureStage::*;
        for stage in [
            QuoteStore,
            QuoteMetadata,
            QuoteEstimate,
            QuoteSave,
            UnsignedAdmission,
            UnsignedQuote,
            UnsignedInput,
            UnsignedMetadata,
            UnsignedWallet,
            UnsignedPrepare,
            UnsignedRpc,
        ] {
            let mut scope = WsolFailureScope::new();
            scope.stage(stage);
            let raw = scope.classify(anyhow!("wallet not found"));
            let typed = scope.classify(WsolFailure::rejected("wallet not found").into());
            assert_eq!(
                raw.downcast_ref::<WsolFailure>().unwrap().kind,
                if stage == QuoteSave {
                    WsolFailureKind::OutcomeUnknown
                } else {
                    WsolFailureKind::Unavailable
                }
            );
            assert_eq!(
                typed.downcast_ref::<WsolFailure>().unwrap().kind,
                match stage {
                    QuoteSave => WsolFailureKind::OutcomeUnknown,
                    QuoteStore => WsolFailureKind::Unavailable,
                    _ => WsolFailureKind::Rejected,
                }
            );
            let integrity = scope.classify(WsolFailure::integrity("bad evidence").into());
            assert_eq!(
                integrity.downcast_ref::<WsolFailure>().unwrap().kind,
                WsolFailureKind::Integrity
            );
            assert!(raw
                .downcast_ref::<WsolFailure>()
                .unwrap()
                .submission_id
                .is_none());
            assert!(raw
                .downcast_ref::<WsolFailure>()
                .unwrap()
                .signature
                .is_none());
        }
    }
}
