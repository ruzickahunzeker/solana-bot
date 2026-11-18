use super::*;
use crate::{
    adapters::rpc::{
        read_runtime::quote_status_fixture as f, signature_status::RpcSignatureStatusReader,
    },
    application::ports::reconciliation::SignatureStatusReader,
};
use serde_json::{json, Value};
fn fields(
    v: Option<ChainObservation>,
) -> Option<(
    String,
    crate::domain::reconciliation::ChainCommitment,
    u64,
    Option<String>,
)> {
    v.map(|v| (v.signature, v.commitment, v.slot, v.error))
}

#[tokio::test]
async fn quote_submit_status_shared_exact_original_request_results_and_two_node_limit() {
    for first in [f::found(), f::value(json!([null])), f::value(json!([]))] {
        let a = f::Legacy::new(f::PRIMARY, first.clone());
        let b = f::Legacy::new(f::BACKUP, f::found());
        let c = f::Legacy::new(f::THIRD, f::found());
        let oracle_a = f::Legacy::new(f::PRIMARY, first.clone());
        let oracle_b = f::Legacy::new(f::BACKUP, f::found());
        let r = f::runtime(vec![
            (f::PRIMARY, vec![f::Reply::Good(first)]),
            (f::BACKUP, vec![f::Reply::Good(f::found())]),
        ]);
        let actual = lookup_selected(
            &r,
            vec![a.client(), b.client(), c.client()],
            &f::signature(),
        )
        .await
        .unwrap();
        let expected = RpcSignatureStatusReader {
            rpcs: vec![oracle_a.client(), oracle_b.client()],
        }
        .lookup_signature(&f::signature())
        .await
        .unwrap();
        assert_eq!(fields(actual), fields(expected));
        for original in [&a, &b, &c] {
            assert!(original.calls.lock().unwrap().is_empty());
        }
        let mut expected_calls = oracle_a.calls.lock().unwrap().clone();
        expected_calls.extend(oracle_b.calls.lock().unwrap().clone());
        assert_eq!(
            r.test_transport()
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.1["params"].clone())
                .collect::<Vec<_>>(),
            expected_calls
        );
        assert!(r
            .test_transport()
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| c.0 != f::THIRD));
        f::released(&r);
    }
}

#[tokio::test]
async fn quote_submit_status_shared_releases_all_execution_rpc_handles_before_io_and_on_cancel() {
    let a = f::Legacy::new(f::PRIMARY, f::found());
    let b = f::Legacy::new(f::BACKUP, f::found());
    let c = f::Legacy::new(f::THIRD, f::found());
    let clients = vec![a.client(), b.client(), c.client()];
    let weak = clients.iter().map(Arc::downgrade).collect::<Vec<_>>();
    let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Hold])]);
    let sig = f::signature();
    let mut pending = Box::pin(lookup_selected(&r, clients, &sig));
    tokio::select! { result=&mut pending=>panic!("unexpected {result:?}"),_=r.test_transport().entered.notified()=>{} }
    assert!(weak.iter().all(|rpc| rpc.upgrade().is_none()));
    for original in [&a, &b, &c] {
        assert!(original.calls.lock().unwrap().is_empty());
    }
    drop(pending);
    f::released(&r);
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn quote_submit_status_shared_original_semantic_rejections_are_terminal_not_fallbacks() {
    for mismatch in [false, true] {
        let mut item = f::status("confirmed", Value::Null);
        if mismatch {
            item["err"] = json!({"InstructionError":[0,{"Custom":42}]});
        } else {
            item.as_object_mut().unwrap().remove("confirmationStatus");
        }
        let value = f::value(json!([item]));
        let a = f::Legacy::new(f::PRIMARY, value.clone());
        let b = f::Legacy::new(f::BACKUP, f::found());
        let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Good(value)])]);
        let actual = lookup_selected(&r, vec![a.client(), b.client()], &f::signature())
            .await
            .unwrap_err();
        let expected = RpcSignatureStatusReader {
            rpcs: vec![a.client(), b.client()],
        }
        .lookup_signature(&f::signature())
        .await
        .unwrap_err();
        assert_eq!(actual.to_string(), expected.to_string());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        assert!(b.calls.lock().unwrap().is_empty());
        f::released(&r);
    }
}

#[tokio::test]
async fn quote_submit_status_shared_actual_backend_remains_readonly_under_disabled_policy_without_wallet(
) {
    use crate::adapters::sdk::quote_submission::SdkQuoteSubmission;
    let client = crate::sdk_client::wsol_conversion::broadcast::tests::disabled_client();
    let r = SharedReadRuntime::new(&[]).unwrap();
    let backend = SdkQuoteSubmission(&client, &r);
    assert_eq!(
        backend
            .lookup_signature("invalid")
            .await
            .unwrap_err()
            .to_string(),
        "invalid persisted transaction signature"
    );
    assert_eq!(
        backend
            .lookup_signature(&f::signature())
            .await
            .unwrap_err()
            .to_string(),
        "no status RPC configured"
    );
}

#[tokio::test]
async fn quote_submit_status_shared_original_scope_blocks_sdk_observer_without_duplicating_quota() {
    let a = f::Legacy::new(f::PRIMARY, f::found());
    let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Hold])]);
    let sig = f::signature();
    let mut pending = Box::pin(lookup_selected(&r, vec![a.client()], &sig));
    tokio::select! { result=&mut pending=>panic!("unexpected {result:?}"),_=r.test_transport().entered.notified()=>{} }
    assert!(
        crate::adapters::rpc::sdk_signature::observe_sdk_signature_shared(&r, f::BACKUP, &sig, 500)
            .await
            .unwrap_err()
            .to_string()
            .contains("quota busy")
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert!(a.calls.lock().unwrap().is_empty());
    drop(pending);
    f::released(&r);
}

#[tokio::test]
async fn quote_submit_status_shared_quote_optional_fields_and_zero_slot_do_not_gain_sdk_restrictions(
) {
    let mut item = f::status("finalized", Value::Null);
    item["slot"] = json!(0);
    item.as_object_mut().unwrap().remove("err");
    let response = f::value(json!([item]));
    let a = f::Legacy::new(f::PRIMARY, response.clone());
    let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Good(response)])]);
    let actual = lookup_selected(&r, vec![a.client()], &f::signature())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual.slot, 0);
    assert_eq!(
        fields(Some(actual)),
        fields(
            RpcSignatureStatusReader {
                rpcs: vec![a.client()]
            }
            .lookup_signature(&f::signature())
            .await
            .unwrap()
        )
    );
    f::released(&r);
}
