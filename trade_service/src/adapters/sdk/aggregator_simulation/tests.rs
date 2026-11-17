use super::*;
use crate::adapters::rpc::aggregator_simulation::tests::{
    fixture, key, runtime, urls, Fake, Fixture, Mode,
};
use base64::Engine;
use serde_json::{json, Value};
use std::sync::Arc;

async fn inspect(r: &Arc<SharedReadRuntime<Fake>>, f: &Fixture) -> Result<AggregatorSimulation> {
    simulate(
        r,
        &urls()[0],
        f.wallet,
        &f.transaction,
        f.target_index,
        f.mint,
        f.target,
        f.program,
        &f.keys,
    )
    .await
}

#[tokio::test]
async fn aggregator_sim_loader_legacy_v0_and_both_token_programs_keep_original_report() {
    for v0 in [false, true] {
        for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
            let f = fixture(program, v0);
            let r = runtime(f.baseline(), f.simulation(), Mode::Good);
            let report = inspect(&r, &f).await.unwrap();
            assert!(report.ok);
            assert_eq!(
                (
                    report.fee_lamports,
                    report.units_consumed,
                    report.payer_debit_lamports
                ),
                (Some(5000), Some(1234), Some(10_000))
            );
            assert_eq!(
                (
                    report.target_token_delta_units,
                    report.target_token_post_units
                ),
                (Some(20), Some(30))
            );
            assert_eq!((report.baseline_slot, report.simulation_slot), (42, 42));
            assert_eq!(
                (
                    report.observed_same_slot_payer_debit_lamports,
                    report.observed_same_slot_target_token_delta_units
                ),
                (Some(10_000), Some(20))
            );
            assert_eq!(
                (
                    report.inner_instruction_groups,
                    report.inner_instruction_count,
                    report.inner_programs,
                    report.inner_payer_system_spend_lamports
                ),
                (Some(0), Some(0), Some(vec![]), Some(0))
            );
            let calls = r.test_transport().calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[1].1["params"][1]["minContextSlot"], 42);
        }
    }
}

#[tokio::test]
async fn aggregator_sim_loader_bad_payer_or_target_baseline_stops_before_simulation() {
    let f = fixture(spl_token_interface::ID, false);
    for damage in 0..5 {
        let mut baseline = f.baseline();
        match damage {
            0 => baseline["value"][0] = Value::Null,
            1 => baseline["value"][1]["owner"] = json!(key(99).to_string()),
            2 => {
                let mut data = base64::engine::general_purpose::STANDARD
                    .decode(baseline["value"][1]["data"][0].as_str().unwrap())
                    .unwrap();
                data[..32].copy_from_slice(key(99).as_ref());
                baseline["value"][1]["data"][0] =
                    json!(base64::engine::general_purpose::STANDARD.encode(data));
            }
            3 => {
                let mut data = base64::engine::general_purpose::STANDARD
                    .decode(baseline["value"][1]["data"][0].as_str().unwrap())
                    .unwrap();
                data[32..64].copy_from_slice(key(99).as_ref());
                baseline["value"][1]["data"][0] =
                    json!(base64::engine::general_purpose::STANDARD.encode(data));
            }
            _ => {
                let mut data = base64::engine::general_purpose::STANDARD
                    .decode(baseline["value"][1]["data"][0].as_str().unwrap())
                    .unwrap();
                data[108] = 0;
                baseline["value"][1]["data"][0] =
                    json!(base64::engine::general_purpose::STANDARD.encode(data));
            }
        }
        let r = runtime(baseline, f.simulation(), Mode::Good);
        assert!(inspect(&r, &f).await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn aggregator_sim_loader_optional_metadata_and_different_bank_are_not_fabricated() {
    let f = fixture(spl_token_interface::ID, false);
    let mut result = f.simulation();
    result["context"]["slot"] = json!(43);
    for field in [
        "fee",
        "unitsConsumed",
        "preBalances",
        "postBalances",
        "preTokenBalances",
        "postTokenBalances",
        "innerInstructions",
    ] {
        result["value"].as_object_mut().unwrap().remove(field);
    }
    let r = runtime(f.baseline(), result, Mode::Good);
    let report = inspect(&r, &f).await.unwrap();
    assert!(report.ok);
    assert_eq!(
        (
            report.fee_lamports,
            report.units_consumed,
            report.payer_debit_lamports,
            report.target_token_delta_units
        ),
        (None, None, None, None)
    );
    assert_eq!(
        (
            report.observed_same_slot_payer_debit_lamports,
            report.observed_same_slot_target_token_delta_units
        ),
        (None, None)
    );
    assert_eq!(
        (
            report.inner_instruction_groups,
            report.inner_instruction_count,
            report.inner_programs,
            report.inner_payer_system_spend_lamports
        ),
        (None, None, None, None)
    );
    assert_eq!(report.target_token_post_units, Some(30));
}

#[tokio::test]
async fn aggregator_sim_loader_cpi_target_mutation_and_foreign_programs_retain_original_rejections()
{
    let f = fixture(spl_token_interface::ID, false);
    for damage in 0..3 {
        let mut result = f.simulation();
        let group = match damage {
            0 => {
                json!({"index":0,"instructions":[{"program":"spl-token","programId":f.program.to_string(),"parsed":{"type":"closeAccount","info":{"account":f.target.to_string()}}}]})
            }
            1 => {
                json!({"index":0,"instructions":[{"programIdIndex":255,"accounts":[],"data":"","stackHeight":2}]})
            }
            _ => json!({"index":1,"instructions":[]}),
        };
        result["value"]["innerInstructions"] = json!([group]);
        let r = runtime(f.baseline(), result, Mode::Good);
        let error = inspect(&r, &f).await.err().unwrap();
        if damage == 0 {
            assert!(error
                .to_string()
                .contains("parsed token instruction references target ATA unexpectedly"));
        }
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn aggregator_sim_loader_instruction_error_is_non_executable_report_without_a_signature() {
    let f = fixture(spl_token_interface::ID, false);
    let mut result = f.simulation();
    result["value"]["err"] = json!({"InstructionError":[0,{"Custom":1}]});
    let r = runtime(f.baseline(), result, Mode::Good);
    let report = inspect(&r, &f).await.unwrap();
    assert!(!report.ok);
    assert_eq!(report.fee_lamports, Some(5000));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    assert!(r
        .test_transport()
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(_, body, _)| body["method"] != "sendTransaction"));
}

#[tokio::test]
async fn aggregator_sim_loader_bad_post_account_or_delta_never_becomes_success() {
    let f = fixture(spl_token_interface::ID, false);
    for damage in 0..3 {
        let mut result = f.simulation();
        match damage {
            0 => result["value"]["accounts"][1]["owner"] = json!(key(99).to_string()),
            1 => {
                let mut data = base64::engine::general_purpose::STANDARD
                    .decode(result["value"]["accounts"][1]["data"][0].as_str().unwrap())
                    .unwrap();
                data[..32].copy_from_slice(key(99).as_ref());
                result["value"]["accounts"][1]["data"][0] =
                    json!(base64::engine::general_purpose::STANDARD.encode(data));
            }
            _ => result["value"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] = json!("100"),
        }
        let r = runtime(f.baseline(), result, Mode::Good);
        assert!(inspect(&r, &f).await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn aggregator_sim_loader_missing_target_baseline_preserves_existing_zero_and_slot_semantics()
{
    let f = fixture(spl_token_interface::ID, true);
    let mut baseline = f.baseline();
    baseline["value"][1] = Value::Null;
    let mut result = f.simulation();
    result["value"]["preTokenBalances"] = json!([]);
    result["value"]["loadedAddresses"] = f.loaded();
    let r = runtime(baseline, result, Mode::Good);
    let report = inspect(&r, &f).await.unwrap();
    assert_eq!(report.target_token_delta_units, None);
    assert_eq!(report.observed_same_slot_target_token_delta_units, Some(30));
    assert_eq!(report.observed_same_slot_payer_debit_lamports, Some(10_000));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
}
