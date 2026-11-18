use super::*;
use crate::adapters::rpc::damm_sell_simulation::shared::{
    tests::{self as fixture, Mode, URL},
    DammSimulationReader,
};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn damm_candidate_adapter_buy_simulation_matches_original_packet_without_sdk_signing() {
    for native in [false, true] {
        for exists in [false, true] {
            let p = profile(SwqosType::Temporal, true, native, exists).await;
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
}

#[tokio::test]
async fn damm_candidate_adapter_buy_live_preview_preserves_zero_signature_message_and_minimum_slot()
{
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        let p = profile(provider, false, true, false).await;
        let candidate = p
            .prepare_live((provider != SwqosType::Default).then(|| tip_accounts(provider)[0]))
            .unwrap();
        let before = candidate.message_bytes();
        let old = MemorySimulation::new();
        candidate
            .validate_unsigned_live(&old.client())
            .await
            .unwrap();
        let r = fixture::runtime(fixture::result(), Mode::Good);
        let reader = DammSimulationReader::new(r.as_ref(), URL.into());
        candidate.validate_unsigned_live(&reader).await.unwrap();
        assert_eq!(before, candidate.message_bytes());
        assert_eq!(candidate.transaction.signatures, [Signature::default()]);
        assert_eq!(
            *old.calls.lock().unwrap(),
            [r.test_transport().calls.lock().unwrap()[0].1["params"].clone()]
        );
    }
}

#[tokio::test]
async fn damm_candidate_adapter_buy_live_preview_chain_transport_and_old_bank_fail_closed() {
    for case in 0..3 {
        let p = profile(SwqosType::Temporal, false, true, false).await;
        let candidate = p
            .prepare_live(Some(tip_accounts(SwqosType::Temporal)[0]))
            .unwrap();
        let mut response = fixture::result();
        if case == 0 {
            response["context"]["slot"] = json!(42);
        }
        if case == 1 {
            response["value"]["err"] = json!({"InstructionError":[2,{"Custom":6001}]});
        }
        let r = fixture::runtime(response, if case == 2 { Mode::Error } else { Mode::Good });
        let reader = DammSimulationReader::new(r.as_ref(), URL.into());
        assert!(candidate.validate_unsigned_live(&reader).await.is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        assert_eq!(candidate.transaction.signatures, [Signature::default()]);
    }
}

#[tokio::test]
async fn damm_candidate_adapter_buy_slow_simulation_and_live_preview_cannot_renew_quote() {
    for simulate in [false, true] {
        let p = profile(SwqosType::Temporal, simulate, true, false).await;
        let r = fixture::runtime(fixture::result(), Mode::Delay(Duration::from_millis(3100)));
        let reader = DammSimulationReader::new(r.as_ref(), URL.into());
        let error = if simulate {
            p.execute(&reader, |_| async { panic!("no SDK") })
                .await
                .err()
                .unwrap()
                .message
        } else {
            let candidate = p
                .prepare_live(Some(tip_accounts(SwqosType::Temporal)[0]))
                .unwrap();
            candidate
                .validate_unsigned_live(&reader)
                .await
                .err()
                .unwrap()
                .to_string()
        };
        assert!(error.contains("expired"));
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}
