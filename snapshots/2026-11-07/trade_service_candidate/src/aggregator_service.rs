use std::fmt;

use crate::aggregator::WalletBalanceSnapshot;
use crate::aggregator_quote::{
    self, BoundSolBuyQuote, QuoteStore, SolBuyQuoteRequest, SolBuySubmitRequest,
    SolBuySubmitResponse, SubmissionReservation,
};
use crate::sdk_client::SdkTradeClient;

#[derive(Debug)]
pub enum QuoteCreationError {
    Invalid(anyhow::Error),
    BalanceUnavailable(anyhow::Error),
    InsufficientBalance,
    Provider(anyhow::Error),
    Persistence(anyhow::Error),
}

impl fmt::Display for QuoteCreationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => write!(formatter, "invalid Jupiter quote request: {error}"),
            Self::BalanceUnavailable(error) => {
                write!(formatter, "wallet balance unavailable: {error}")
            }
            Self::InsufficientBalance => {
                formatter.write_str("wallet SOL balance below quote amount")
            }
            Self::Provider(error) => write!(formatter, "Jupiter quote unavailable: {error}"),
            Self::Persistence(error) => {
                write!(formatter, "Jupiter quote persistence failed: {error}")
            }
        }
    }
}

impl std::error::Error for QuoteCreationError {}

#[derive(Debug)]
pub enum SubmissionError {
    SimulateOnly,
    Invalid(String),
    Conflict,
    InFlight,
    Reservation(anyhow::Error),
    MarkSubmitting(anyhow::Error),
    Execution(anyhow::Error),
    Finalize(anyhow::Error),
}

impl fmt::Display for SubmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SimulateOnly => {
                formatter.write_str("this Trade Service instance only accepts simulations")
            }
            Self::Invalid(message) => formatter.write_str(message),
            Self::Conflict => formatter.write_str("idempotency key was used for another quote"),
            Self::InFlight => formatter
                .write_str("idempotency key is already reserved; recover the original submission"),
            Self::Reservation(error) => write!(formatter, "aggregator reservation failed: {error}"),
            Self::MarkSubmitting(error) => {
                write!(formatter, "aggregator submission state failed: {error}")
            }
            Self::Execution(error) => write!(formatter, "aggregator execution failed: {error}"),
            Self::Finalize(error) => write!(
                formatter,
                "aggregator submission persistence failed: {error}"
            ),
        }
    }
}

impl std::error::Error for SubmissionError {}

fn validate_submission_identity(
    bound: &BoundSolBuyQuote,
    wallet_pubkey: &str,
    request: &SolBuySubmitRequest,
) -> Result<(), SubmissionError> {
    if bound.expires_at_ms <= aggregator_quote::current_time_ms() {
        return Err(SubmissionError::Invalid("quote expired".into()));
    }
    if !matches!(bound.status.as_str(), "quoted" | "preflighted") {
        return Err(SubmissionError::Invalid(
            "quote is terminal; request a new quote".into(),
        ));
    }
    if bound.provider != "jupiter" {
        return Err(SubmissionError::Invalid(
            "quote provider is not Jupiter".into(),
        ));
    }
    if request.wallet_id != bound.wallet_id || wallet_pubkey != bound.wallet_pubkey {
        return Err(SubmissionError::Invalid("quote wallet mismatch".into()));
    }
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        return Err(SubmissionError::Invalid(
            "idempotency_key is required and must be at most 128 characters".into(),
        ));
    }
    if !request.simulate && !request.confirmed {
        return Err(SubmissionError::Invalid(
            "live SOL buy requires confirmed=true".into(),
        ));
    }
    Ok(())
}

pub async fn submit_jupiter_bound_quote(
    store: &QuoteStore,
    bound: &BoundSolBuyQuote,
    wallet: &SdkTradeClient,
    request: &SolBuySubmitRequest,
    simulate_only: bool,
) -> Result<SolBuySubmitResponse, SubmissionError> {
    if simulate_only && !request.simulate {
        return Err(SubmissionError::SimulateOnly);
    }
    // Reject before reserving the idempotency key or asking Jupiter to build.
    aggregator_quote::require_gate_two_for_submission(request.simulate)
        .map_err(|error| SubmissionError::Invalid(error.to_string()))?;
    validate_submission_identity(bound, &wallet.wallet().to_string(), request)?;
    let key = format!("{}:{}", request.wallet_id, request.idempotency_key);
    match store
        .reserve_submission(&key, &bound.quote_id)
        .await
        .map_err(SubmissionError::Reservation)?
    {
        SubmissionReservation::Existing(response) => return Ok(response),
        SubmissionReservation::Conflict => return Err(SubmissionError::Conflict),
        SubmissionReservation::InFlight | SubmissionReservation::Unknown => {
            return Err(SubmissionError::InFlight)
        }
        SubmissionReservation::Reserved => {}
    }
    store
        .mark_submitting(&key, &bound.quote_id)
        .await
        .map_err(SubmissionError::MarkSubmitting)?;
    let response = match aggregator_quote::submit(bound, wallet, request).await {
        Ok(response) => response,
        Err(error) => {
            let _ = store
                .mark_unknown(&key, &bound.quote_id, &error.to_string())
                .await;
            let _ = store.mark_quote_status(&bound.quote_id, "failed").await;
            return Err(SubmissionError::Execution(error));
        }
    };
    store
        .finalize_submission(&key, response.clone())
        .await
        .map_err(SubmissionError::Finalize)?;
    Ok(response)
}

fn balance_snapshot(
    balance_lamports: u64,
    amount_lamports: u64,
) -> Result<WalletBalanceSnapshot, QuoteCreationError> {
    if balance_lamports < amount_lamports {
        return Err(QuoteCreationError::InsufficientBalance);
    }
    Ok(WalletBalanceSnapshot {
        balance_lamports,
        fetched_at_ms: aggregator_quote::current_time_ms(),
    })
}

/// The compatibility HTTP API and `/v1/trades` use the same funded quote path.
/// Read-only provider comparison deliberately does not call this function so
/// that an unfunded wallet can still sample public market quotes.
pub async fn create_jupiter_bound_quote(
    store: &QuoteStore,
    request: &SolBuyQuoteRequest,
    wallet: &SdkTradeClient,
) -> Result<BoundSolBuyQuote, QuoteCreationError> {
    aggregator_quote::validate_request(request).map_err(QuoteCreationError::Invalid)?;
    let balance_lamports = wallet
        .wallet_balance()
        .await
        .map_err(QuoteCreationError::BalanceUnavailable)?;
    let snapshot = balance_snapshot(balance_lamports, request.amount_lamports)?;
    let mut quote = aggregator_quote::quote(request)
        .await
        .map_err(QuoteCreationError::Provider)?;
    quote.balance_snapshot = Some(snapshot);
    store
        .insert(request, wallet.wallet().to_string(), quote)
        .await
        .map_err(QuoteCreationError::Persistence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound_quote() -> BoundSolBuyQuote {
        BoundSolBuyQuote {
            quote_id: "quote-1".into(),
            provider: "jupiter".into(),
            provider_quote_id: "jup:route".into(),
            route_fingerprint: "route".into(),
            request_hash: "request".into(),
            wallet_id: "wallet-1".into(),
            wallet_pubkey: "payer-1".into(),
            quote_slippage_bps: 100,
            expires_at_ms: aggregator_quote::current_time_ms() + 10_000,
            status: "quoted".into(),
            quote: aggregator_quote::SolBuyQuoteResponse {
                input_mint: crate::models::WSOL_MINT.into(),
                output_mint: crate::models::USDC_MINT.into(),
                amount_lamports: 1_000_000,
                expected_output_units: 1,
                minimum_output_units: 1,
                price_impact_percent: "0".into(),
                route: vec![],
                executable: false,
                balance_snapshot: None,
            },
            message_hash: None,
            preflight: None,
            audit: None,
        }
    }

    #[test]
    fn submission_identity_is_checked_before_reservation() {
        let mut bound = bound_quote();
        let mut request = SolBuySubmitRequest {
            wallet_id: "wallet-1".into(),
            confirmed: false,
            simulate: true,
            idempotency_key: "same-key".into(),
        };
        assert!(validate_submission_identity(&bound, "payer-1", &request).is_ok());
        request.wallet_id = "wallet-2".into();
        assert!(validate_submission_identity(&bound, "payer-1", &request).is_err());
        request.wallet_id = "wallet-1".into();
        assert!(validate_submission_identity(&bound, "payer-2", &request).is_err());
        request.idempotency_key.clear();
        assert!(validate_submission_identity(&bound, "payer-1", &request).is_err());
        request.idempotency_key = "same-key".into();
        request.simulate = false;
        assert!(validate_submission_identity(&bound, "payer-1", &request).is_err());
        request.simulate = true;
        bound.provider = "dflow".into();
        assert!(validate_submission_identity(&bound, "payer-1", &request).is_err());
        bound.provider = "jupiter".into();
        bound.status = "failed".into();
        assert!(validate_submission_identity(&bound, "payer-1", &request).is_err());
        bound.status = "preflighted".into();
        assert!(validate_submission_identity(&bound, "payer-1", &request).is_ok());
        bound.expires_at_ms = 0;
        assert!(validate_submission_identity(&bound, "payer-1", &request).is_err());
    }

    #[test]
    fn funded_quote_snapshot_rejects_insufficient_wallet_balance() {
        assert!(matches!(
            balance_snapshot(999_999, 1_000_000),
            Err(QuoteCreationError::InsufficientBalance)
        ));
        let snapshot = balance_snapshot(1_000_000, 1_000_000).unwrap();
        assert_eq!(snapshot.balance_lamports, 1_000_000);
        assert!(snapshot.fetched_at_ms > 0);
    }
}
