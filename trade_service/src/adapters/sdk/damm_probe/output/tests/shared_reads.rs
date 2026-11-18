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
        json!({"context":{"slot":case.source.rpc.simulation_slot},"value":case.reply}),
        mode,
    )
}
async fn shared(
    case: &Case,
    r: &crate::adapters::rpc::read_runtime::SharedReadRuntime<reads::Fake>,
) -> Result<DammOutputEvidence> {
    let reader = DammQuoteReader::new(r, URL.into());
    let evidence = crate::adapters::sdk::damm_probe::inspect_prepared_damm_probe_at_slot(
        &reader,
        &case.prepared,
        42,
    )
    .await?;
    evidence_damm_probe_output(&evidence).await
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
async fn damm_quote_adapter_sell_original_direction_fee_ata_and_encoding_matrix_matches_evidence_and_requests(
) {
    let mut count = 0;
    for reversed in [false, true] {
        for extended in [false, true] {
            for sell in [false, true] {
                for quote in [WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
                    for exists in [false, true] {
                        for encoding in [Encoding::Compiled, Encoding::Partial, Encoding::Parsed] {
                            let mut case =
                                Case::new(reversed, extended, sell, exists, quote, encoding);
                            case.add_balances();
                            let expected = case.output_evidence().await.unwrap();
                            let r = runtime(&case, Mode::Good);
                            let actual = shared(&case, &r).await.unwrap();
                            assert_eq!(actual.output_units(), expected.output_units());
                            assert_eq!(
                                actual.simulation_slot().unwrap(),
                                expected.simulation_slot().unwrap()
                            );
                            actual.validate_details(&case.candidate_details()).unwrap();
                            let old = case.source.rpc.calls.lock().unwrap();
                            let new = r.test_transport().calls.lock().unwrap();
                            assert_eq!((old.len(), new.len()), (2, 2));
                            for i in 0..2 {
                                assert_eq!(old[i].1, new[i].1["params"]);
                            }
                            assert_eq!(new[0].0, URL);
                            assert_eq!(new[1].0, URL);
                            assert_eq!(new[0].1["method"], "getMultipleAccounts");
                            assert_eq!(new[1].1["method"], "simulateTransaction");
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 96);
}

#[tokio::test]
async fn damm_quote_adapter_sell_account_regression_keeps_terminal_category_not_generic_lane_fallback(
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
            _ => change_token(&mut case.source.rpc.values[6], |a| a.amount = 99),
        }
        let before = case.output_evidence().await.err().unwrap();
        let r = runtime(&case, Mode::Good);
        let after = shared(&case, &r).await.err().unwrap();
        same_failure(&before, &after);
        if change == 0 {
            assert_eq!(
                after
                    .downcast_ref::<PreparationBalanceFailure>()
                    .map(|f| f.allows_lane_fallback()),
                Some(false)
            );
        }
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn damm_quote_adapter_sell_cpi_context_and_same_bank_failures_do_not_release_output() {
    for change in 0..7 {
        let mut case = Case::default();
        case.add_balances();
        match change {
            0 => case.reply["err"] = json!({"InstructionError":[1,"InvalidArgument"]}),
            1 => case.reply["innerInstructions"] = Value::Null,
            2 => case.reply["accounts"] = json!([]),
            3 => case.source.rpc.simulation_slot = 41,
            4 => {
                case.reply["loadedAddresses"] =
                    json!({"writable":[key(99).to_string()],"readonly":[]})
            }
            5 => case.reply["preTokenBalances"] = Value::Null,
            _ => case.reply["postTokenBalances"][0]["uiTokenAmount"]["amount"] = json!("99999"),
        }
        let before = case.output_evidence().await.err().unwrap();
        let r = runtime(&case, Mode::Good);
        let after = shared(&case, &r).await.err().unwrap();
        same_failure(&before, &after);
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn damm_quote_adapter_sell_late_probe_cannot_renew_original_evidence_timestamp() {
    let case = Case::default();
    let r = runtime(&case, Mode::DelayProbe(Duration::from_millis(3100)));
    let error = shared(&case, &r).await.err().unwrap();
    assert!(error.to_string().contains("expired"));
    assert_eq!(
        error
            .downcast_ref::<PreparationBalanceFailure>()
            .map(|f| f.allows_lane_fallback()),
        Some(false)
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn damm_quote_adapter_sell_cancel_probe_releases_existing_permit_without_output_or_retry() {
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
        tokio::select! {_=&mut pending=>panic!("held probe returned evidence"),_=probe_entered=>{}}
        assert!(r.aggregator_simulation_slot(URL).is_err());
        assert!(r.latest_blockhash().await.is_ok());
    }
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
    assert!(r.aggregator_simulation_slot(URL).is_ok());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 3);
}
