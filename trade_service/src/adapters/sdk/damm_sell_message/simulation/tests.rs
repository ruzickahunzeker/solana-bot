use super::*;
use crate::{
    adapters::{
        rpc::damm_sell_simulation::tests::MemorySimulation,
        sdk::{
            damm_sell_message::tests::{fixture, key},
            ordinary_params::simulation_fee_strategy,
        },
    },
    domain::preparation_balance::PreparationBalanceSnapshot,
};
use solana_sdk::signature::Signature;
use std::sync::atomic::{AtomicUsize, Ordering};

fn pair() -> (Arc<DammSellMessageProfile>, Arc<DammSellMessageProfile>) {
    let f = fixture(SwqosType::Temporal, false, 50);
    let final_message = f.profile().unwrap();
    let mut intent = f.intent.clone();
    intent.simulate = true;
    let mut request = f.request.clone();
    request.simulate = true;
    request.gas_fee_strategy = simulation_fee_strategy();
    let preview = DammSellMessageProfile::new(
        f.context, &intent, &request, f.balance, f.percent, f.holding, f.quote,
    )
    .unwrap();
    (final_message, preview)
}

#[test]
fn damm_sell_message_final_and_preview_share_the_same_immutable_output_evidence() {
    let (final_message, preview) = pair();
    assert!(Arc::ptr_eq(&final_message.quote, &preview.quote));
}

fn fake_result() -> Result<SdkLaneReport, SdkInvocationFailure> {
    Ok(SdkLaneReport {
        success: true,
        signatures: vec!["fake-final-signature".into()],
        error: None,
    })
}

#[tokio::test]
async fn damm_sell_message_preview_cannot_release_sdk_from_a_bank_before_its_output_quote() {
    let (submit, preview) = pair();
    let mut source = MemorySimulation::new();
    source.response["context"]["slot"] = serde_json::json!(122);
    let error = submit
        .after_unsigned_preview(&preview, &source.client(), || async {
            panic!("older preview bank must not release SDK execution")
        })
        .await
        .err()
        .unwrap();
    assert!(error.signatures.is_empty());
    assert!(error.message.contains("precedes"));
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][1]["minContextSlot"], 123);
}

#[tokio::test]
async fn damm_sell_message_replaced_hash_or_account_context_never_releases_sdk() {
    for replacement_hash in [false, true] {
        let (submit, preview) = pair();
        let mut source = MemorySimulation::new();
        if replacement_hash {
            source.response["value"]["replacementBlockhash"] = serde_json::json!({
                "blockhash":solana_sdk::hash::Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":999,
            });
        } else {
            source.response["value"]["loadedAddresses"] =
                serde_json::json!({"writable":[],"readonly":[key(8).to_string()]});
        }
        let error = submit
            .after_unsigned_preview(&preview, &source.client(), || async {
                panic!("changed preview context must not release SDK execution")
            })
            .await
            .err()
            .unwrap();
        assert!(error.signatures.is_empty());
        assert!(error.message.contains("fixed candidate context"));
        assert_eq!(source.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn damm_sell_message_distinct_quote_instance_cannot_replace_the_original_before_preview() {
    let (submit, mut preview) = pair();
    Arc::get_mut(&mut preview).unwrap().quote = fixture(SwqosType::Temporal, true, 50).quote;
    let source = MemorySimulation::new();
    let error = submit
        .after_unsigned_preview(&preview, &source.client(), || async {
            panic!("a different quote instance must not release SDK execution")
        })
        .await
        .err()
        .unwrap();
    assert!(error.signatures.is_empty());
    assert!(source.calls.lock().unwrap().is_empty());
}

#[test]
fn damm_sell_message_unsigned_candidate_owns_original_message_and_only_a_zero_signature() {
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        for percent in [50, 100] {
            let profile = fixture(provider, true, percent).profile().unwrap();
            let candidate = profile.prepare_simulation().unwrap();
            assert_eq!(candidate.transaction().signatures, [Signature::default()]);
            assert_eq!(
                *candidate.transaction().message.recent_blockhash(),
                profile.intent.recent_blockhash
            );
            let bytes = candidate.message_bytes();
            let mut copy = bytes.clone();
            copy.fill(0);
            assert_eq!(candidate.message_bytes(), bytes);
            assert_ne!(candidate.message_bytes(), copy);
            assert_eq!(
                candidate.transaction().message.static_account_keys()[0],
                profile.context.wallet
            );
        }
    }
}

#[test]
fn damm_sell_message_unsigned_candidate_does_not_accept_a_submit_profile_or_simulation_tip() {
    let submit = fixture(SwqosType::Temporal, false, 50).profile().unwrap();
    assert!(submit.prepare_simulation().is_err());
    let simulation = fixture(SwqosType::Temporal, true, 50).profile().unwrap();
    assert!(DammSellCandidate::build(
        simulation,
        Some(
            crate::adapters::sdk::damm_sell_message::candidate::tip_accounts(SwqosType::Temporal)
                [0]
        )
    )
    .is_err());
}

#[tokio::test]
async fn damm_sell_message_simulation_uses_rpc_and_never_calls_the_sdk_signing_closure() {
    let profile = fixture(SwqosType::Temporal, true, 50).profile().unwrap();
    let source = MemorySimulation::new();
    let calls = AtomicUsize::new(0);
    let report = profile
        .execute(&source.client(), |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            panic!("simulation must not enter the SDK executor")
        })
        .await
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    assert!(report.success);
    assert!(report.signatures.is_empty());
    assert!(report.error.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_sell_message_simulation_chain_failure_returns_no_signature_without_sdk_fallback() {
    let profile = fixture(SwqosType::Temporal, true, 50).profile().unwrap();
    let mut source = MemorySimulation::new();
    source.response["value"]["err"] = serde_json::json!({"InstructionError":[2,{"Custom":6001}]});
    let report = profile
        .execute(&source.client(), |_| async { panic!("no SDK fallback") })
        .await
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    assert!(!report.success);
    assert!(report.signatures.is_empty());
    assert!(report.error.unwrap().contains("6001"));
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_sell_message_old_invoke_cannot_bypass_unsigned_simulation() {
    let profile = fixture(SwqosType::Default, true, 50).profile().unwrap();
    let error = profile
        .invoke(|_| async { panic!("SDK simulation forbidden") })
        .await
        .err()
        .unwrap();
    assert!(error.signatures.is_empty());
    assert!(error.message.contains("unsigned candidate"));
}

#[tokio::test]
async fn damm_sell_message_unsigned_preview_finishes_one_rpc_before_the_submit_closure() {
    let (submit, preview) = pair();
    let source = MemorySimulation::new();
    let sent = AtomicUsize::new(0);
    let result = submit
        .after_unsigned_preview(&preview, &source.client(), || async {
            assert_eq!(source.calls.lock().unwrap().len(), 1);
            sent.fetch_add(1, Ordering::SeqCst);
            fake_result()
        })
        .await
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    assert_eq!(sent.load(Ordering::SeqCst), 1);
    assert_eq!(result.signatures, ["fake-final-signature"]);
}

#[tokio::test]
async fn damm_sell_message_failed_or_unavailable_preview_never_calls_submit() {
    for unavailable in [false, true] {
        let (submit, preview) = pair();
        let mut source = MemorySimulation::new();
        if unavailable {
            source.fail = true;
        } else {
            source.response["value"]["err"] =
                serde_json::json!({"InstructionError":[1,{"Custom":6002}]});
        }
        let error = submit
            .after_unsigned_preview(&preview, &source.client(), || async {
                panic!("failed preview must not submit")
            })
            .await
            .err()
            .unwrap();
        assert!(error.signatures.is_empty());
        assert!(!error.message.contains("simulation-secret"));
        assert_eq!(source.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn damm_sell_message_different_preview_identity_minimum_or_snapshot_cannot_authorize_submit()
{
    for change in 0..4 {
        let (submit, mut preview) = pair();
        let source = MemorySimulation::new();
        let value = Arc::get_mut(&mut preview).unwrap();
        match change {
            0 => value.context.wallet = key(30),
            1 => value.intent.slippage_bps = 1,
            2 => value.protocol[1].data[16] ^= 1,
            3 => value.holding = fixture(SwqosType::Temporal, true, 50).holding,
            _ => unreachable!(),
        }
        let error = submit
            .after_unsigned_preview(&preview, &source.client(), || async {
                panic!("mismatched preview must not submit")
            })
            .await
            .err()
            .unwrap();
        assert!(error.signatures.is_empty());
        assert!(source.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn damm_sell_message_slow_simulation_cannot_refresh_balance_or_release_submit() {
    let (submit, preview) = pair();
    let mut source = MemorySimulation::new();
    source.delay = Some(std::time::Duration::from_millis(3100));
    let error = submit
        .after_unsigned_preview(&preview, &source.client(), || async {
            panic!("expired balance must not submit")
        })
        .await
        .err()
        .unwrap();
    assert!(error.message.contains("expired"));
    assert!(error.signatures.is_empty());
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_sell_message_cancelled_unsigned_preview_drops_rpc_without_submit() {
    let (submit, preview) = pair();
    let mut source = MemorySimulation::new();
    source.block = true;
    let rpc = source.client();
    let task = tokio::spawn(async move {
        submit
            .after_unsigned_preview(&preview, &rpc, || async {
                panic!("cancelled preview must not submit")
            })
            .await
    });
    source.entered.notified().await;
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_sell_message_expired_candidate_is_rejected_before_simulation_or_sdk() {
    let mut profile = fixture(SwqosType::Default, true, 50).profile().unwrap();
    let value = Arc::get_mut(&mut profile).unwrap();
    value.holding = Arc::new(PreparationBalanceSnapshot::fixture(
        Address::from_bytes(value.context.wallet.to_bytes()),
        Address::from_bytes(value.intent.mint.to_bytes()),
        value.balance,
        std::time::Instant::now() - std::time::Duration::from_secs(4),
    ));
    let source = MemorySimulation::new();
    let error = profile
        .execute(&source.client(), |_| async {
            panic!("expired candidate must not use SDK")
        })
        .await
        .err()
        .unwrap();
    assert!(error.message.contains("expired"));
    assert!(source.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn damm_sell_message_live_dispatch_preserves_sdk_uncertainty_without_simulating() {
    let profile = fixture(SwqosType::Temporal, false, 50).profile().unwrap();
    let source = MemorySimulation::new();
    let error = profile
        .execute(&source.client(), |_| async {
            Err(SdkInvocationFailure {
                signatures: vec!["known-final-one".into(), "known-final-two".into()],
                message: "SDK uncertain".into(),
            })
        })
        .await
        .err()
        .unwrap();
    assert_eq!(error.signatures, ["known-final-one", "known-final-two"]);
    assert_eq!(error.message, "SDK uncertain");
    assert!(source.calls.lock().unwrap().is_empty());
}
