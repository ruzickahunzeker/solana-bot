use anyhow::{bail, Result};
use sol_trade_sdk::{
    client::TradingInfrastructure,
    common::{GasFeeStrategy, GasFeeStrategyType},
    swqos::{SwqosType, TradeType},
};
use solana_sdk::signature::Signature;
use std::{future::Future, sync::Arc};

use super::trade_command::ExecuteTrade;
use crate::{
    application::trade_execution::{TradeExecutionResult, UncertainSdkSubmission},
    domain::{reconciliation::ChainObservation, trade::Side},
    sanitize::redact_urls,
};

pub(crate) struct SdkLaneReport {
    pub(crate) success: bool,
    pub(crate) signatures: Vec<String>,
    pub(crate) error: Option<String>,
}

pub(crate) struct SdkInvocationFailure {
    pub(crate) signatures: Vec<String>,
    pub(crate) message: String,
}

pub(crate) enum SdkLaneObservation {
    Skipped,
    Observed(ChainObservation),
    NotVisible,
}

pub(crate) trait OrdinarySdkLane: Send + Sync {
    type Prepared: Send;
    fn provider(&self) -> SwqosType;
    fn prepare(&self, trade: &ExecuteTrade) -> impl Future<Output = Result<Self::Prepared>> + Send;
    fn validate_prepared(&self, _prepared: &Self::Prepared, _trade: &ExecuteTrade) -> Result<()> {
        Ok(())
    }
    fn invoke(
        &self,
        prepared: Self::Prepared,
    ) -> impl Future<Output = Result<SdkLaneReport, SdkInvocationFailure>> + Send;
    fn observe(
        &self,
        signature: &str,
        trade: &ExecuteTrade,
    ) -> impl Future<Output = Result<SdkLaneObservation>> + Send;
}

pub(crate) fn ensure_sender_layout(
    expected: SwqosType,
    actual: &[SwqosType],
    middleware: bool,
) -> Result<()> {
    if actual != [expected] {
        bail!(
            "{} SDK lane requires exactly one matching sender; actual={actual:?}",
            expected.as_str()
        );
    }
    if middleware {
        bail!("SDK middleware requires a separate execution audit");
    }
    Ok(())
}

/// SDK initialization appends Default. Select the intended sender, never its fallback.
pub(crate) fn selected_sender_index(
    expected: SwqosType,
    actual: &[SwqosType],
    middleware: bool,
) -> Result<usize> {
    if middleware {
        bail!("SDK middleware requires a separate execution audit");
    }
    let normalized = expected != SwqosType::Default
        && (actual == [expected, SwqosType::Default] || actual == [SwqosType::Default, expected]);
    if actual != [expected] && !normalized {
        bail!(
            "{} SDK lane has no unique intended sender; actual={actual:?}",
            expected.as_str()
        );
    }
    actual
        .iter()
        .position(|provider| *provider == expected)
        .ok_or_else(|| anyhow::anyhow!("intended SDK sender missing"))
}

pub(crate) fn execution_infrastructure(
    original: &TradingInfrastructure,
    expected: SwqosType,
    middleware: bool,
) -> Result<Arc<TradingInfrastructure>> {
    let actual = original
        .swqos_clients
        .iter()
        .map(|sender| sender.get_swqos_type())
        .collect::<Vec<_>>();
    let index = selected_sender_index(expected, &actual, middleware)?;
    let sender = original.swqos_clients[index].clone();
    ensure_sender_layout(expected, &[sender.get_swqos_type()], false)?;
    Ok(Arc::new(TradingInfrastructure {
        rpc: original.rpc.clone(),
        swqos_clients: Arc::new(vec![sender]),
        config: original.config.clone(),
        max_sender_concurrency: original.max_sender_concurrency,
        effective_core_ids: original.effective_core_ids.clone(),
    }))
}

pub(crate) fn ensure_single_fee_strategy(
    fees: &GasFeeStrategy,
    provider: SwqosType,
    side: Side,
    simulate: bool,
) -> Result<()> {
    let target = if simulate {
        SwqosType::Default
    } else {
        provider
    };
    let strategies = fees.get_strategies(match side {
        Side::Buy => TradeType::Buy,
        Side::Sell => TradeType::Sell,
    });
    let selected = strategies
        .iter()
        .filter(|(lane, _, _)| *lane == target)
        .collect::<Vec<_>>();
    if selected.len() != 1 || selected[0].1 != GasFeeStrategyType::Normal {
        bail!(
            "{} SDK lane requires one Normal fee strategy",
            target.as_str()
        );
    }
    Ok(())
}

pub(crate) fn decode_sdk_report(
    result: Result<(
        bool,
        Vec<Signature>,
        Option<sol_trade_sdk::swqos::common::TradeError>,
        Vec<(SwqosType, i64)>,
    )>,
) -> Result<SdkLaneReport, SdkInvocationFailure> {
    let (success, signatures, error, _) = result.map_err(|error| SdkInvocationFailure {
        signatures: vec![],
        message: redact_urls(&error.to_string()),
    })?;
    Ok(SdkLaneReport {
        success,
        signatures: unique_signatures(
            &signatures
                .into_iter()
                .filter(|s| *s != Signature::default())
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        ),
        error: error.map(|e| redact_urls(&format!("{e:?}"))),
    })
}

fn unique_signatures(input: &[String]) -> Vec<String> {
    let mut signatures = vec![];
    for signature in input {
        if !signature.is_empty() && !signatures.contains(signature) {
            signatures.push(signature.clone());
        }
    }
    signatures
}

fn with_notes(message: String, notes: &[String]) -> String {
    if notes.is_empty() {
        return message;
    }
    let note = format!("fallback_notes=[{}]", notes.join("; "));
    if message.is_empty() {
        note
    } else {
        format!("{message}; {note}")
    }
}

fn uncertain(signatures: Vec<String>, message: String, notes: &[String]) -> anyhow::Error {
    UncertainSdkSubmission::new(
        unique_signatures(&signatures),
        with_notes(redact_urls(&message), notes),
    )
    .into()
}

/// Only failures before invoke permit a different lane. SDK entry is an irreversible boundary.
pub(crate) async fn execute<L: OrdinarySdkLane>(
    lanes: impl IntoIterator<Item = L>,
    trade: &ExecuteTrade,
) -> Result<TradeExecutionResult> {
    let mut last_error = None;
    let mut notes = vec![];
    for lane in lanes {
        let label = lane.provider().as_str();
        let prepared = match lane.prepare(trade).await {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(kind) = preparation_rejection(&error) {
                    let reason = redact_urls(&error.to_string());
                    return Err(
                        error.context(format!("{label} preparation {kind} rejected: {reason}"))
                    );
                }
                let error = redact_urls(&error.to_string());
                tracing::warn!(swqos_lane = label, error = %error, "SDK lane preparation failed before invocation");
                if lane.provider() != SwqosType::Default {
                    notes.push(format!("{label} lane skipped: {error}"));
                }
                last_error = Some(anyhow::anyhow!("{label}: {error}"));
                continue;
            }
        };
        if let Err(error) = lane.validate_prepared(&prepared, trade) {
            let reason = redact_urls(&error.to_string());
            return Err(error.context(format!(
                "{label} pre-invoke preparation verification failed: {reason}"
            )));
        }
        tracing::info!(
            swqos_lane = label,
            simulate = trade.simulate,
            "entering SDK lane; automatic post-entry fallback disabled"
        );
        let report = match lane.invoke(prepared).await {
            Ok(report) => report,
            Err(failure) => {
                let message = format!(
                    "{label} SDK execution outcome uncertain; automatic lane fallback disabled: {}",
                    failure.message
                );
                if trade.simulate {
                    let signatures = unique_signatures(&failure.signatures);
                    let mut message = with_notes(redact_urls(&message), &notes);
                    if signatures.len() > 1 {
                        message.push_str(&format!(
                            "; known SDK signatures=[{}]",
                            signatures.join(",")
                        ));
                    }
                    return Ok(TradeExecutionResult {
                        success: false,
                        signature: signatures.first().cloned(),
                        error: Some(message),
                    });
                }
                return Err(uncertain(failure.signatures, message, &notes));
            }
        };
        let signatures = unique_signatures(&report.signatures);
        if trade.simulate {
            let error = report.error.map(|e| redact_urls(&e));
            let mut result = TradeExecutionResult {
                success: report.success,
                signature: signatures.first().cloned(),
                error,
            };
            if !notes.is_empty() {
                result.error = Some(with_notes(result.error.unwrap_or_default(), &notes));
            }
            if signatures.len() > 1 {
                result.error = Some(format!(
                    "{}; known SDK signatures=[{}]",
                    result.error.unwrap_or_default(),
                    signatures.join(",")
                ));
            }
            return Ok(result);
        }
        if !report.success || signatures.len() != 1 {
            let reason = if signatures.is_empty() {
                "SDK returned no usable signature"
            } else if signatures.len() > 1 {
                "SDK returned multiple transaction signatures"
            } else {
                "SDK returned non-success"
            };
            let message = report
                .error
                .map_or_else(|| reason.to_string(), |error| format!("{reason}: {error}"));
            return Err(uncertain(
                signatures,
                format!("{label} {message}; automatic lane fallback disabled"),
                &notes,
            ));
        }
        let signature = &signatures[0];
        match lane.observe(signature, trade).await {
            Ok(SdkLaneObservation::Skipped) => {}
            Ok(SdkLaneObservation::Observed(observation)) if observation.signature == *signature && observation.slot > 0 && observation.error.is_none() => {}
            Ok(SdkLaneObservation::Observed(observation)) => {
                let error = observation.error.unwrap_or_else(|| "signature observation identity or slot mismatch".into());
                return Err(uncertain(signatures, format!("{label} signature observation is not success evidence: {error}"), &notes));
            }
            Ok(SdkLaneObservation::NotVisible) => return Err(uncertain(signatures, format!("{label} submit signature not visible after {}ms; automatic lane fallback disabled", trade.mev_submit_verify_ms), &notes)),
            Err(error) => return Err(uncertain(signatures, format!("{label} signature verification unavailable: {error}"), &notes)),
        }
        let mut result = TradeExecutionResult {
            success: true,
            signature: Some(signature.clone()),
            error: report.error.map(|e| redact_urls(&e)),
        };
        if !notes.is_empty() {
            result.error = Some(with_notes(result.error.unwrap_or_default(), &notes));
        }
        return Ok(result);
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no RPC lanes configured")))
}

#[cfg(test)]
mod tests;

pub(crate) fn ensure_requested_provider(
    use_mev: Option<bool>,
    requested: Option<crate::domain::trade::MevProvider>,
    actual: SwqosType,
) -> Result<()> {
    use crate::domain::trade::MevProvider;
    let allowed = match (use_mev, requested) {
        (Some(false), Some(provider))
            if !matches!(provider, MevProvider::Auto | MevProvider::Default) =>
        {
            false
        }
        (Some(true), Some(MevProvider::Default)) => false,
        (Some(false), _) | (_, Some(MevProvider::Default)) => actual == SwqosType::Default,
        (_, Some(provider)) if provider != MevProvider::Auto => {
            crate::sdk_client::provider_swqos_type(provider) == Some(actual)
        }
        (Some(true), _) => actual != SwqosType::Default,
        _ => true,
    };
    anyhow::ensure!(
        allowed,
        "DAMM selected provider differs from the original MEV policy"
    );
    Ok(())
}

pub(crate) fn preparation_rejection(error: &anyhow::Error) -> Option<&'static str> {
    if error
        .downcast_ref::<crate::domain::preparation_balance::PreparationBalanceFailure>()
        .is_some_and(|failure| !failure.allows_lane_fallback())
    {
        Some("balance")
    } else if error
        .downcast_ref::<crate::domain::parameter_read::ParameterReadFailure>()
        .is_some_and(|failure| !failure.allows_lane_fallback())
    {
        Some("parameters")
    } else {
        None
    }
}
