use super::{super::*, read_rpc::ReadRpc};
use crate::adapters::{
    rpc::{
        read_http::{HttpReply, ReadHttpTransport, ReadRequest},
        read_runtime::SharedReadRuntime,
    },
    sdk::{pool_resolution::DiscoveryLane, sol_hop_resolution},
};
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use solana_client::rpc_request::RpcRequest;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Clone, Copy)]
enum Mode {
    Good,
    Empty,
    FirstRate,
    Partial,
    BadBalance,
    Hold,
    Slow,
}
struct Transport {
    fixture: ReadRpc,
    mode: Mode,
    calls: Mutex<Vec<(String, Value)>>,
    active: AtomicUsize,
    urls: Mutex<Vec<String>>,
    entered: tokio::sync::Notify,
}
struct Active<'a>(&'a Transport, String);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        let mut urls = self.0.urls.lock().unwrap();
        let index = urls.iter().position(|url| url == &self.1).unwrap();
        urls.remove(index);
    }
}
impl ReadHttpTransport for Transport {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> anyhow::Result<HttpReply> {
        let body = request.test_body();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone()));
        {
            let mut urls = self.urls.lock().unwrap();
            assert!(urls.iter().all(|other| other == url.as_str()));
            urls.push(url.to_string());
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        let _guard = Active(self, url.to_string());
        self.entered.notify_one();
        if matches!(self.mode, Mode::Hold) {
            std::future::pending::<()>().await;
        }
        if matches!(self.mode, Mode::Slow) {
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
        if matches!(self.mode, Mode::FirstRate) && url.host_str() == Some("first.invalid") {
            return Ok(HttpReply::for_test(StatusCode::TOO_MANY_REQUESTS, vec![]));
        }
        if matches!(self.mode, Mode::Partial)
            && body["method"] == "getProgramAccounts"
            && body["params"][1]["filters"][1]["memcmp"]["offset"] == 200
        {
            return Ok(HttpReply::for_test(StatusCode::SERVICE_UNAVAILABLE, vec![]));
        }
        let method = match body["method"].as_str().unwrap() {
            "getAccountInfo" => RpcRequest::GetAccountInfo,
            "getMultipleAccounts" => RpcRequest::GetMultipleAccounts,
            "getEpochInfo" => RpcRequest::GetEpochInfo,
            "getProgramAccounts" => RpcRequest::GetProgramAccounts,
            "getTokenAccountBalance" => RpcRequest::GetTokenAccountBalance,
            _ => panic!("unexpected money or unrelated query in SOL hop discovery"),
        };
        let result = if matches!(self.mode, Mode::Empty | Mode::Partial)
            && method == RpcRequest::GetProgramAccounts
        {
            json!([])
        } else if matches!(self.mode, Mode::BadBalance)
            && method == RpcRequest::GetTokenAccountBalance
        {
            json!({"context":{"slot":42},"value":{"amount":"18446744073709551616","decimals":6,"uiAmount":null,"uiAmountString":"unused"}})
        } else {
            self.fixture
                .respond(method, body["params"].clone())
                .unwrap()
        };
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":result})).unwrap(),
        ))
    }
}
fn urls() -> Vec<String> {
    vec![
        "https://first.invalid/?key=fixture-secret".into(),
        "https://second.invalid".into(),
    ]
}
fn lane(index: usize) -> DiscoveryLane {
    DiscoveryLane::new(
        urls()[index].clone(),
        solana_commitment_config::CommitmentConfig::default(),
        false,
    )
}
fn runtime(fixture: ReadRpc, mode: Mode) -> Arc<SharedReadRuntime<Transport>> {
    Arc::new(SharedReadRuntime::with_test_transport(
        &urls(),
        Transport {
            fixture,
            mode,
            calls: Mutex::new(vec![]),
            active: AtomicUsize::new(0),
            urls: Mutex::new(vec![]),
            entered: tokio::sync::Notify::new(),
        },
    ))
}
fn cache() -> AsyncMutex<HashMap<Pubkey, (Pubkey, tokio::time::Instant)>> {
    AsyncMutex::new(HashMap::new())
}
fn quote(pool: &sol_trade_sdk::instruction::utils::raydium_cpmm_types::PoolState) -> Pubkey {
    if pool.token0_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT {
        pool.token1_mint
    } else {
        pool.token0_mint
    }
}
fn witness(calls: Vec<(RpcRequest, Value)>) -> Vec<String> {
    let mut result = calls
        .into_iter()
        .map(|(method, mut args)| {
            if matches!(
                method,
                RpcRequest::GetAccountInfo | RpcRequest::GetMultipleAccounts
            ) {
                args[1]["encoding"] = json!("base64");
            }
            serde_json::to_string(&json!([method.to_string(), args])).unwrap()
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

#[tokio::test]
async fn sol_hop_loader_real_sdk_directions_keep_original_read_witness_and_positive_ttl() {
    for reverse in [false, true] {
        let (fixture, id, pool) = super::cpmm::hop_fixture(reverse, false);
        let quote = quote(&pool);
        assert_eq!(
            sol_hop_resolution::on_rpc(
                &fixture.client(),
                &quote,
                &sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
            )
            .await
            .unwrap(),
            Some(id)
        );
        let old = witness(fixture.calls());
        fixture.clear_calls();
        let r = runtime(fixture.clone(), Mode::Good);
        let c = cache();
        assert_eq!(
            sol_hop_resolution::resolve(r.clone(), &[lane(0)], &c, quote)
                .await
                .unwrap(),
            id
        );
        assert_eq!(witness(fixture.calls()), old);
        let entries = c.lock().await;
        let (stored, expiry) = entries.get(&quote).unwrap();
        assert_eq!(*stored, id);
        let remaining = expiry.saturating_duration_since(tokio::time::Instant::now());
        assert!(remaining > Duration::from_secs(29) && remaining <= Duration::from_secs(30));
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        fixture.assert_read_only();
    }
}

#[tokio::test]
async fn sol_hop_loader_cached_hit_precedes_validation_and_needs_no_registered_endpoint() {
    let r = runtime(ReadRpc::default(), Mode::Good);
    let c = cache();
    let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
    let id = Pubkey::new_from_array([99; 32]);
    c.lock().await.insert(
        wsol,
        (id, tokio::time::Instant::now() + Duration::from_secs(30)),
    );
    assert_eq!(
        sol_hop_resolution::resolve(r.clone(), &[], &c, wsol)
            .await
            .unwrap(),
        id
    );
    c.lock().await.insert(
        wsol,
        (id, tokio::time::Instant::now() - Duration::from_secs(1)),
    );
    assert_eq!(
        sol_hop_resolution::resolve(r.clone(), &[], &c, wsol)
            .await
            .unwrap_err()
            .to_string(),
        "quote mint is already WSOL"
    );
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sol_hop_loader_expired_success_rescans_and_failed_lookup_preserves_old_entry() {
    let (fixture, id, pool) = super::cpmm::hop_fixture(false, false);
    let q = quote(&pool);
    let old = Pubkey::new_from_array([99; 32]);
    let expired = tokio::time::Instant::now() - Duration::from_secs(1);
    let c = cache();
    c.lock().await.insert(q, (old, expired));
    let r = runtime(fixture.clone(), Mode::Partial);
    assert!(sol_hop_resolution::resolve(r, &[lane(0)], &c, q)
        .await
        .is_err());
    assert_eq!(*c.lock().await.get(&q).unwrap(), (old, expired));
    let r = runtime(fixture.clone(), Mode::Good);
    assert_eq!(
        sol_hop_resolution::resolve(r.clone(), &[lane(0)], &c, q)
            .await
            .unwrap(),
        id
    );
    assert!(!r.test_transport().calls.lock().unwrap().is_empty());
    assert_eq!(c.lock().await.get(&q).unwrap().0, id);
    fixture.assert_read_only();
}

#[tokio::test]
async fn sol_hop_loader_concurrent_hits_share_original_cache_lock_and_only_one_scan() {
    let (fixture, id, pool) = super::cpmm::hop_fixture(false, false);
    let r = runtime(fixture.clone(), Mode::Good);
    let c = cache();
    let q = quote(&pool);
    let lanes = [lane(0)];
    let (a, b) = tokio::join!(
        sol_hop_resolution::resolve(r.clone(), &lanes, &c, q),
        sol_hop_resolution::resolve(r.clone(), &lanes, &c, q)
    );
    assert_eq!(a.unwrap(), id);
    assert_eq!(b.unwrap(), id);
    assert_eq!(
        r.test_transport()
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, body)| body["method"] == "getProgramAccounts")
            .count(),
        1
    );
    assert_eq!(c.lock().await.len(), 1);
    fixture.assert_read_only();
}

#[tokio::test]
async fn sol_hop_loader_only_two_complete_empty_lanes_claim_no_pool_and_preserve_legacy_errors() {
    let q = Pubkey::new_from_array([7; 32]);
    for (lanes, expected) in [
        (
            vec![],
            "no verified WSOL/Quote Raydium CPMM pool: no Default RPC lane configured",
        ),
        (
            vec![lane(0)],
            "no verified WSOL/Quote Raydium CPMM pool: no Default RPC lane configured",
        ),
        (
            vec![lane(0), lane(1)],
            "no WSOL/Quote Raydium CPMM pool found on two RPC lanes",
        ),
    ] {
        let r = runtime(ReadRpc::default(), Mode::Empty);
        let c = cache();
        assert_eq!(
            sol_hop_resolution::resolve(r, &lanes, &c, q)
                .await
                .unwrap_err()
                .to_string(),
            expected
        );
        assert!(c.lock().await.is_empty());
    }
    let r = runtime(ReadRpc::default(), Mode::Partial);
    let c = cache();
    let error = sol_hop_resolution::resolve(r.clone(), &[lane(0), lane(1)], &c, q)
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("found on two RPC lanes"));
    assert!(c.lock().await.is_empty());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn sol_hop_loader_failed_primary_uses_original_serial_lane_order_without_hidden_retry() {
    let (fixture, id, pool) = super::cpmm::hop_fixture(false, false);
    let r = runtime(fixture.clone(), Mode::FirstRate);
    let c = cache();
    assert_eq!(
        sol_hop_resolution::resolve(r.clone(), &[lane(0), lane(1)], &c, quote(&pool))
            .await
            .unwrap(),
        id
    );
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls[0].0, Url::parse(&urls()[0]).unwrap().to_string());
    assert!(calls[1..]
        .iter()
        .all(|(url, _)| url == &Url::parse(&urls()[1]).unwrap().to_string()));
    drop(calls);
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
    fixture.assert_read_only();
}

#[tokio::test]
async fn sol_hop_loader_bad_balances_zero_vaults_and_original_cpmm_policy_never_cache_route() {
    for hook in [false, true] {
        let (fixture, _, pool) = super::cpmm::hop_policy_fixture(hook);
        let r = runtime(fixture.clone(), Mode::Good);
        let c = cache();
        let error = sol_hop_resolution::resolve(r, &[lane(0)], &c, quote(&pool))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(if hook {
            "Active transfer hook"
        } else {
            "fees exceed vault balance"
        }));
        assert!(c.lock().await.is_empty());
        fixture.assert_read_only();
    }
    for zero in [false, true] {
        let (fixture, _, pool) = super::cpmm::hop_fixture(false, zero);
        let r = runtime(
            fixture.clone(),
            if zero { Mode::Good } else { Mode::BadBalance },
        );
        let c = cache();
        let error = sol_hop_resolution::resolve(r, &[lane(0)], &c, quote(&pool))
            .await
            .unwrap_err()
            .to_string();
        if !zero {
            assert!(error.contains("invalid hop balance units"));
        }
        assert!(c.lock().await.is_empty());
        fixture.assert_read_only();
    }
}

#[tokio::test]
async fn sol_hop_loader_candidate_budget_exhaustion_is_not_an_empty_scan() {
    let (mut fixture, id, pool) = super::cpmm::hop_fixture(false, true);
    fixture.program_scan(
        sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM,
        &vec![id; 20],
    );
    let r = runtime(fixture.clone(), Mode::Good);
    let c = cache();
    let error = sol_hop_resolution::resolve(r.clone(), &[lane(0)], &c, quote(&pool))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("request count exceeds limit"));
    assert!(!error.contains("found on two RPC lanes"));
    assert!(r.test_transport().calls.lock().unwrap().len() <= 32);
    assert!(c.lock().await.is_empty());
    fixture.assert_read_only();
}

#[tokio::test]
async fn sol_hop_loader_cancellation_releases_original_cache_lock_without_partial_success() {
    let r = runtime(ReadRpc::default(), Mode::Hold);
    let c = Arc::new(cache());
    let q = Pubkey::new_from_array([7; 32]);
    let first = {
        let r = r.clone();
        let c = c.clone();
        tokio::spawn(async move { sol_hop_resolution::resolve(r, &[lane(0)], &c, q).await })
    };
    tokio::time::timeout(
        Duration::from_secs(1),
        r.test_transport().entered.notified(),
    )
    .await
    .unwrap();
    assert!(c.try_lock().is_err());
    assert!(r.sol_hop_slot().is_err());
    let second = {
        let r = r.clone();
        let c = c.clone();
        tokio::spawn(async move { sol_hop_resolution::resolve(r, &[lane(0)], &c, q).await })
    };
    tokio::task::yield_now().await;
    assert!(!second.is_finished());
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    second.abort();
    assert!(second.await.err().unwrap().is_cancelled());
    first.abort();
    assert!(first.await.err().unwrap().is_cancelled());
    assert!(c.try_lock().unwrap().is_empty());
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
    assert!(r.sol_hop_slot().is_ok());
}

#[tokio::test]
async fn sol_hop_loader_five_second_total_does_not_renew_after_multiple_reads() {
    let (fixture, _, pool) = super::cpmm::hop_fixture(false, false);
    let r = runtime(fixture.clone(), Mode::Slow);
    let c = cache();
    let started = Instant::now();
    let error = sol_hop_resolution::resolve(r.clone(), &[lane(0)], &c, quote(&pool))
        .await
        .unwrap_err()
        .to_string();
    assert!(started.elapsed() >= Duration::from_millis(4700));
    assert!(started.elapsed() < Duration::from_secs(6));
    assert!(!error.contains("found on two RPC lanes"));
    assert!(!error.contains("fixture-secret"));
    assert!(c.lock().await.is_empty());
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
    fixture.assert_read_only();
}
