use crate::domain::trade::TradeStatus;
use crate::{
    application::validated_trade::ValidatedTrade,
    domain::{execution_policy::BroadcastPolicy, trade::ExecutionMethod},
};

pub(crate) fn describe_trade(
    side: crate::domain::trade::Side,
    amount_lamports: Option<u64>,
    sell_percent: Option<u64>,
) -> String {
    match side {
        crate::domain::trade::Side::Buy => {
            format!("amount_units={}", amount_lamports.unwrap_or_default())
        }
        crate::domain::trade::Side::Sell => {
            format!("sell_percent={}", sell_percent.unwrap_or_default())
        }
    }
}

pub(crate) fn ensure_execution_allowed(
    trade: &ValidatedTrade,
    default_simulate: bool,
    broadcast: BroadcastPolicy,
) -> anyhow::Result<()> {
    broadcast.ensure_trade_allowed(
        trade.execution_method == ExecutionMethod::Sdk && !trade.aggregator_sol,
        trade.simulate.unwrap_or(default_simulate),
    )
}

/// A backend report, not proof that a signature is valid or finalized on-chain.
pub struct TradeExecutionResult {
    pub success: bool,
    pub signature: Option<String>,
    pub error: Option<String>,
}

/// A durable original-order journal owns its parent row; callers must not overwrite it.
pub(crate) struct JournaledTradeOutcome {
    pub record: Option<crate::application::trade_record::TradeRecordV1>,
    pub signature: Option<String>,
    pub error: Option<String>,
}

/// SDK entry may have signed/submitted even when no receipt reached this process.
#[derive(Debug)]
pub(crate) struct UncertainSdkSubmission {
    signatures: Vec<String>,
    message: String,
}

impl UncertainSdkSubmission {
    pub(crate) fn new(signatures: Vec<String>, message: String) -> Self {
        Self {
            signatures,
            message,
        }
    }
    pub(crate) fn signatures(&self) -> &[String] {
        &self.signatures
    }
}

impl std::fmt::Display for UncertainSdkSubmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if self.signatures.len() > 1 {
            write!(f, "; known SDK signatures=[{}]", self.signatures.join(","))?;
        }
        Ok(())
    }
}
impl std::error::Error for UncertainSdkSubmission {}

/// An ordinary SOL aggregator reported an uncertain send, not a proven chain failure.
#[derive(Debug)]
pub(crate) struct UncertainSolBuySubmission {
    signature: Option<String>,
    message: String,
}

impl UncertainSolBuySubmission {
    pub(crate) fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }
    pub(crate) fn new(signature: Option<String>, message: Option<String>) -> Self {
        Self {
            signature,
            message: crate::sanitize::redact_urls(
                message.as_deref().unwrap_or(
                    "SOL buy submission outcome uncertain; reconcile original signature",
                ),
            ),
        }
    }
}

impl std::fmt::Display for UncertainSolBuySubmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for UncertainSolBuySubmission {}

/// Preserve untyped legacy classification; typed live uncertainty is not reconciliation.
pub(crate) fn classify_result(
    result: anyhow::Result<TradeExecutionResult>,
    simulate: bool,
) -> (TradeStatus, Option<String>, Option<String>) {
    if !simulate {
        if let Err(error) = &result {
            if let Some(uncertain) = error.downcast_ref::<UncertainSdkSubmission>() {
                return (
                    TradeStatus::Unknown,
                    uncertain.signatures().first().cloned(),
                    Some(uncertain.to_string()),
                );
            }
            if let Some(uncertain) = error.downcast_ref::<UncertainSolBuySubmission>() {
                return (
                    TradeStatus::Unknown,
                    uncertain.signature.clone(),
                    Some(uncertain.to_string()),
                );
            }
        }
    }
    match result {
        Ok(result) if result.success => {
            if simulate {
                return (TradeStatus::Simulated, result.signature, result.error);
            }
            if result.signature.is_some() {
                (TradeStatus::Submitted, result.signature, result.error)
            } else {
                (TradeStatus::Unknown, None, result.error)
            }
        }
        Ok(result) => {
            let known_failed_submission = result.error.as_deref().is_some_and(|error| {
                error.contains("landed with transaction error")
                    || error.contains("submit signature not visible")
                    || error.contains("sendTransaction failed")
            });
            let status = if known_failed_submission {
                TradeStatus::Failed
            } else if result.signature.is_some() {
                TradeStatus::Unknown
            } else {
                TradeStatus::Failed
            };
            (status, result.signature, result.error)
        }
        Err(error) => (TradeStatus::Failed, None, Some(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_sdk_uncertainty_keeps_signatures_even_under_context() {
        for signatures in [
            vec![],
            vec!["original-a".into()],
            vec!["original-a".into(), "original-b".into()],
        ] {
            let error = anyhow::Error::new(UncertainSdkSubmission::new(
                signatures.clone(),
                "opaque SDK outcome".into(),
            ))
            .context("outer context must not hide signature diagnostics");
            let (status, signature, error) = classify_result(Err(error), false);
            assert_eq!(status, TradeStatus::Unknown);
            assert_eq!(signature, signatures.first().cloned());
            let error = error.unwrap();
            assert!(error.contains("opaque SDK outcome"));
            if signatures.len() > 1 {
                assert!(error.contains("original-b"));
            }
        }
    }

    #[test]
    fn typed_sdk_uncertainty_does_not_change_simulation_error_classification() {
        let error = UncertainSdkSubmission::new(vec!["original".into()], "synthetic".into());
        let (status, signature, error) = classify_result(Err(error.into()), true);
        assert_eq!(status, TradeStatus::Failed);
        assert_eq!(signature, None);
        assert_eq!(error.as_deref(), Some("synthetic"));
    }

    #[test]
    fn legacy_untyped_text_is_not_promoted_to_typed_unknown() {
        for text in [
            "opaque SDK outcome",
            "sendTransaction failed",
            "submit signature not visible",
            "landed with transaction error",
        ] {
            let (status, signature, _) = classify_result(Err(anyhow::anyhow!(text)), false);
            assert_eq!(status, TradeStatus::Failed);
            assert_eq!(signature, None);
        }
    }
}
