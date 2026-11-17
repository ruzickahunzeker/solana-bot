use super::*;
use crate::adapters::rpc::discovery_reads::{DiscoveryProtocol, DiscoveryReadSession};
use sol_trade_sdk::instruction::utils::meteora_dbc;

fn scoped(
    r: &Arc<SharedReadRuntime<Fake>>,
    protocol: DiscoveryProtocol,
) -> DiscoveryReadSession<Fake> {
    DiscoveryReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        protocol,
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(3),
    )
}
fn free(r: &SharedReadRuntime<Fake>) {
    let one = r.discovery_slot().unwrap();
    let two = r.discovery_slot().unwrap();
    assert!(r.discovery_slot().is_err());
    drop((one, two));
}
fn dbc_query(mint: Pubkey) -> Value {
    use solana_client::{
        rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
        rpc_filter::{Memcmp, RpcFilterType},
    };
    json!([
        meteora_dbc::accounts::PROGRAM.to_string(),
        RpcProgramAccountsConfig {
            filters: Some(vec![
                RpcFilterType::DataSize(meteora_dbc::VIRTUAL_POOL_ACCOUNT_SIZE),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                    0,
                    &meteora_dbc::VIRTUAL_POOL_DISCRIMINATOR
                )),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                    meteora_dbc::BASE_MINT_OFFSET,
                    mint.as_ref()
                ))
            ]),
            account_config: RpcAccountInfoConfig {
                encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                ..Default::default()
            },
            with_context: None,
            sort_results: None,
        }
    ])
}

#[tokio::test]
async fn pool_discovery_session_real_dbc_scan_keeps_explicit_empty_distinct_from_failure() {
    let r = runtime(vec![Mode::Good]);
    let s = scoped(&r, DiscoveryProtocol::Dbc);
    let result = meteora_dbc::find_pool_by_base_mint(s.rpc(), &key(1)).await;
    let error = s.finish(result).err().unwrap();
    assert!(error.downcast_ref::<ParameterReadFailure>().is_none());
    assert!(error.to_string().contains("No Meteora DBC pool found"));
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let mut expected = dbc_query(key(1));
    expected[1]["commitment"] = json!("processed");
    assert_eq!(calls[0].1["params"], expected);
    drop(calls);
    free(&r);
}

#[tokio::test]
async fn pool_discovery_session_scans_are_exactly_bound_to_protocol_mint_size_and_flags() {
    let valid = dbc_query(key(1));
    for damage in 0..9 {
        let r = runtime(vec![]);
        let s = scoped(&r, DiscoveryProtocol::Dbc);
        let mut params = valid.clone();
        match damage {
            0 => params[0] = json!(key(9).to_string()),
            1 => params[1]["filters"][0] = json!({"dataSize":425}),
            2 => params[1]["filters"][1]["memcmp"]["bytes"] = json!(key(9).to_string()),
            3 => params[1]["filters"][2]["memcmp"]["offset"] = json!(135),
            4 => params[1]["filters"][2]["memcmp"]["bytes"] = json!(key(9).to_string()),
            5 => params[1]["withContext"] = json!(true),
            6 => params[1]["sortResults"] = json!(true),
            7 => params[1]["dataSlice"] = json!({"offset":0,"length":1}),
            _ => params[1]["filters"] = json!([]),
        }
        assert!(s
            .rpc()
            .send::<Value>(RpcRequest::GetProgramAccounts, params)
            .await
            .is_err());
        assert!(!s
            .finish(Ok(()))
            .unwrap_err()
            .downcast_ref::<ParameterReadFailure>()
            .unwrap()
            .allows_lane_fallback());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
    }
    for protocol in [DiscoveryProtocol::Pump, DiscoveryProtocol::PumpSwap] {
        let r = runtime(vec![]);
        let s = scoped(&r, protocol);
        assert!(s
            .rpc()
            .send::<Value>(RpcRequest::GetProgramAccounts, valid.clone())
            .await
            .is_err());
        assert!(s.finish(Ok(())).is_err());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
    }
    let r = runtime(vec![]);
    let s = ParameterReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        ParameterReadScope::new(Pool::MeteoraDbc, Address::from_bytes([1; 32]), None),
    );
    assert!(s
        .rpc()
        .send::<Value>(RpcRequest::GetProgramAccounts, valid.clone())
        .await
        .is_err());
    assert!(s.finish(Ok(())).is_err());
    let mut scope = ParameterReadScope::new(Pool::MeteoraDbc, Address::from_bytes([1; 32]), None);
    scope.discovery = true;
    let query =
        request::ParameterQuery::parse(RpcRequest::GetProgramAccounts, valid, scope).unwrap();
    assert_eq!(query.max_body(), 1_048_576);
    assert!(query
        .validate_result(&json!(vec![json!({}); 1001]))
        .is_err());
    assert!(query
        .validate_result(&json!([{"pubkey":"invalid","account":account()}]))
        .is_err());
}

#[tokio::test]
async fn pool_discovery_session_two_branch_capacity_and_pacing_do_not_consume_parameter_or_hash_slots(
) {
    let r = runtime(vec![Mode::Good; 2]);
    let parameters = r.parameter_slot().unwrap();
    let a = scoped(&r, DiscoveryProtocol::Pump);
    let b = scoped(&r, DiscoveryProtocol::Dbc);
    let first = key(1);
    let second = key(2);
    let (left, right) = tokio::join!(a.rpc().get_account(&first), b.rpc().get_account(&second));
    left.unwrap();
    right.unwrap();
    let c = scoped(&r, DiscoveryProtocol::PumpSwap);
    assert!(c.rpc().get_account(&key(3)).await.is_err());
    assert!(c.finish(Ok(())).is_err());
    r.latest_blockhash().await.unwrap();
    a.finish(Ok(())).unwrap();
    b.finish(Ok(())).unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(90));
    drop(calls);
    drop(parameters);
    free(&r);
}

#[tokio::test]
async fn pool_discovery_session_sibling_rate_fault_keeps_own_evidence_but_shares_endpoint_cooldown()
{
    let r = runtime(vec![Mode::Good, Mode::Rate]);
    let a = scoped(&r, DiscoveryProtocol::Pump);
    a.rpc().get_account(&key(1)).await.unwrap();
    let b = scoped(&r, DiscoveryProtocol::Dbc);
    assert!(b.rpc().get_account(&key(2)).await.is_err());
    assert!(b.finish(Ok("swallowed fault")).is_err());
    assert_eq!(
        a.finish(Ok("valid own evidence")).unwrap(),
        "valid own evidence"
    );
    let p = session(&r);
    assert!(p.rpc().get_account(&key(3)).await.is_err());
    assert!(p.finish(Ok(())).is_err());
    r.latest_blockhash().await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2].0, Url::parse(&urls()[1]).unwrap().to_string());
    drop(calls);
    free(&r);
}

#[tokio::test]
async fn pool_discovery_session_cancellation_drops_io_and_releases_both_branch_slots() {
    let r = runtime(vec![Mode::Hold]);
    let s = scoped(&r, DiscoveryProtocol::Pump);
    let task = tokio::spawn(async move {
        let result = s.rpc().get_account(&key(1)).await;
        s.finish(result.map_err(anyhow::Error::from))
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        r.test_transport().entered.notified(),
    )
    .await
    .unwrap();
    let other = r.discovery_slot().unwrap();
    assert!(r.discovery_slot().is_err());
    assert!(r.parameter_slot().is_ok());
    r.latest_blockhash().await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(r.test_transport().dropped.load(Ordering::SeqCst), 1);
    drop(other);
    free(&r);
}

#[tokio::test]
async fn pool_discovery_session_caller_cannot_extend_total_budget_beyond_three_seconds() {
    let r = runtime(vec![Mode::Slow, Mode::Slow, Mode::Hold]);
    let s = DiscoveryReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        DiscoveryProtocol::Pump,
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(30),
    );
    let started = Instant::now();
    s.rpc().get_account(&key(1)).await.unwrap();
    s.rpc().get_account(&key(2)).await.unwrap();
    assert!(s.rpc().get_account(&key(3)).await.is_err());
    assert!(s.finish(Ok(())).is_err());
    assert!(started.elapsed() >= Duration::from_millis(2800));
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 3);
    free(&r);
}

#[tokio::test]
async fn pool_discovery_session_rejects_money_simulation_and_unrelated_read_methods_before_http() {
    for protocol in [
        DiscoveryProtocol::Pump,
        DiscoveryProtocol::Dbc,
        DiscoveryProtocol::PumpSwap,
    ] {
        for method in [
            RpcRequest::SendTransaction,
            RpcRequest::SimulateTransaction,
            RpcRequest::GetBalance,
            RpcRequest::GetLatestBlockhash,
            RpcRequest::GetTokenAccountsByOwner,
            RpcRequest::GetVersion,
        ] {
            let r = runtime(vec![]);
            let s = scoped(&r, protocol);
            assert!(s.rpc().send::<Value>(method, json!([])).await.is_err());
            assert!(!s
                .finish(Ok(()))
                .unwrap_err()
                .downcast_ref::<ParameterReadFailure>()
                .unwrap()
                .allows_lane_fallback());
            assert!(r.test_transport().calls.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn pool_discovery_session_invalid_unavailable_and_unknown_endpoints_are_not_empty_successes()
{
    for (mode, available) in [
        (Mode::WrongId, false),
        (Mode::BadAccount, false),
        (Mode::Oversize, false),
        (Mode::RpcError, true),
    ] {
        let r = runtime(vec![mode]);
        let s = scoped(&r, DiscoveryProtocol::Dbc);
        assert!(s.rpc().get_account(&key(1)).await.is_err());
        assert!(s.rpc().get_account(&key(2)).await.is_err());
        let error = s.finish(Ok("empty result")).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<ParameterReadFailure>()
                .unwrap()
                .allows_lane_fallback(),
            available
        );
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        free(&r);
    }
    let r = runtime(vec![]);
    let s = DiscoveryReadSession::new(
        r.clone(),
        "https://unknown.invalid?key=fixture-secret".into(),
        CommitmentConfig::processed(),
        DiscoveryProtocol::Pump,
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(3),
    );
    assert!(s.rpc().get_account(&key(1)).await.is_err());
    assert!(!format!("{:?}", s.finish(Ok(())).unwrap_err()).contains("fixture-secret"));
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}
