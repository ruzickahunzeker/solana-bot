use super::*;
use crate::{
    adapters::rpc::damm_quote_reads::{
        tests::{self as reads, Mode, URL},
        DammQuoteReader,
    },
    domain::preparation_balance::PreparationBalanceFailure,
};
use std::time::Duration;

fn runtime(
    case: &Case,
    mode: Mode,
) -> std::sync::Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime<reads::Fake>> {
    reads::runtime(
        json!({"context":{"slot":case.source.rpc.bank},"value":case.source.rpc.values}),
        json!({"context":{"slot":case.source.rpc.simulation_slot},"value":case.result}),
        mode,
    )
}
async fn shared(
    case: &Case,
    r: &crate::adapters::rpc::read_runtime::SharedReadRuntime<reads::Fake>,
) -> Result<BuyOutputEvidence> {
    quote(&DammQuoteReader::new(r, URL.into()), &case.plan, 42).await
}
fn same_failure(before: &anyhow::Error, after: &anyhow::Error) {
    assert_eq!(before.to_string(), after.to_string());
    assert_eq!(
        before
            .downcast_ref::<PreparationBalanceFailure>()
            .map(|f| f.allows_lane_fallback()),
        after
            .downcast_ref::<PreparationBalanceFailure>()
            .map(|f| f.allows_lane_fallback())
    );
}

#[tokio::test]
async fn damm_quote_adapter_buy_original_seventy_two_native_usdc_fee_ata_encodings_match_requests_and_budget(
) {
    let mut count = 0;
    for reversed in [false, true] {
        for native in [false, true] {
            for extended in [false, true] {
                for input_exists in [false, true] {
                    if !native && !input_exists {
                        continue;
                    }
                    for output_exists in [false, true] {
                        for encoding in [Encoding::Parsed, Encoding::Compiled, Encoding::Partial] {
                            let case = Case::new(
                                reversed,
                                native,
                                extended,
                                input_exists,
                                output_exists,
                                encoding,
                            );
                            let expected = case.run().await.unwrap();
                            let r = runtime(&case, Mode::Good);
                            let actual = shared(&case, &r).await.unwrap();
                            assert_eq!(actual.native, expected.native);
                            assert_eq!(actual.message_hash, expected.message_hash);
                            assert_eq!(
                                (
                                    actual.output_units,
                                    actual.context(),
                                    actual.output_initialized(),
                                    actual.wallet_remaining_lamports,
                                    actual.decimals
                                ),
                                (
                                    expected.output_units,
                                    expected.context(),
                                    expected.output_initialized(),
                                    expected.wallet_remaining_lamports,
                                    expected.decimals
                                )
                            );
                            assert_eq!(
                                actual.minimum(&case.plan, 1500).unwrap(),
                                expected.minimum(&case.plan, 1500).unwrap()
                            );
                            let old = case.source.rpc.calls.lock().unwrap();
                            let new = r.test_transport().calls.lock().unwrap();
                            assert_eq!((old.len(), new.len()), (2, 2));
                            for i in 0..2 {
                                assert_eq!(old[i].1, new[i].1["params"]);
                            }
                            assert_eq!(new[0].0, URL);
                            assert_eq!(new[1].0, URL);
                            assert_eq!(case.plan.transaction.signatures, [Signature::default()]);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 72);
}

#[tokio::test]
async fn damm_quote_adapter_buy_missing_same_bank_fee_native_token_or_cpi_evidence_keeps_original_rejection(
) {
    for field in [
        "fee",
        "preBalances",
        "postBalances",
        "preTokenBalances",
        "postTokenBalances",
        "innerInstructions",
        "accounts",
    ] {
        let mut case = Case::default();
        case.result[field] = Value::Null;
        let before = case.run().await.err().unwrap();
        let r = runtime(&case, Mode::Good);
        let after = shared(&case, &r).await.err().unwrap();
        same_failure(&before, &after);
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn damm_quote_adapter_buy_account_slot_identity_and_native_budget_faults_match_original_categories(
) {
    for change in 0..5 {
        let mut case = Case::default();
        match change {
            0 => case.source.rpc.bank = 41,
            1 => case.source.rpc.bank = 0,
            2 => {
                case.source.rpc.values.pop();
            }
            3 => case.source.rpc.values[0]["owner"] = json!(key(99).to_string()),
            _ => case.result["postBalances"][0] = json!(0),
        }
        let before = case.run().await.err().unwrap();
        let r = runtime(&case, Mode::Good);
        let after = shared(&case, &r).await.err().unwrap();
        same_failure(&before, &after);
        assert_eq!(
            r.test_transport().calls.lock().unwrap().len(),
            if change == 4 { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn damm_quote_adapter_buy_original_five_second_whole_quote_limit_cancels_probe_not_renewed_per_step(
) {
    let case = Case::default();
    let r = runtime(&case, Mode::Delay(Duration::from_millis(2600)));
    let start = std::time::Instant::now();
    let error = shared(&case, &r).await.err().unwrap();
    assert!(error.to_string().contains("unsigned quote timed out"));
    assert!(start.elapsed() >= Duration::from_secs(5) && start.elapsed() < Duration::from_secs(7));
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
    assert!(r.parameter_slot().is_ok() && r.aggregator_simulation_slot(URL).is_ok());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn damm_quote_adapter_buy_late_probe_cannot_renew_original_quote_timestamp() {
    let case = Case::default();
    let r = runtime(&case, Mode::DelayProbe(Duration::from_millis(3100)));
    let error = shared(&case, &r).await.err().unwrap();
    assert!(error.to_string().contains("expired"));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn damm_quote_adapter_buy_cancel_probe_releases_shared_permit_and_keeps_hash_independent() {
    let case = Case::default();
    let r = runtime(&case, Mode::HoldProbe);
    {
        let pending = shared(&case, &r);
        tokio::pin!(pending);
        let probe_entered = async {
            loop {
                let notified = r.test_transport().entered.notified();
                if r.test_transport().calls.lock().unwrap().len() == 2 {
                    break;
                }
                notified.await;
            }
        };
        tokio::select! {_=&mut pending=>panic!("held probe returned quote"),_=probe_entered=>{}}
        assert!(r.aggregator_simulation_slot(URL).is_err());
        assert!(r.latest_blockhash().await.is_ok());
    }
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
    assert!(r.aggregator_simulation_slot(URL).is_ok());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 3);
}
