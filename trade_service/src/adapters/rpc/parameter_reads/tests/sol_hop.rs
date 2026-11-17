use super::*;
use crate::adapters::rpc::{
    discovery_reads::{DiscoveryProtocol, DiscoveryReadSession},
    sol_hop_reads::SolHopReadSession,
};
use sol_trade_sdk::instruction::utils::{
    raydium_cpmm::accounts::RAYDIUM_CPMM, raydium_cpmm_types::POOL_STATE_DISCRIMINATOR,
};

fn scoped(r: &Arc<SharedReadRuntime<Fake>>) -> SolHopReadSession<Fake> {
    SolHopReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        Address::from_bytes(key(1).to_bytes()),
        Instant::now() + Duration::from_secs(5),
    )
}
fn scan(reverse: bool) -> Value {
    use solana_client::{
        rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
        rpc_filter::{Memcmp, RpcFilterType},
    };
    let (a, b) = if reverse { (200, 168) } else { (168, 200) };
    json!([
        RAYDIUM_CPMM.to_string(),
        RpcProgramAccountsConfig {
            filters: Some(vec![
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(0, &POOL_STATE_DISCRIMINATOR)),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(a, key(1).as_ref())),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                    b,
                    sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT.as_ref()
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
async fn sol_hop_session_exact_pair_scans_accept_both_orientations_and_reject_widening() {
    let r = runtime(vec![Mode::Good; 2]);
    let s = scoped(&r);
    for reverse in [false, true] {
        assert_eq!(
            s.rpc()
                .send::<Value>(RpcRequest::GetProgramAccounts, scan(reverse))
                .await
                .unwrap(),
            json!([])
        );
    }
    s.finish(Ok(())).unwrap();
    for damage in 0..9 {
        let r = runtime(vec![]);
        let s = scoped(&r);
        let mut args = scan(false);
        match damage {
            0 => args[0] = json!(key(9).to_string()),
            1 => args[1]["filters"][0]["memcmp"]["bytes"] = json!(key(9).to_string()),
            2 => args[1]["filters"][1]["memcmp"]["offset"] = json!(169),
            3 => args[1]["filters"][1]["memcmp"]["bytes"] = json!(key(9).to_string()),
            4 => args[1]["filters"][2]["memcmp"]["bytes"] = json!(key(9).to_string()),
            5 => args[1]["filters"]
                .as_array_mut()
                .unwrap()
                .push(json!({"dataSize":637})),
            6 => args[1]["withContext"] = json!(true),
            7 => args[1]["dataSlice"] = json!({"offset":0,"length":1}),
            _ => args[1]["sortResults"] = json!(true),
        }
        assert!(s
            .rpc()
            .send::<Value>(RpcRequest::GetProgramAccounts, args)
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
    let r = runtime(vec![]);
    let p = ParameterReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        ParameterReadScope::new(Pool::RaydiumCpmm, Address::from_bytes([1; 32]), None),
    );
    assert!(p
        .rpc()
        .send::<Value>(RpcRequest::GetProgramAccounts, scan(false))
        .await
        .is_err());
    assert!(p.finish(Ok(())).is_err());
    let d = DiscoveryReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        DiscoveryProtocol::PumpSwap,
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(3),
    );
    assert!(d
        .rpc()
        .send::<Value>(RpcRequest::GetProgramAccounts, scan(false))
        .await
        .is_err());
    assert!(d.finish(Ok(())).is_err());
}

#[tokio::test]
async fn sol_hop_session_sdk_balance_keeps_full_u64_and_checks_context_and_metadata() {
    let r = runtime(vec![Mode::Good]);
    let s = scoped(&r);
    assert_eq!(
        s.rpc()
            .get_token_account_balance(&key(4))
            .await
            .unwrap()
            .amount,
        u64::MAX.to_string()
    );
    s.finish(Ok(())).unwrap();
    let mut scope = ParameterReadScope::new(Pool::RaydiumCpmm, Address::from_bytes([1; 32]), None);
    scope.hop_quote = Some(scope.mint);
    let query = request::ParameterQuery::parse(
        RpcRequest::GetTokenAccountBalance,
        json!([key(4).to_string(),{"commitment":"processed"}]),
        scope,
    )
    .unwrap();
    let good = json!({"context":{"slot":42},"value":{"amount":u64::MAX.to_string(),"decimals":6,"uiAmount":null,"uiAmountString":"display"}});
    query.validate_result(&good).unwrap();
    for damage in 0..7 {
        let mut value = good.clone();
        match damage {
            0 => value["context"]["slot"] = json!(0),
            1 => value["value"]["amount"] = json!("18446744073709551616"),
            2 => value["value"]["amount"] = json!("0.1"),
            3 => value["value"]["amount"] = json!(-1),
            4 => value["value"]["decimals"] = json!(256),
            5 => value["value"]["uiAmountString"] = Value::Null,
            _ => value["value"] = Value::Null,
        }
        assert!(query.validate_result(&value).is_err());
    }
    assert!(request::ParameterQuery::parse(
        RpcRequest::GetTokenAccountBalance,
        json!([key(4).to_string(),{"commitment":"processed","minContextSlot":1}]),
        scope
    )
    .is_err());
    let r = runtime(vec![]);
    let ordinary = session(&r);
    assert!(ordinary
        .rpc()
        .get_token_account_balance(&key(4))
        .await
        .is_err());
    assert!(ordinary.finish(Ok(())).is_err());
    let discovery = DiscoveryReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        DiscoveryProtocol::Dbc,
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(3),
    );
    assert!(discovery
        .rpc()
        .get_token_account_balance(&key(4))
        .await
        .is_err());
    assert!(discovery.finish(Ok(())).is_err());
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sol_hop_session_single_scope_capacity_and_pacing_leave_other_profiles_free() {
    let r = runtime(vec![Mode::Good; 2]);
    let s = scoped(&r);
    s.rpc().get_account(&key(1)).await.unwrap();
    s.rpc().get_token_account_balance(&key(4)).await.unwrap();
    let other = scoped(&r);
    assert!(other.rpc().get_account(&key(2)).await.is_err());
    assert!(other.finish(Ok(())).is_err());
    assert!(r.parameter_slot().is_ok());
    let a = r.discovery_slot().unwrap();
    let b = r.discovery_slot().unwrap();
    drop((a, b));
    r.latest_blockhash().await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls[1].2.duration_since(calls[0].2) >= Duration::from_millis(90));
    drop(calls);
    s.finish(Ok(())).unwrap();
    assert!(r.sol_hop_slot().is_ok());
}

#[tokio::test]
async fn sol_hop_session_cancellation_closes_scope_and_releases_active_read() {
    let r = runtime(vec![Mode::Hold]);
    let s = scoped(&r);
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
    assert!(r.sol_hop_slot().is_err());
    r.latest_blockhash().await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(r.test_transport().dropped.load(Ordering::SeqCst), 1);
    assert!(r.sol_hop_slot().is_ok());
}

#[tokio::test]
async fn sol_hop_session_429_shares_cooldown_without_switching_current_query() {
    let r = runtime(vec![Mode::Rate]);
    let s = scoped(&r);
    assert!(s
        .rpc()
        .send::<Value>(RpcRequest::GetProgramAccounts, scan(false))
        .await
        .is_err());
    assert!(s.finish(Ok(None::<Pubkey>)).is_err());
    let p = session(&r);
    assert!(p.rpc().get_account(&key(4)).await.is_err());
    assert!(p.finish(Ok(())).is_err());
    r.latest_blockhash().await.unwrap();
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, Url::parse(&urls()[1]).unwrap().to_string());
}

#[tokio::test]
async fn sol_hop_session_total_deadline_cannot_be_renewed_by_caller() {
    let r = runtime(vec![
        Mode::Slow,
        Mode::Slow,
        Mode::Slow,
        Mode::Slow,
        Mode::Hold,
    ]);
    let s = SolHopReadSession::new(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(30),
    );
    let started = Instant::now();
    for n in 1..=4 {
        s.rpc().get_account(&key(n)).await.unwrap();
    }
    assert!(s.rpc().get_account(&key(5)).await.is_err());
    assert!(s.finish(Ok(())).is_err());
    assert!(started.elapsed() >= Duration::from_millis(4700));
    assert!(started.elapsed() < Duration::from_secs(6));
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 5);
    assert!(r.sol_hop_slot().is_ok());
}

#[tokio::test]
async fn sol_hop_session_queued_admissions_are_capped_before_candidate_fanout() {
    let r = runtime(vec![Mode::Hold]);
    let s = ParameterReadSession::sol_hop(
        r.clone(),
        urls()[0].clone(),
        CommitmentConfig::processed(),
        Address::from_bytes([1; 32]),
        Instant::now() + Duration::from_secs(5),
    );
    let sender = Arc::new(Sender {
        runtime: r.clone(),
        url: urls()[0].clone(),
        state: s.state.clone(),
    });
    let mut tasks = tokio::task::JoinSet::new();
    for n in 0..32 {
        let sender = sender.clone();
        tasks.spawn(async move {
            sender
                .send(
                    RpcRequest::GetAccountInfo,
                    json!([key(n).to_string(),{"encoding":"base64","commitment":"processed"}]),
                )
                .await
        });
    }
    tokio::time::timeout(
        Duration::from_secs(1),
        r.test_transport().entered.notified(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while s.state.calls.load(Ordering::Relaxed) < 32 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(sender
        .send(
            RpcRequest::GetAccountInfo,
            json!([key(99).to_string(),{"encoding":"base64"}])
        )
        .await
        .is_err());
    assert!(!s
        .finish(Ok(()))
        .unwrap_err()
        .downcast_ref::<ParameterReadFailure>()
        .unwrap()
        .allows_lane_fallback());
    while let Some(result) = tasks.join_next().await {
        assert!(result.unwrap().is_err());
    }
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    assert!(r.sol_hop_slot().is_ok());
}

#[tokio::test]
async fn sol_hop_session_bad_reads_and_denied_methods_cannot_become_empty_success() {
    for mode in [Mode::WrongId, Mode::Oversize, Mode::RpcError] {
        let r = runtime(vec![mode]);
        let s = scoped(&r);
        assert!(s
            .rpc()
            .send::<Value>(RpcRequest::GetProgramAccounts, scan(false))
            .await
            .is_err());
        assert!(s.finish(Ok(None::<Pubkey>)).is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
    for method in [
        RpcRequest::SendTransaction,
        RpcRequest::SimulateTransaction,
        RpcRequest::GetBalance,
        RpcRequest::GetLatestBlockhash,
        RpcRequest::GetTokenSupply,
        RpcRequest::GetTokenAccountsByOwner,
    ] {
        let r = runtime(vec![]);
        let s = scoped(&r);
        assert!(s.rpc().send::<Value>(method, json!([])).await.is_err());
        assert!(s.finish(Ok(())).is_err());
        assert!(r.test_transport().calls.lock().unwrap().is_empty());
    }
}
