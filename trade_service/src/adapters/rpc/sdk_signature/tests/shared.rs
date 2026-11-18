use super::*;
use crate::adapters::rpc::read_runtime::quote_status_fixture as f;
use std::time::Instant;

fn response(value: Value) -> Value {
    json!({"context":{"slot":55},"value":value})
}
fn fields(v: Option<ChainObservation>) -> Option<(String, ChainCommitment, u64, Option<String>)> {
    v.map(|v| (v.signature, v.commitment, v.slot, v.error))
}

#[tokio::test]
async fn sdk_observer_shared_original_finality_and_error_observations_have_exact_history_params() {
    for commitment in ["processed", "confirmed", "finalized"] {
        for failed in [false, true] {
            let mut item = status(commitment);
            if failed {
                item["err"] = json!({"InstructionError":[0,{"Custom":42}]});
                item["status"] = json!({"Err":item["err"]});
            }
            let value = json!([item]);
            let old = MemoryRpc::new(vec![value.clone()]);
            let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Good(response(value))])]);
            let actual = observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), 1000)
                .await
                .unwrap();
            let expected = observe_sdk_signature(old.client(), &signature(), 1000)
                .await
                .unwrap();
            assert_eq!(fields(actual), fields(expected));
            assert_readonly(&old);
            let calls = r.test_transport().calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, f::PRIMARY);
            assert_eq!(calls[0].1["params"], old.calls.lock().unwrap()[0].1);
            drop(calls);
            f::released(&r);
        }
    }
}

#[tokio::test]
async fn sdk_observer_shared_missing_polls_only_original_signature_with_shared_one_second_starts() {
    let old = MemoryRpc::new(vec![json!([null]), json!([status("confirmed")])]);
    let r = f::runtime(vec![(
        f::PRIMARY,
        vec![
            f::Reply::Good(response(json!([null]))),
            f::Reply::Good(response(json!([status("confirmed")]))),
        ],
    )]);
    assert_eq!(
        fields(
            observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), 1800)
                .await
                .unwrap()
        ),
        fields(
            observe_sdk_signature(old.client(), &signature(), 1800)
                .await
                .unwrap()
        )
    );
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_readonly(&old);
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(990));
    assert!(calls.iter().all(|c| c.0 == f::PRIMARY
        && c.1["params"] == json!([[signature()],{"searchTransactionHistory":true}])));
    drop(calls);
    f::released(&r);
}

#[tokio::test]
async fn sdk_observer_shared_short_budget_is_not_extended_to_obtain_an_extra_poll() {
    let old = MemoryRpc::new(vec![json!([null]), json!([status("confirmed")])]);
    assert!(observe_sdk_signature(old.client(), &signature(), 900)
        .await
        .unwrap()
        .is_some());
    let r = f::runtime(vec![(
        f::PRIMARY,
        vec![f::Reply::Good(response(json!([null])))],
    )]);
    let start = Instant::now();
    assert!(
        observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), 900)
            .await
            .unwrap()
            .is_none()
    );
    assert!(start.elapsed() < Duration::from_millis(1600));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    f::released(&r);
}

#[tokio::test]
async fn sdk_observer_shared_zero_budget_invalid_signature_and_large_budget_keep_original_guards() {
    let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Good(f::found())])]);
    assert!(
        observe_sdk_signature_shared(&r, "unregistered", "invalid", 0)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        observe_sdk_signature_shared(&r, f::PRIMARY, "invalid", 500)
            .await
            .unwrap_err()
            .to_string(),
        "invalid persisted transaction signature"
    );
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
    assert!(
        observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), u64::MAX)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    f::released(&r);
}

#[tokio::test]
async fn sdk_observer_shared_original_count_commitment_error_consistency_and_zero_slot_rejections()
{
    let mut no_commitment = status("confirmed");
    no_commitment
        .as_object_mut()
        .unwrap()
        .remove("confirmationStatus");
    let mut zero = status("confirmed");
    zero["slot"] = json!(0);
    let mut mismatch = status("confirmed");
    mismatch["err"] = json!({"InstructionError":[0,{"Custom":42}]});
    for value in [
        json!([]),
        json!([null, null]),
        json!([no_commitment]),
        json!([zero]),
        json!([mismatch]),
    ] {
        let old = MemoryRpc::new(vec![value.clone()]);
        let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Good(response(value))])]);
        assert_eq!(
            observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), 1500)
                .await
                .unwrap_err()
                .to_string(),
            observe_sdk_signature(old.client(), &signature(), 1500)
                .await
                .unwrap_err()
                .to_string()
        );
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        f::released(&r);
    }
}

#[tokio::test]
async fn sdk_observer_shared_node_timeout_remains_an_error_when_caller_budget_is_not_exhausted() {
    let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Hold])]);
    assert_eq!(
        observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), 1800)
            .await
            .unwrap_err()
            .to_string(),
        "signature status RPC timed out"
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    f::released(&r);
}

#[tokio::test]
async fn sdk_observer_shared_rate_or_protocol_fault_never_fails_over_to_other_registered_keys() {
    for reply in [
        f::Reply::Rate,
        f::Reply::Fault(f::Fault::Rpc),
        f::Reply::Fault(f::Fault::Transport),
        f::Reply::Fault(f::Fault::Id),
    ] {
        let r = f::runtime(vec![(f::PRIMARY, vec![reply])]);
        let error = observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), 1500)
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("fixture-secret"));
        let calls = r.test_transport().calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, f::PRIMARY);
        drop(calls);
        f::released(&r);
    }
}

#[tokio::test]
async fn sdk_observer_shared_caller_cancellation_releases_io_and_same_runtime_capacity() {
    let r = f::runtime(vec![(
        f::PRIMARY,
        vec![f::Reply::Hold, f::Reply::Good(f::found())],
    )]);
    let sig = signature();
    drop(observe_sdk_signature_shared(&r, f::PRIMARY, &sig, 10_000));
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
    assert!(tokio::time::timeout(
        Duration::from_millis(30),
        observe_sdk_signature_shared(&r, f::PRIMARY, &sig, 10_000)
    )
    .await
    .is_err());
    f::released(&r);
    assert!(observe_sdk_signature_shared(&r, f::PRIMARY, &sig, 1800)
        .await
        .unwrap()
        .is_some());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    f::released(&r);
}

#[tokio::test]
async fn sdk_observer_shared_budget_expiry_during_pacing_does_not_refund_or_send() {
    let r = f::runtime(vec![(
        f::PRIMARY,
        vec![f::Reply::Good(f::found()), f::Reply::Good(f::found())],
    )]);
    let sig = signature();
    assert!(observe_sdk_signature_shared(&r, f::PRIMARY, &sig, 500)
        .await
        .unwrap()
        .is_some());
    assert!(observe_sdk_signature_shared(&r, f::PRIMARY, &sig, 20)
        .await
        .unwrap()
        .is_none());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    f::released(&r);
    assert!(observe_sdk_signature_shared(&r, f::PRIMARY, &sig, 1800)
        .await
        .unwrap()
        .is_some());
    let calls = r.test_transport().calls.lock().unwrap();
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(990));
    drop(calls);
    f::released(&r);
}

#[tokio::test]
async fn sdk_observer_shared_late_ready_success_or_error_cannot_outlive_original_caller_budget() {
    let mut zero = f::found();
    zero["value"][0]["slot"] = json!(0);
    for value in [f::found(), zero, f::value(json!([null]))] {
        let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Late(700, value)])]);
        assert!(
            observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), 100)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        f::released(&r);
    }
}

#[tokio::test]
async fn sdk_observer_shared_repeated_missing_never_adds_a_request_after_total_budget() {
    let r = f::runtime(vec![(
        f::PRIMARY,
        vec![
            f::Reply::Good(f::value(json!([null]))),
            f::Reply::Good(f::value(json!([null]))),
        ],
    )]);
    assert!(
        observe_sdk_signature_shared(&r, f::PRIMARY, &signature(), 1250)
            .await
            .unwrap()
            .is_none()
    );
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| c.0 == f::PRIMARY));
    drop(calls);
    f::released(&r);
}

#[tokio::test]
async fn sdk_observer_shared_quota_is_shared_with_quote_while_zero_and_invalid_guards_stay_first() {
    let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Hold])]);
    let urls = [f::PRIMARY.to_string()];
    let sig = signature();
    let mut pending = Box::pin(r.quote_signature_status(&urls, &sig));
    tokio::select! { result=&mut pending=>panic!("unexpected {result:?}"),_=r.test_transport().entered.notified()=>{} }
    assert!(observe_sdk_signature_shared(&r, f::BACKUP, &sig, 500)
        .await
        .unwrap_err()
        .to_string()
        .contains("quota busy"));
    assert!(observe_sdk_signature_shared(&r, f::PRIMARY, "invalid", 0)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        observe_sdk_signature_shared(&r, f::PRIMARY, "invalid", 500)
            .await
            .unwrap_err()
            .to_string(),
        "invalid persisted transaction signature"
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    drop(pending);
    f::released(&r);
}
