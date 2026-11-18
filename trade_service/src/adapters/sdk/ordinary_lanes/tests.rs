use super::*;
#[path = "balance_gate_tests.rs"]
mod balance_gate_tests;
mod shared_observer;
mod worker_lifecycle;
use crate::{
    adapters::sdk::ordinary_params::default_fee_strategy,
    application::trade_execution::classify_result,
    domain::{
        reconciliation::ChainCommitment,
        trade::{Pool, TradeStatus},
    },
};
use solana_sdk::{hash::Hash, pubkey::Pubkey};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

// These senders are metadata only. No SDK initializer, signer or provider transport exists.
struct MetadataSender(SwqosType);
impl sol_trade_sdk::swqos::SwqosClientTrait for MetadataSender {
    fn send_transaction<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _: TradeType,
        _: &'life1 solana_sdk::transaction::VersionedTransaction,
        _: bool,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { panic!("metadata-only sender must never submit") })
    }
    fn send_transactions<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _: TradeType,
        _: &'life1 Vec<solana_sdk::transaction::VersionedTransaction>,
        _: bool,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { panic!("metadata-only sender must never submit") })
    }
    fn get_tip_account(&self) -> Result<String> {
        panic!("metadata projection must not request a tip account")
    }
    fn get_swqos_type(&self) -> SwqosType {
        self.0
    }
}

fn infrastructure(providers: &[SwqosType]) -> TradingInfrastructure {
    TradingInfrastructure {
        rpc: Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new_mock(
            "fails".into(),
        )),
        swqos_clients: Arc::new(
            providers
                .iter()
                .map(|provider| {
                    Arc::new(MetadataSender(*provider)) as Arc<sol_trade_sdk::swqos::SwqosClient>
                })
                .collect(),
        ),
        config: sol_trade_sdk::common::InfrastructureConfig::new(
            "memory://sdk-lane-metadata".into(),
            vec![],
            solana_commitment_config::CommitmentConfig::processed(),
        ),
        max_sender_concurrency: 2,
        effective_core_ids: Arc::new(vec![]),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Prepare,
    Invoke,
    Observe,
}

#[derive(Clone)]
enum Reply {
    Report(bool, Vec<String>, Option<String>),
    Failure(Vec<String>),
}

#[derive(Clone)]
enum Observation {
    Skip,
    Missing,
    Seen(ChainObservation),
    Error,
}

struct Lane {
    provider: SwqosType,
    events: Arc<Mutex<Vec<(SwqosType, Stage)>>>,
    prepare_error: bool,
    reply: Reply,
    observation: Observation,
    block: Option<Stage>,
    dropped: Arc<AtomicBool>,
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Lane {
    fn new(provider: SwqosType, events: Arc<Mutex<Vec<(SwqosType, Stage)>>>) -> Self {
        Self {
            provider,
            events,
            prepare_error: false,
            reply: Reply::Report(true, vec![signature(1)], None),
            observation: Observation::Skip,
            block: None,
            dropped: Arc::default(),
        }
    }
    async fn stage(&self, stage: Stage) {
        self.events.lock().unwrap().push((self.provider, stage));
        if self.block == Some(stage) {
            let _drop = DropFlag(self.dropped.clone());
            std::future::pending::<()>().await;
        }
    }
}

impl OrdinarySdkLane for Lane {
    type Prepared = ();
    fn provider(&self) -> SwqosType {
        self.provider
    }
    async fn prepare(&self, _: &ExecuteTrade) -> Result<()> {
        self.stage(Stage::Prepare).await;
        if self.prepare_error {
            bail!("prepare https://fixture.invalid/?api-key=secret failed");
        }
        Ok(())
    }
    async fn invoke(&self, _: ()) -> Result<SdkLaneReport, SdkInvocationFailure> {
        self.stage(Stage::Invoke).await;
        match &self.reply {
            Reply::Report(success, signatures, error) => Ok(SdkLaneReport {
                success: *success,
                signatures: signatures.clone(),
                error: error.clone(),
            }),
            Reply::Failure(signatures) => Err(SdkInvocationFailure {
                signatures: signatures.clone(),
                message: "opaque https://fixture.invalid/?key=secret error".into(),
            }),
        }
    }
    async fn observe(&self, signature_text: &str, _: &ExecuteTrade) -> Result<SdkLaneObservation> {
        assert_eq!(signature_text, signature(1));
        self.stage(Stage::Observe).await;
        Ok(match &self.observation {
            Observation::Skip => SdkLaneObservation::Skipped,
            Observation::Missing => SdkLaneObservation::NotVisible,
            Observation::Seen(observation) => SdkLaneObservation::Observed(observation.clone()),
            Observation::Error => bail!("RPC https://fixture.invalid/?key=secret unavailable"),
        })
    }
}

fn signature(n: u8) -> String {
    Signature::from([n; 64]).to_string()
}
fn trade() -> ExecuteTrade {
    ExecuteTrade {
        pool: Pool::Pump,
        mint: Pubkey::new_from_array([1; 32]),
        quote_mint: Pubkey::new_from_array([2; 32]),
        pool_id: None,
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(1_000_000),
        sell_percent: Some(100),
        slippage_bps: 1_500,
        tip_sol: 0.0001,
        recent_blockhash: Hash::new_from_array([3; 32]),
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 10,
        simulate: false,
        use_mev: None,
        mev_provider: None,
    }
}
fn seen(commitment: ChainCommitment) -> ChainObservation {
    ChainObservation {
        signature: signature(1),
        slot: 12,
        commitment,
        error: None,
    }
}

#[tokio::test]
async fn only_preparation_failure_can_select_a_new_lane_and_notes_are_redacted() {
    let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
    let mut first = Lane::new(SwqosType::Temporal, events.clone());
    first.prepare_error = true;
    let second = Lane::new(SwqosType::Default, events.clone());
    let (status, sig, error) = classify_result(execute([first, second], &trade()).await, false);
    assert_eq!(status, TradeStatus::Submitted);
    assert_eq!(sig, Some(signature(1)));
    let error = error.unwrap();
    assert!(error.starts_with("fallback_notes=[Temporal lane skipped:"));
    assert!(!error.contains("secret"));
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            (SwqosType::Temporal, Stage::Prepare),
            (SwqosType::Default, Stage::Prepare),
            (SwqosType::Default, Stage::Invoke),
            (SwqosType::Default, Stage::Observe)
        ]
    );
}

#[tokio::test]
async fn empty_or_all_unprepared_lanes_never_invoke_and_remain_ordinary_failures() {
    let (status, sig, error) = classify_result(execute(Vec::<Lane>::new(), &trade()).await, false);
    assert_eq!(status, TradeStatus::Failed);
    assert_eq!(sig, None);
    assert_eq!(error.as_deref(), Some("no RPC lanes configured"));
    let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
    let mut a = Lane::new(SwqosType::Temporal, events.clone());
    let mut b = Lane::new(SwqosType::Default, events.clone());
    a.prepare_error = true;
    b.prepare_error = true;
    let (status, _, error) = classify_result(execute([a, b], &trade()).await, false);
    assert_eq!(status, TradeStatus::Failed);
    assert!(!error.unwrap().contains("secret"));
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .all(|(_, s)| *s == Stage::Prepare));
}

#[tokio::test]
async fn every_uncertain_sdk_receipt_stops_before_the_second_lane() {
    let cases = [
        Reply::Failure(vec![]),
        Reply::Failure(vec![signature(1)]),
        Reply::Report(false, vec![], None),
        Reply::Report(
            false,
            vec![signature(1)],
            Some("sendTransaction failed".into()),
        ),
        Reply::Report(true, vec![], None),
        Reply::Report(true, vec![signature(1), signature(2)], None),
        Reply::Report(
            false,
            vec![signature(1), signature(2)],
            Some("program failure".into()),
        ),
    ];
    for reply in cases {
        let expected = match &reply {
            Reply::Failure(s) | Reply::Report(_, s, _) => s.first().cloned(),
        };
        let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
        let mut a = Lane::new(SwqosType::Temporal, events.clone());
        a.reply = reply;
        let b = Lane::new(SwqosType::Default, events.clone());
        let (status, sig, error) = classify_result(execute([a, b], &trade()).await, false);
        assert_eq!(status, TradeStatus::Unknown);
        assert_eq!(sig, expected);
        assert!(!error.as_ref().unwrap().contains("secret"));
        if error.as_ref().unwrap().contains("multiple") {
            assert!(error.unwrap().contains(&signature(2)));
        }
        assert_eq!(
            *events.lock().unwrap(),
            [
                (SwqosType::Temporal, Stage::Prepare),
                (SwqosType::Temporal, Stage::Invoke)
            ]
        );
    }
}

#[tokio::test]
async fn failed_or_missing_observation_keeps_the_original_signature_without_retry() {
    let mut onchain_error = seen(ChainCommitment::Finalized);
    onchain_error.error = Some("synthetic transaction error".into());
    let mut wrong_signature = seen(ChainCommitment::Confirmed);
    wrong_signature.signature = signature(2);
    let mut zero_slot = seen(ChainCommitment::Confirmed);
    zero_slot.slot = 0;
    for observation in [
        Observation::Missing,
        Observation::Error,
        Observation::Seen(onchain_error),
        Observation::Seen(wrong_signature),
        Observation::Seen(zero_slot),
    ] {
        let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
        let mut a = Lane::new(SwqosType::Temporal, events.clone());
        a.observation = observation;
        let b = Lane::new(SwqosType::Default, events.clone());
        let (status, sig, error) = classify_result(execute([a, b], &trade()).await, false);
        assert_eq!(status, TradeStatus::Unknown);
        assert_eq!(sig, Some(signature(1)));
        assert!(!error.unwrap().contains("secret"));
        assert_eq!(
            *events.lock().unwrap(),
            [
                (SwqosType::Temporal, Stage::Prepare),
                (SwqosType::Temporal, Stage::Invoke),
                (SwqosType::Temporal, Stage::Observe)
            ]
        );
    }
}

#[tokio::test]
async fn successful_receipt_and_any_observed_commitment_remain_submitted_not_finalized() {
    for observation in [
        Observation::Skip,
        Observation::Seen(seen(ChainCommitment::Processed)),
        Observation::Seen(seen(ChainCommitment::Confirmed)),
        Observation::Seen(seen(ChainCommitment::Finalized)),
    ] {
        let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
        let mut a = Lane::new(SwqosType::Temporal, events.clone());
        a.observation = observation;
        let b = Lane::new(SwqosType::Default, events.clone());
        let (status, sig, error) = classify_result(execute([a, b], &trade()).await, false);
        assert_eq!(status, TradeStatus::Submitted);
        assert_eq!(sig, Some(signature(1)));
        assert_eq!(error, None);
        assert_eq!(events.lock().unwrap().len(), 3);
    }
}

#[tokio::test]
async fn simulation_never_falls_back_after_invoke_or_reads_chain_status() {
    let mut trade = trade();
    trade.simulate = true;
    for reply in [
        Reply::Report(true, vec![signature(1)], None),
        Reply::Report(false, vec![], Some("simulation failed".into())),
        Reply::Failure(vec![signature(1), signature(2)]),
    ] {
        let success = matches!(&reply, Reply::Report(true, _, _));
        let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
        let mut a = Lane::new(SwqosType::Temporal, events.clone());
        a.reply = reply;
        let b = Lane::new(SwqosType::Default, events.clone());
        let (status, sig, error) = classify_result(execute([a, b], &trade).await, true);
        assert_eq!(
            status,
            if success {
                TradeStatus::Simulated
            } else if sig.is_some() {
                TradeStatus::Unknown
            } else {
                TradeStatus::Failed
            }
        );
        if error
            .as_ref()
            .is_some_and(|e| e.contains("known SDK signatures"))
        {
            assert!(error.unwrap().contains(&signature(2)));
        }
        assert_eq!(events.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn cancelling_local_preparation_invocation_or_observation_never_enters_another_lane() {
    for stage in [Stage::Prepare, Stage::Invoke, Stage::Observe] {
        let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
        let mut a = Lane::new(SwqosType::Temporal, events.clone());
        a.block = Some(stage);
        let dropped = a.dropped.clone();
        let b = Lane::new(SwqosType::Default, events.clone());
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(20),
            execute([a, b], &trade())
        )
        .await
        .is_err());
        assert!(dropped.load(Ordering::SeqCst));
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .all(|(lane, _)| *lane == SwqosType::Temporal));
        assert_eq!(events.lock().unwrap().last().unwrap().1, stage);
    }
}

#[test]
fn sender_guard_rejects_hidden_default_empty_multiple_wrong_provider_and_middleware() {
    for provider in [SwqosType::Default, SwqosType::Temporal, SwqosType::Jito] {
        assert!(ensure_sender_layout(provider, &[provider], false).is_ok());
        assert!(ensure_sender_layout(provider, &[provider], true).is_err());
        for actual in [
            vec![],
            vec![provider, provider],
            vec![SwqosType::Helius],
            vec![provider, SwqosType::Default],
        ] {
            assert!(ensure_sender_layout(provider, &actual, false).is_err());
        }
    }
}

#[test]
fn selection_handles_sdk_appended_default_without_accepting_hidden_fallback_or_fanout() {
    for expected in [SwqosType::Temporal, SwqosType::Jito] {
        assert_eq!(
            selected_sender_index(expected, &[expected], false).unwrap(),
            0
        );
        assert_eq!(
            selected_sender_index(expected, &[expected, SwqosType::Default], false).unwrap(),
            0
        );
        assert_eq!(
            selected_sender_index(expected, &[SwqosType::Default, expected], false).unwrap(),
            1
        );
        for actual in [
            vec![],
            vec![SwqosType::Default],
            vec![expected, expected],
            vec![expected, SwqosType::Helius],
            vec![expected, SwqosType::Default, SwqosType::Default],
        ] {
            assert!(selected_sender_index(expected, &actual, false).is_err());
        }
        assert!(selected_sender_index(expected, &[expected, SwqosType::Default], true).is_err());
    }
    assert_eq!(
        selected_sender_index(SwqosType::Default, &[SwqosType::Default], false).unwrap(),
        0
    );
    assert!(selected_sender_index(
        SwqosType::Default,
        &[SwqosType::Temporal, SwqosType::Default],
        false
    )
    .is_err());
}

#[test]
fn execution_projection_isolates_all_actual_providers_and_reuses_exact_resources() {
    for provider in SwqosType::values() {
        let providers = if provider == SwqosType::Default {
            vec![provider]
        } else {
            vec![provider, SwqosType::Default]
        };
        let original = infrastructure(&providers);
        let selected = execution_infrastructure(&original, provider, false).unwrap();
        assert_eq!(selected.swqos_clients.len(), 1);
        assert!(Arc::ptr_eq(
            &selected.swqos_clients[0],
            &original.swqos_clients[0]
        ));
        assert!(Arc::ptr_eq(&selected.rpc, &original.rpc));
        assert!(Arc::ptr_eq(
            &selected.effective_core_ids,
            &original.effective_core_ids
        ));
        assert_eq!(selected.config, original.config);
        assert_eq!(
            selected.max_sender_concurrency,
            original.max_sender_concurrency
        );
        assert_eq!(original.swqos_clients.len(), providers.len());
        ensure_sender_layout(
            provider,
            &[selected.swqos_clients[0].get_swqos_type()],
            false,
        )
        .unwrap();
    }
}

#[test]
fn execution_projection_rejects_missing_actual_provider_and_does_not_relabel_default() {
    for providers in [
        vec![],
        vec![SwqosType::Default],
        vec![SwqosType::Temporal, SwqosType::Temporal],
        vec![SwqosType::Temporal, SwqosType::Jito],
    ] {
        let original = infrastructure(&providers);
        assert!(execution_infrastructure(&original, SwqosType::Temporal, false).is_err());
        assert_eq!(original.swqos_clients.len(), providers.len());
    }
    let original = infrastructure(&[SwqosType::Temporal, SwqosType::Default]);
    assert!(execution_infrastructure(&original, SwqosType::Temporal, true).is_err());
    assert_eq!(original.swqos_clients.len(), 2);
}

#[test]
fn fee_guard_checks_actual_provider_side_and_single_normal_strategy() {
    for provider in [SwqosType::Default, SwqosType::Temporal, SwqosType::Jito] {
        for side in [Side::Buy, Side::Sell] {
            for simulate in [false, true] {
                let fees = default_fee_strategy(provider, 0.0001);
                assert!(ensure_single_fee_strategy(&fees, provider, side, simulate).is_ok());
                let target = if simulate {
                    SwqosType::Default
                } else {
                    provider
                };
                let direction = if side == Side::Buy {
                    TradeType::Buy
                } else {
                    TradeType::Sell
                };
                fees.del_all(target, direction);
                assert!(ensure_single_fee_strategy(&fees, provider, side, simulate).is_err());
                fees.set(
                    target,
                    direction,
                    GasFeeStrategyType::LowTipHighCuPrice,
                    300_000,
                    1_000,
                    0.0,
                );
                assert!(ensure_single_fee_strategy(&fees, provider, side, simulate).is_err());
                fees.set(
                    target,
                    direction,
                    GasFeeStrategyType::HighTipLowCuPrice,
                    300_000,
                    1_000,
                    0.0,
                );
                assert!(ensure_single_fee_strategy(&fees, provider, side, simulate).is_err());
            }
        }
    }
}

#[test]
fn sdk_decoder_preserves_unique_signatures_filters_zero_placeholders_and_redacts_errors() {
    let report = decode_sdk_report(Ok((
        false,
        vec![
            Signature::default(),
            Signature::from([1; 64]),
            Signature::from([1; 64]),
            Signature::from([2; 64]),
        ],
        None,
        vec![],
    )))
    .ok()
    .unwrap();
    assert!(!report.success);
    assert_eq!(report.signatures, [signature(1), signature(2)]);
    let report = decode_sdk_report(Ok((
        false,
        vec![],
        Some(sol_trade_sdk::swqos::common::TradeError {
            code: 500,
            message: "https://fixture.invalid/?key=secret".into(),
            instruction: None,
        }),
        vec![],
    )));
    // Transport errors and SDK report errors both remain diagnostic, never permission to retry.
    assert!(!report.ok().unwrap().error.unwrap().contains("secret"));
    let failure = decode_sdk_report(Err(anyhow::anyhow!(
        "RPC https://fixture.invalid/?key=secret failed"
    )))
    .err()
    .unwrap();
    assert!(failure.signatures.is_empty());
    assert!(!failure.message.contains("secret"));
}
