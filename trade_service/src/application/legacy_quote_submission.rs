//! Ordinary aggregator path with original-receipt uncertainty; still no signed journal.
use anyhow::{bail, Result};

use crate::{
    application::{
        ports::quote_execution::QuoteSubmissionBackend,
        quote_submission_types::{
            QuoteSubmissionCommand, QuoteSubmissionOutcome, QuoteSubmissionResult,
        },
    },
    domain::quote::BoundQuote,
};

pub(crate) async fn execute<B: QuoteSubmissionBackend>(
    backend: &B,
    bound: &BoundQuote,
    request: &QuoteSubmissionCommand,
) -> Result<QuoteSubmissionResult> {
    if request.wallet_id != bound.wallet_id {
        bail!("quote wallet mismatch");
    }
    if request.idempotency_key.trim().is_empty() || request.idempotency_key.len() > 128 {
        bail!("idempotency_key is required and must be at most 128 characters");
    }
    if !request.simulate && !request.confirmed {
        bail!("live SOL buy requires confirmed=true");
    }
    if !request.simulate {
        backend.ensure_live_allowed()?;
    }
    let (prepared, preflight) = backend.prepare(bound).await?;
    if !preflight.executable {
        bail!("aggregator preflight did not produce an executable transaction");
    }
    let mut result = QuoteSubmissionResult {
        quote_id: bound.quote_id.clone(),
        wallet_id: bound.wallet_id.clone(),
        status: QuoteSubmissionOutcome::Simulated,
        signature: None,
        error: None,
        preflight,
    };
    if request.simulate {
        return Ok(result);
    }
    backend.ensure_live_allowed()?;
    let signed = backend.sign(prepared)?;
    match backend.broadcast(&signed).await {
        Ok(signature) if signature == signed.signature => {
            result.status = QuoteSubmissionOutcome::Submitted;
            result.signature = Some(signature);
        }
        Ok(_) => {
            result.status = QuoteSubmissionOutcome::Unknown;
            result.signature = Some(signed.signature);
            result.error =
                Some("RPC returned a different signature; reconcile original signature".into());
        }
        Err(error) => {
            result.status = QuoteSubmissionOutcome::Unknown;
            result.signature = Some(signed.signature);
            result.error = Some(crate::sanitize::redact_urls(&error.to_string()));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::dto::sol_buy::SolBuySubmitResponse,
        domain::{quote::QuotePreflight, submission::SignedSubmission},
    };
    use std::sync::Mutex;

    struct Backend {
        events: Mutex<Vec<&'static str>>,
        denied: bool,
        revoke_after_prepare: bool,
        executable: bool,
        fail: Option<&'static str>,
    }

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/contracts/sol_buy_v1.json"
        ))
        .unwrap()
    }

    fn bound() -> BoundQuote {
        serde_json::from_value(fixture()["bound"].clone()).unwrap()
    }
    fn command(simulate: bool) -> QuoteSubmissionCommand {
        QuoteSubmissionCommand {
            wallet_id: "fixture-wallet".into(),
            confirmed: !simulate,
            simulate,
            idempotency_key: " legacy-key ".into(),
        }
    }

    impl Backend {
        fn new() -> Self {
            Self {
                events: Mutex::new(vec![]),
                denied: false,
                revoke_after_prepare: false,
                executable: true,
                fail: None,
            }
        }
        fn events(&self) -> Vec<&'static str> {
            self.events.lock().unwrap().clone()
        }
    }

    impl QuoteSubmissionBackend for Backend {
        type Prepared = ();
        fn ensure_live_allowed(&self) -> Result<()> {
            let mut events = self.events.lock().unwrap();
            let prepared = events.contains(&"prepare");
            events.push("policy");
            if self.denied || (self.revoke_after_prepare && prepared) {
                bail!("synthetic policy denial");
            }
            Ok(())
        }
        async fn prepare(&self, _: &BoundQuote) -> Result<((), QuotePreflight)> {
            self.events.lock().unwrap().push("prepare");
            if self.fail == Some("prepare") {
                bail!("synthetic prepare error");
            }
            let mut preflight: QuotePreflight =
                serde_json::from_value(fixture()["preflight"].clone()).unwrap();
            preflight.executable = self.executable;
            Ok(((), preflight))
        }
        fn sign(&self, _: ()) -> Result<SignedSubmission> {
            self.events.lock().unwrap().push("sign");
            if self.fail == Some("sign") {
                bail!("synthetic sign error");
            }
            Ok(SignedSubmission {
                signature: "synthetic-signature".into(),
                bytes: vec![1],
            })
        }
        async fn broadcast(&self, _: &SignedSubmission) -> Result<String> {
            self.events.lock().unwrap().push("broadcast");
            if self.fail == Some("broadcast") {
                bail!("synthetic broadcast error");
            }
            if self.fail == Some("broadcast-url") {
                bail!("sendTransaction failed: https://rpc.example.invalid/private-key");
            }
            if self.fail == Some("broadcast-mismatch") {
                return Ok("not-the-original-signature".into());
            }
            Ok("synthetic-signature".into())
        }
    }

    #[tokio::test]
    async fn simulation_never_checks_live_authority_signs_or_broadcasts() {
        let mut backend = Backend::new();
        backend.denied = true;
        let result = execute(&backend, &bound(), &command(true)).await.unwrap();
        assert_eq!(result.status, QuoteSubmissionOutcome::Simulated);
        assert!(result.signature.is_none());
        assert_eq!(backend.events(), vec!["prepare"]);
    }

    #[tokio::test]
    async fn live_preserves_policy_prepare_sign_send_order_and_wire_response() {
        let backend = Backend::new();
        let result = execute(&backend, &bound(), &command(false)).await.unwrap();
        assert_eq!(
            backend.events(),
            vec!["policy", "prepare", "policy", "sign", "broadcast"]
        );
        let wire = serde_json::to_value(SolBuySubmitResponse::from(result)).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"quote_id":"fixture-quote-v1","wallet_id":"fixture-wallet","status":"submitted","signature":"synthetic-signature","error":null,"preflight":fixture()["preflight"]})
        );
    }

    #[tokio::test]
    async fn uncertain_send_preserves_original_signature_without_retry() {
        let mut backend = Backend::new();
        backend.fail = Some("broadcast");
        let result = execute(&backend, &bound(), &command(false)).await.unwrap();
        assert_eq!(result.status, QuoteSubmissionOutcome::Unknown);
        assert_eq!(result.signature.as_deref(), Some("synthetic-signature"));
        assert_eq!(result.error.as_deref(), Some("synthetic broadcast error"));
        assert_eq!(
            backend.events(),
            vec!["policy", "prepare", "policy", "sign", "broadcast"]
        );
    }

    #[tokio::test]
    async fn a_different_rpc_receipt_is_unknown_and_never_replaces_the_original_signature() {
        let mut backend = Backend::new();
        backend.fail = Some("broadcast-mismatch");
        let result = execute(&backend, &bound(), &command(false)).await.unwrap();
        assert_eq!(result.status, QuoteSubmissionOutcome::Unknown);
        assert_eq!(result.signature.as_deref(), Some("synthetic-signature"));
        assert_eq!(
            result.error.as_deref(),
            Some("RPC returned a different signature; reconcile original signature")
        );
        assert_eq!(
            backend.events(),
            vec!["policy", "prepare", "policy", "sign", "broadcast"]
        );
    }

    #[tokio::test]
    async fn uncertain_send_redacts_rpc_urls_without_losing_the_original_receipt() {
        let mut backend = Backend::new();
        backend.fail = Some("broadcast-url");
        let result = execute(&backend, &bound(), &command(false)).await.unwrap();
        assert_eq!(result.status, QuoteSubmissionOutcome::Unknown);
        assert_eq!(result.signature.as_deref(), Some("synthetic-signature"));
        assert_eq!(
            result.error.as_deref(),
            Some("sendTransaction failed: <redacted-url>")
        );
        assert_eq!(
            backend.events(),
            vec!["policy", "prepare", "policy", "sign", "broadcast"]
        );
    }

    #[tokio::test]
    async fn wallet_key_and_confirmation_errors_precede_all_backend_work() {
        for (field, message) in [
            ("wallet", "quote wallet mismatch"),
            (
                "empty-key",
                "idempotency_key is required and must be at most 128 characters",
            ),
            (
                "long-key",
                "idempotency_key is required and must be at most 128 characters",
            ),
            ("confirm", "live SOL buy requires confirmed=true"),
        ] {
            let backend = Backend::new();
            let mut request = command(false);
            match field {
                "wallet" => request.wallet_id = "different".into(),
                "empty-key" => request.idempotency_key = " ".into(),
                "long-key" => request.idempotency_key = "a".repeat(129),
                "confirm" => request.confirmed = false,
                _ => unreachable!(),
            }
            assert_eq!(
                execute(&backend, &bound(), &request)
                    .await
                    .unwrap_err()
                    .to_string(),
                message
            );
            assert!(backend.events().is_empty());
        }
    }

    #[tokio::test]
    async fn a_nonexecutable_preflight_cannot_reach_signing() {
        let mut backend = Backend::new();
        backend.executable = false;
        assert_eq!(
            execute(&backend, &bound(), &command(false))
                .await
                .unwrap_err()
                .to_string(),
            "aggregator preflight did not produce an executable transaction"
        );
        assert_eq!(backend.events(), vec!["policy", "prepare"]);
    }

    #[tokio::test]
    async fn prepare_and_sign_failures_never_broadcast() {
        for stage in ["prepare", "sign"] {
            let mut backend = Backend::new();
            backend.fail = Some(stage);
            assert!(execute(&backend, &bound(), &command(false)).await.is_err());
            assert!(!backend.events().contains(&"broadcast"));
        }
    }

    #[tokio::test]
    async fn denied_authority_before_or_after_prepare_never_signs() {
        for initially_denied in [false, true] {
            let mut backend = Backend::new();
            backend.denied = initially_denied;
            backend.revoke_after_prepare = true;
            assert_eq!(
                execute(&backend, &bound(), &command(false))
                    .await
                    .unwrap_err()
                    .to_string(),
                "synthetic policy denial"
            );
            assert!(!backend.events().contains(&"sign"));
            assert!(!backend.events().contains(&"broadcast"));
            assert_eq!(backend.events().contains(&"prepare"), !initially_denied);
        }
    }
}
