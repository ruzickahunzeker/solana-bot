use super::*;
use crate::{
    application::{
        ports::ordinary_sol_buy::SolBuyIntent,
        quote_submission_types::{QuoteSubmissionOutcome, QuoteSubmissionResult},
        trade_command::{TradeExecution, TradeRoute},
        trade_execution::classify_result,
    },
    domain::{
        quote::{BoundQuote, QuoteEstimate, QuotePreflight},
        trade::{
            Address, ExecutionMethod, ExecutionMode, MevProvider, Pool, TradeAmount, TradeStatus,
        },
    },
};
use std::sync::Mutex;

struct Pipeline {
    events: Mutex<Vec<&'static str>>,
    requests: Mutex<Vec<SolBuyIntent>>,
    submissions: Mutex<Vec<QuoteSubmissionCommand>>,
    failure: Option<&'static str>,
    outcome: QuoteSubmissionOutcome,
    signature: Option<&'static str>,
    error: Option<&'static str>,
}

fn snapshot() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap()
}

impl Pipeline {
    fn new() -> Self {
        Self {
            events: Mutex::default(),
            requests: Mutex::default(),
            submissions: Mutex::default(),
            failure: None,
            outcome: QuoteSubmissionOutcome::Submitted,
            signature: Some("synthetic-original-signature"),
            error: Some("literal backend report"),
        }
    }
    fn event(&self, stage: &'static str) -> anyhow::Result<()> {
        self.events.lock().unwrap().push(stage);
        if self.failure == Some(stage) {
            anyhow::bail!("synthetic {stage} failure");
        }
        Ok(())
    }
    fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
}

impl OrdinarySolBuyPipeline for Pipeline {
    type Request = SolBuyIntent;
    fn map_request(&self, input: SolBuyIntent) -> Self::Request {
        self.events.lock().unwrap().push("map");
        self.requests.lock().unwrap().push(input.clone());
        input
    }
    fn validate_request(&self, _: &Self::Request) -> anyhow::Result<()> {
        self.event("validate")
    }
    async fn quote(&self, _: &Self::Request) -> anyhow::Result<QuoteEstimate> {
        self.event("quote")?;
        Ok(
            serde_json::from_value::<BoundQuote>(snapshot()["bound"].clone())
                .unwrap()
                .quote,
        )
    }
    fn bind(&self, _: &Self::Request, quote: QuoteEstimate) -> anyhow::Result<BoundQuote> {
        self.event("bind")?;
        let mut bound: BoundQuote = serde_json::from_value(snapshot()["bound"].clone()).unwrap();
        assert_eq!(quote, bound.quote);
        bound.quote = quote;
        Ok(bound)
    }
    async fn submit(
        &self,
        bound: &BoundQuote,
        request: &QuoteSubmissionCommand,
    ) -> anyhow::Result<QuoteSubmissionResult> {
        self.event("submit")?;
        self.submissions.lock().unwrap().push(request.clone());
        let expected: BoundQuote = serde_json::from_value(snapshot()["bound"].clone()).unwrap();
        assert_eq!(*bound, expected);
        let preflight: QuotePreflight =
            serde_json::from_value(snapshot()["preflight"].clone()).unwrap();
        Ok(QuoteSubmissionResult {
            quote_id: bound.quote_id.clone(),
            wallet_id: bound.wallet_id.clone(),
            status: self.outcome,
            signature: self.signature.map(str::to_owned),
            error: self.error.map(str::to_owned),
            preflight,
        })
    }
}

fn command() -> TradeCommand {
    TradeCommand {
        wallet_id: "fixture-wallet".into(),
        idempotency_key: " literal-key ".into(),
        route: TradeRoute {
            pool: Pool::PumpAmm,
            mint: Address::from_bytes([1; 32]),
            quote_mint: Address::from_bytes([2; 32]),
            pool_id: None,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
        },
        amount: TradeAmount::BuySol {
            lamports: 1_000_000,
        },
        execution: TradeExecution {
            method: ExecutionMethod::Sdk,
            slippage_bps: 1234,
            tip_sol: 0.00037,
            use_mev: Some(true),
            mev_provider: Some(MevProvider::Temporal),
            mode: ExecutionMode::Live,
        },
        aggregator_sol: true,
        pumpapi_priority_fee_sol: 0.0007,
    }
}

#[tokio::test]
async fn ordinary_sol_buy_preserves_mapping_order_and_live_simulate_confirmation() {
    for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
        for simulate in [false, true] {
            let pipeline = Pipeline::new();
            let mut command = command();
            command.execution.method = method;
            command.execution.mode = ExecutionMode::resolve(Some(simulate), false);
            let result = execute(&pipeline, &command).await.unwrap();
            assert!(result.success);
            assert_eq!(
                pipeline.events(),
                ["map", "validate", "quote", "bind", "submit"]
            );
            assert_eq!(
                &*pipeline.requests.lock().unwrap(),
                &[SolBuyIntent {
                    wallet_id: command.wallet_id.clone(),
                    mint: command.route.mint,
                    amount_lamports: 1_000_000,
                    slippage_bps: 1234
                }]
            );
            let submissions = pipeline.submissions.lock().unwrap();
            assert_eq!(submissions.len(), 1);
            assert_eq!(submissions[0].wallet_id, command.wallet_id);
            assert_eq!(submissions[0].idempotency_key, " literal-key ");
            assert_eq!(submissions[0].simulate, simulate);
            assert_eq!(submissions[0].confirmed, !simulate);
            assert_eq!(
                result.signature.as_deref(),
                Some("synthetic-original-signature")
            );
            assert_eq!(result.error.as_deref(), Some("literal backend report"));
        }
    }
}

#[tokio::test]
async fn ordinary_sol_buy_amount_and_slippage_errors_precede_every_pipeline_stage() {
    let pipeline = Pipeline::new();
    let mut command = command();
    command.amount = TradeAmount::SellPercent { percent: 100 };
    command.execution.slippage_bps = u64::MAX;
    assert_eq!(
        execute(&pipeline, &command)
            .await
            .err()
            .unwrap()
            .to_string(),
        "SOL aggregator buy amount is missing"
    );
    assert!(pipeline.events().is_empty());
    command.amount = TradeAmount::BuySol { lamports: 1 };
    assert_eq!(
        execute(&pipeline, &command)
            .await
            .err()
            .unwrap()
            .to_string(),
        "aggregator slippage exceeds supported range"
    );
    assert!(pipeline.events().is_empty());
}

#[tokio::test]
async fn ordinary_sol_buy_stage_failures_stop_without_later_calls_or_retry() {
    for (failure, expected) in [
        ("validate", vec!["map", "validate"]),
        ("quote", vec!["map", "validate", "quote"]),
        ("bind", vec!["map", "validate", "quote", "bind"]),
        ("submit", vec!["map", "validate", "quote", "bind", "submit"]),
    ] {
        let mut pipeline = Pipeline::new();
        pipeline.failure = Some(failure);
        assert_eq!(
            execute(&pipeline, &command())
                .await
                .err()
                .unwrap()
                .to_string(),
            format!("synthetic {failure} failure")
        );
        assert_eq!(pipeline.events(), expected);
    }
}

#[tokio::test]
async fn ordinary_sol_buy_result_projection_preserves_other_outcomes_and_types_live_uncertainty() {
    for (outcome, success) in [
        (QuoteSubmissionOutcome::Pending, false),
        (QuoteSubmissionOutcome::Simulated, true),
        (QuoteSubmissionOutcome::Submitted, true),
        (QuoteSubmissionOutcome::Confirmed, false),
        (QuoteSubmissionOutcome::Finalized, false),
        (QuoteSubmissionOutcome::Unknown, false),
        (QuoteSubmissionOutcome::Failed, false),
    ] {
        for signature in [None, Some(""), Some("original")] {
            let mut pipeline = Pipeline::new();
            pipeline.outcome = outcome;
            pipeline.signature = signature;
            let result = execute(&pipeline, &command()).await;
            if outcome == QuoteSubmissionOutcome::Unknown {
                assert!(result
                    .as_ref()
                    .err()
                    .unwrap()
                    .is::<UncertainSolBuySubmission>());
                let (status, original, error) = classify_result(result, false);
                assert_eq!(status, TradeStatus::Unknown);
                assert_eq!(original.as_deref(), signature);
                assert_eq!(error.as_deref(), Some("literal backend report"));
            } else {
                let result = result.unwrap();
                assert_eq!(result.success, success);
                assert_eq!(result.signature.as_deref(), signature);
                assert_eq!(result.error.as_deref(), Some("literal backend report"));
            }
            assert_eq!(
                pipeline.events(),
                ["map", "validate", "quote", "bind", "submit"]
            );
        }
    }
}

#[tokio::test]
async fn ordinary_sol_buy_live_unknown_is_not_classified_from_failure_text_or_missing_signature() {
    for signature in [None, Some(""), Some("original")] {
        for report in [
            None,
            Some(""),
            Some("sendTransaction failed"),
            Some("submit signature not visible"),
            Some("landed with transaction error"),
            Some("RPC https://rpc.example.invalid/private-key timed out"),
        ] {
            let mut pipeline = Pipeline::new();
            pipeline.outcome = QuoteSubmissionOutcome::Unknown;
            pipeline.signature = signature;
            pipeline.error = report;
            let error = execute(&pipeline, &command()).await.err().unwrap();
            assert!(error.is::<UncertainSolBuySubmission>());
            let (status, original, message) =
                classify_result(Err(error.context("outer context")), false);
            assert_eq!(status, TradeStatus::Unknown);
            assert_eq!(original.as_deref(), signature);
            assert_eq!(
                message.as_deref(),
                Some(match report {
                    None => "SOL buy submission outcome uncertain; reconcile original signature",
                    Some(value) if value.contains("https://") => "RPC <redacted-url> timed out",
                    Some(value) => value,
                })
            );
            assert_eq!(
                pipeline.events(),
                ["map", "validate", "quote", "bind", "submit"]
            );
        }
    }
}

#[tokio::test]
async fn ordinary_sol_buy_simulation_does_not_gain_live_uncertainty_semantics() {
    let mut pipeline = Pipeline::new();
    pipeline.outcome = QuoteSubmissionOutcome::Unknown;
    pipeline.signature = None;
    let mut command = command();
    command.execution.mode = ExecutionMode::Simulate;
    let result = execute(&pipeline, &command).await.unwrap();
    assert!(!result.success);
    assert_eq!(classify_result(Ok(result), true).0, TradeStatus::Failed);
    let error = UncertainSolBuySubmission::new(Some("original".into()), Some("synthetic".into()));
    let (status, signature, error) = classify_result(Err(error.into()), true);
    assert_eq!(status, TradeStatus::Failed);
    assert!(signature.is_none());
    assert_eq!(error.as_deref(), Some("synthetic"));
}
