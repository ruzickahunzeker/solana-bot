use super::*;
use crate::adapters::rpc::wsol_quote_reads::tests::{key, runtime, urls, Fake, Fixture, Mode};
use crate::application::wsol_failure::{WsolFailureKind, WsolFailureScope, WsolFailureStage};
use base64::Engine;
use serde_json::{json, Value};
use solana_sdk::message::Message;
use std::sync::Arc;
async fn inspect(
    r: &Arc<SharedReadRuntime<Fake>>,
    f: &Fixture,
    input: &WsolQuoteInput,
) -> Result<WsolQuoteEstimate> {
    estimate(r, &urls()[0], CommitmentConfig::confirmed(), f.owner, input).await
}
fn fields(v: &WsolQuoteEstimate) -> (u64, u64, bool, u64, u64, u64) {
    (
        v.amount_lamports,
        v.source_balance_lamports,
        v.ata_exists,
        v.estimated_network_fee_lamports,
        v.temporary_rent_lamports,
        v.rent_refund_lamports,
    )
}
fn normalize(method: &str, mut params: Value) -> Value {
    if method == "getAccountInfo" {
        params[1]["encoding"] = json!("base64");
    }
    if method == "getFeeForMessage" {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(params[0].as_str().unwrap())
            .unwrap();
        let mut message: Message = bincode::deserialize(&bytes).unwrap();
        if message.header.num_required_signatures == 2 {
            message.account_keys[1] = key(90);
        }
        params[0] = json!(base64::engine::general_purpose::STANDARD.encode(message.serialize()));
    }
    json!([method, params])
}
#[tokio::test]
async fn wsol_quote_loader_four_paths_match_original_reports_and_normalized_rpc_witness() {
    for (direction, token, all) in [
        (WsolDirection::Wrap, None, false),
        (WsolDirection::Wrap, Some(2_000_000), false),
        (WsolDirection::Unwrap, Some(2_000_000), true),
        (WsolDirection::Unwrap, Some(2_000_000), false),
    ] {
        let mut f = Fixture::default();
        f.token = token;
        f.fee = Some(if direction == WsolDirection::Unwrap && !all {
            12_345
        } else {
            5000
        });
        let input = Fixture::input(direction, if all { None } else { Some(1_000_000) }, all);
        let old = f.legacy();
        let expected =
            crate::sdk_client::wsol_conversion::queries::estimate(f.owner, &old.client(), &input)
                .await
                .unwrap();
        let r = runtime(f.clone(), Mode::Good);
        let actual = inspect(&r, &f, &input).await.unwrap();
        assert_eq!(fields(&actual), fields(&expected));
        actual.validate_for(&input).unwrap();
        let old = old
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(m, p)| normalize(m, p.clone()))
            .collect::<Vec<_>>();
        let new = r
            .test_transport()
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, v, _)| normalize(v["method"].as_str().unwrap(), v["params"].clone()))
            .collect::<Vec<_>>();
        assert_eq!(old, new);
        assert_eq!(new.len(), 5);
        assert_eq!(actual.estimated_network_fee_lamports, f.fee.unwrap());
    }
}
#[tokio::test]
async fn wsol_quote_loader_original_identity_native_and_amount_failures_keep_typed_errors() {
    for corrupt in 1..=5 {
        let mut f = Fixture::default();
        f.corrupt = corrupt;
        let input = Fixture::input(WsolDirection::Unwrap, Some(1_000_000), false);
        let old = crate::sdk_client::wsol_conversion::queries::estimate(
            f.owner,
            &f.legacy().client(),
            &input,
        )
        .await
        .unwrap_err();
        let r = runtime(f.clone(), Mode::Good);
        let new = inspect(&r, &f, &input).await.unwrap_err();
        assert_eq!(old.to_string(), new.to_string());
        assert_eq!(
            new.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Integrity
        );
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}
#[tokio::test]
async fn wsol_quote_loader_exact_sol_budget_and_overflow_match_original_boundaries() {
    for (sol, fee, ok) in [
        (3_144_280, 5000, true),
        (3_144_279, 5000, false),
        (u64::MAX, u64::MAX, false),
    ] {
        let mut f = Fixture::default();
        f.sol = sol;
        f.token = None;
        f.fee = Some(fee);
        let input = Fixture::input(WsolDirection::Wrap, Some(1_000_000), false);
        let old = crate::sdk_client::wsol_conversion::queries::estimate(
            f.owner,
            &f.legacy().client(),
            &input,
        )
        .await;
        let r = runtime(f.clone(), Mode::Good);
        let new = inspect(&r, &f, &input).await;
        assert_eq!(old.is_ok(), ok);
        assert_eq!(new.is_ok(), ok);
        if ok {
            assert_eq!(fields(&old.unwrap()), fields(&new.unwrap()));
        } else {
            let (old, new) = (old.unwrap_err(), new.unwrap_err());
            assert_eq!(old.to_string(), new.to_string());
            assert_eq!(
                new.downcast_ref::<WsolFailure>().unwrap().kind,
                WsolFailureKind::Rejected
            );
        }
    }
}
#[tokio::test]
async fn wsol_quote_loader_unavailable_fee_and_transport_never_become_zero_cost_success() {
    let input = Fixture::input(WsolDirection::Wrap, Some(1_000_000), false);
    for mode in [
        Mode::Good,
        Mode::WrongId(0),
        Mode::Oversize(0),
        Mode::Error(0),
    ] {
        let mut f = Fixture::default();
        f.fee = None;
        let r = runtime(f.clone(), mode);
        let err = inspect(&r, &f, &input).await.unwrap_err();
        let mut scope = WsolFailureScope::new();
        scope.stage(WsolFailureStage::QuoteEstimate);
        let classified = scope.classify(err);
        assert_eq!(
            classified.downcast_ref::<WsolFailure>().unwrap().kind,
            WsolFailureKind::Unavailable
        );
    }
}
#[tokio::test]
async fn wsol_quote_loader_partial_unwrap_queries_public_two_signer_message_only() {
    let f = Fixture::default();
    let r = runtime(f.clone(), Mode::Good);
    let input = Fixture::input(WsolDirection::Unwrap, Some(1_000_000), false);
    let result = inspect(&r, &f, &input).await.unwrap();
    assert_eq!(result.temporary_rent_lamports, f.rent);
    assert_eq!(result.rent_refund_lamports, 0);
    let calls = r.test_transport().calls.lock().unwrap();
    let message: Message = bincode::deserialize(
        &base64::engine::general_purpose::STANDARD
            .decode(calls[4].1["params"][0].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(message.header.num_required_signatures, 2);
    assert_eq!(message.account_keys[0], f.owner);
    assert_ne!(message.account_keys[1], f.owner);
    assert_ne!(message.account_keys[1], f.ata());
    assert_eq!(message.instructions.len(), 4);
    assert!(calls
        .iter()
        .all(|(_, v, _)| v["method"] != "sendTransaction" && v["method"] != "simulateTransaction"));
}
#[tokio::test]
async fn wsol_quote_loader_raw_u64_balance_and_explicit_zero_fee_rent_are_not_rounded() {
    for direction in [WsolDirection::Wrap, WsolDirection::Unwrap] {
        let mut f = Fixture::default();
        f.sol = u64::MAX;
        f.rent = 0;
        f.fee = Some(0);
        f.token = if direction == WsolDirection::Wrap {
            None
        } else {
            Some(u64::MAX)
        };
        let input = Fixture::input(
            direction,
            if direction == WsolDirection::Wrap {
                Some(1)
            } else {
                None
            },
            direction == WsolDirection::Unwrap,
        );
        let r = runtime(f.clone(), Mode::Good);
        let actual = inspect(&r, &f, &input).await.unwrap();
        let old = crate::sdk_client::wsol_conversion::queries::estimate(
            f.owner,
            &f.legacy().client(),
            &input,
        )
        .await
        .unwrap();
        assert_eq!(fields(&actual), fields(&old));
        assert_eq!(actual.source_balance_lamports, u64::MAX);
        assert_eq!(
            (
                actual.estimated_network_fee_lamports,
                actual.temporary_rent_lamports,
                actual.rent_refund_lamports
            ),
            (0, 0, 0)
        );
    }
}
#[tokio::test]
async fn wsol_quote_loader_rejections_stop_at_original_read_and_release_scope() {
    let f = Fixture::default();
    for (direction, amount, expected_calls) in [
        (WsolDirection::Wrap, None, 2),
        (WsolDirection::Wrap, Some(0), 3),
        (WsolDirection::Unwrap, Some(2_000_001), 3),
    ] {
        let input = Fixture::input(direction, amount, false);
        let r = runtime(f.clone(), Mode::Good);
        let new = inspect(&r, &f, &input).await.unwrap_err();
        let old = crate::sdk_client::wsol_conversion::queries::estimate(
            f.owner,
            &f.legacy().client(),
            &input,
        )
        .await
        .unwrap_err();
        assert_eq!(new.to_string(), old.to_string());
        assert_eq!(
            r.test_transport().calls.lock().unwrap().len(),
            expected_calls
        );
        assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
    }
}
