use super::*;
use crate::adapters::rpc::damm_sell_simulation::shared::{
    tests::{self as fixture, Mode, URL},
    DammSimulationReader,
};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn damm_candidate_adapter_sell_simulation_matches_oracle_without_entering_sdk() {
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        let p = super::super::super::tests::fixture(provider, true, 50)
            .profile()
            .unwrap();
        let old = MemorySimulation::new();
        let expected = p
            .execute(&old.client(), |_| async { panic!("no SDK") })
            .await
            .unwrap_or_else(|e| panic!("{}", e.message));
        let r = fixture::runtime(fixture::result(), Mode::Good);
        let reader = DammSimulationReader::new(r.as_ref(), URL.into());
        let actual = p
            .execute(&reader, |_| async { panic!("no SDK") })
            .await
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(
            (actual.success, actual.signatures, actual.error),
            (expected.success, expected.signatures, expected.error)
        );
        assert_eq!(
            *old.calls.lock().unwrap(),
            [r.test_transport().calls.lock().unwrap()[0].1["params"].clone()]
        );
    }
}

#[tokio::test]
async fn damm_candidate_adapter_sell_preview_finishes_before_original_submit_and_rejects_bad_bank()
{
    for good in [false, true] {
        let (submit, preview) = pair();
        let mut response = fixture::result();
        if !good {
            response["context"]["slot"] = json!(122);
        }
        let r = fixture::runtime(response, Mode::Good);
        let reader = DammSimulationReader::new(r.as_ref(), URL.into());
        let called = AtomicUsize::new(0);
        let result = submit
            .after_unsigned_preview(&preview, &reader, || async {
                assert!(good);
                assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
                called.fetch_add(1, Ordering::SeqCst);
                fake_result()
            })
            .await;
        assert_eq!(result.is_ok(), good);
        assert_eq!(called.load(Ordering::SeqCst), usize::from(good));
        if let Err(error) = result {
            assert!(error.signatures.is_empty());
        }
    }
}

#[tokio::test]
async fn damm_candidate_adapter_sell_preview_chain_and_transport_failure_never_releases_submit() {
    for fail in [false, true] {
        let (submit, preview) = pair();
        let mut response = fixture::result();
        response["value"]["err"] = json!({"InstructionError":[1,{"Custom":6002}]});
        let r = fixture::runtime(response, if fail { Mode::Error } else { Mode::Good });
        let reader = DammSimulationReader::new(r.as_ref(), URL.into());
        let error = submit
            .after_unsigned_preview(&preview, &reader, || async { panic!("must not submit") })
            .await
            .err()
            .unwrap();
        assert!(error.signatures.is_empty() && !error.message.contains("private"));
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn damm_candidate_adapter_sell_slow_shared_preview_cannot_renew_balance_or_release_submit() {
    let (submit, preview) = pair();
    let r = fixture::runtime(fixture::result(), Mode::Delay(Duration::from_millis(3100)));
    let reader = DammSimulationReader::new(r.as_ref(), URL.into());
    let error = submit
        .after_unsigned_preview(&preview, &reader, || async {
            panic!("expired balance must not submit")
        })
        .await
        .err()
        .unwrap();
    assert!(error.signatures.is_empty() && error.message.contains("expired"));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}
