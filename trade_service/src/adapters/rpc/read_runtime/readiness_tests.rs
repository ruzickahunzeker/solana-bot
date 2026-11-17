use super::*;
use crate::{
    adapters::rpc::readiness::RpcReadinessProbe,
    domain::{readiness::ReadinessWalletMetadata, trade::Address},
};

fn probe(execution_urls: Vec<String>) -> RpcReadinessProbe {
    RpcReadinessProbe::new(
        ReadinessWalletMetadata {
            wallet: Address::from_bytes([1; 32]),
            rpc_lanes: 3,
            swqos_lanes: vec!["Temporal".into(), "Default".into()],
            active_swqos_lanes: vec!["Temporal".into()],
            mev_protection: false,
            require_mev_submit: true,
        },
        execution_urls,
    )
}

#[tokio::test]
async fn readiness_shared_read_primary_success_preserves_metadata_and_processed_request() {
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Success, Answer::Success));
    let probe = probe(urls());
    assert!(probe.ping(&runtime).await);
    let metadata = probe.into_metadata();
    assert_eq!(metadata.wallet.to_bytes(), [1; 32]);
    assert_eq!(metadata.rpc_lanes, 3);
    assert_eq!(metadata.swqos_lanes, ["Temporal", "Default"]);
    assert_eq!(metadata.active_swqos_lanes, ["Temporal"]);
    assert!(!metadata.mev_protection && metadata.require_mev_submit);
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "primary.example.invalid");
    assert_eq!(calls[0].1["method"], "getLatestBlockhash");
    assert_eq!(calls[0].1["params"], json!([{"commitment":"processed"}]));
    assert_eq!(runtime.slots[2].available_permits(), 1);
}

#[tokio::test]
async fn readiness_shared_read_keeps_execution_order_instead_of_global_endpoint_order() {
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Success, Answer::Success));
    assert!(
        probe(vec![urls()[1].clone(), urls()[0].clone()])
            .ping(&runtime)
            .await
    );
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "backup.example.invalid");
}

#[tokio::test]
async fn readiness_shared_read_empty_invalid_or_unregistered_candidates_never_widen_to_global_rpc()
{
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Success, Answer::Success));
    for candidates in [
        vec![],
        vec!["https://other.example.invalid".into()],
        vec!["not a URL".into(), "file:///tmp/fixture".into()],
        vec!["https://primary.example.invalid/?key=different-secret".into()],
    ] {
        assert!(!probe(candidates).ping(&runtime).await);
    }
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn readiness_shared_read_aliases_deduplicate_and_bad_replies_cannot_report_success() {
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Invalid, Answer::Invalid));
    let candidates = vec![
        urls()[0].clone(),
        "https://PRIMARY.example.invalid:443/?key=fixture-secret".into(),
        urls()[1].clone(),
    ];
    assert!(!probe(candidates).ping(&runtime).await);
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "primary.example.invalid");
    assert_eq!(calls[1].0, "backup.example.invalid");
}

#[tokio::test]
async fn readiness_shared_read_429_cools_other_profiles_without_retry_or_cooldown_extension() {
    let mut fake = Fake::new(Answer::Success, Answer::Success);
    fake.limited_primary = true;
    let runtime = SharedReadRuntime::with_transport(&urls(), fake);
    assert!(probe(urls()).ping(&runtime).await);
    let cooldown = runtime.budget.lock().unwrap().cooldowns[0];
    assert!(cooldown.is_some());
    runtime.latest_blockhash().await.unwrap();
    assert!(runtime
        .signature_status(&Url::parse(&urls()[0]).unwrap(), 123, &signature())
        .await
        .is_err());
    assert!(!probe(vec![urls()[0].clone()]).ping(&runtime).await);
    assert_eq!(runtime.budget.lock().unwrap().cooldowns[0], cooldown);
    let calls = runtime.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].0, "primary.example.invalid");
    assert!(calls[1..]
        .iter()
        .all(|call| call.0 == "backup.example.invalid"));
}

#[tokio::test]
async fn readiness_shared_read_observation_429_does_not_escape_execution_subset() {
    let mut fake = Fake::new(Answer::Success, Answer::Success);
    fake.limited_primary = true;
    let runtime = SharedReadRuntime::with_transport(&urls(), fake);
    assert_eq!(
        runtime
            .signature_status(&Url::parse(&urls()[0]).unwrap(), 1, &signature())
            .await
            .unwrap()
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(!probe(vec![urls()[0].clone()]).ping(&runtime).await);
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 1);
    assert!(probe(vec![urls()[1].clone()]).ping(&runtime).await);
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn readiness_shared_read_busy_fails_fast_while_other_reserved_profiles_still_enter() {
    let runtime = Arc::new(SharedReadRuntime::with_transport(
        &urls(),
        Fake::new(Answer::Hold, Answer::Success),
    ));
    let ready = {
        let runtime = runtime.clone();
        tokio::spawn(async move { probe(urls()).ping(&runtime).await })
    };
    tokio::time::timeout(Duration::from_secs(1), runtime.transport.entered.notified())
        .await
        .unwrap();
    assert!(
        !tokio::time::timeout(Duration::from_millis(200), probe(urls()).ping(&runtime))
            .await
            .unwrap()
    );
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 1);
    let hash = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.latest_blockhash().await })
    };
    tokio::time::timeout(Duration::from_secs(1), runtime.transport.entered.notified())
        .await
        .unwrap();
    assert_eq!(runtime.slots[0].available_permits(), 0);
    assert_eq!(runtime.slots[2].available_permits(), 0);
    assert!(runtime
        .signature_status(&Url::parse(&urls()[0]).unwrap(), 123, &signature())
        .await
        .is_ok());
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 3);
    ready.abort();
    hash.abort();
    assert!(ready.await.unwrap_err().is_cancelled());
    assert!(hash.await.unwrap_err().is_cancelled());
    assert_eq!(
        runtime
            .slots
            .each_ref()
            .map(|slot| slot.available_permits()),
        [1, 1, 1, 1, 1, 1, 1, 2]
    );
}

#[tokio::test]
async fn readiness_shared_read_cancel_releases_slot_without_refunding_start_spacing() {
    let runtime = Arc::new(SharedReadRuntime::with_transport(
        &urls(),
        Fake::new(Answer::Hold, Answer::Success),
    ));
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move { probe(urls()).ping(&runtime).await })
    };
    tokio::time::timeout(Duration::from_secs(1), runtime.transport.entered.notified())
        .await
        .unwrap();
    let next_start = runtime.budget.lock().unwrap().next_start[2];
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.slots[2].available_permits(), 1);
    assert_eq!(runtime.budget.lock().unwrap().next_start[2], next_start);
    assert_eq!(runtime.transport.dropped.load(Ordering::SeqCst), 1);
    // An unsent attempt rejected by pacing must not refund the canceled request either.
    let reserved = Instant::now() + Duration::from_secs(60);
    runtime.budget.lock().unwrap().next_start[2] = reserved;
    assert!(!probe(urls()).ping(&runtime).await);
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 1);
    assert_eq!(runtime.budget.lock().unwrap().next_start[2], reserved);
}

#[tokio::test]
async fn readiness_shared_read_pacing_rejection_never_claims_success_or_blocks_refresh() {
    let runtime =
        SharedReadRuntime::with_transport(&urls(), Fake::new(Answer::Success, Answer::Success));
    let reserved = Instant::now() + Duration::from_secs(60);
    runtime.budget.lock().unwrap().next_start[2] = reserved;
    assert!(!probe(urls()).ping(&runtime).await);
    assert!(runtime.transport.calls.lock().unwrap().is_empty());
    assert_eq!(runtime.budget.lock().unwrap().next_start[2], reserved);
    assert_eq!(runtime.slots[2].available_permits(), 1);
    runtime.latest_blockhash().await.unwrap();
    assert_eq!(runtime.transport.calls.lock().unwrap().len(), 1);
    assert_eq!(runtime.budget.lock().unwrap().next_start[2], reserved);
}

#[tokio::test]
async fn readiness_shared_read_node_and_total_deadlines_bound_serial_attempts() {
    let urls = (0..6)
        .map(|index| format!("https://node-{index}.example.invalid"))
        .collect::<Vec<_>>();
    let runtime =
        SharedReadRuntime::with_transport(&urls, Fake::new(Answer::Hold, Answer::Success));
    let started = Instant::now();
    assert!(
        !tokio::time::timeout(Duration::from_millis(1800), probe(urls).ping(&runtime))
            .await
            .unwrap()
    );
    assert!(started.elapsed() >= Duration::from_millis(1400));
    let calls = runtime.transport.calls.lock().unwrap();
    assert!(!calls.is_empty() && calls.len() <= 3);
    assert_eq!(
        runtime.transport.dropped.load(Ordering::SeqCst),
        calls.len()
    );
    assert_eq!(runtime.slots[2].available_permits(), 1);
}
