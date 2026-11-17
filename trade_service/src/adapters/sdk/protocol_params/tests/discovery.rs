use super::{super::*, read_rpc::ReadRpc};
use crate::{
    adapters::{
        rpc::{
            read_http::{HttpReply, ReadHttpTransport, ReadRequest},
            read_runtime::SharedReadRuntime,
        },
        sdk::pool_resolution::{self, DiscoveryLane},
    },
    application::{
        pool_queries::{PoolQueries, PoolQueryService},
        ports::pool_queries::{ParamCacheWriter, PoolReadBackend},
    },
    domain::{route::PoolRouteHint, trade::Address},
};
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use sol_trade_sdk::instruction::utils::{meteora_dbc, pumpfun, pumpswap};
use solana_client::rpc_request::RpcRequest;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum Mode {
    Good,
    DbcUnavailable,
    DbcInvalid,
    FirstRate,
    Unavailable,
    Hold,
}
struct Transport {
    fixture: ReadRpc,
    mode: Mode,
    calls: Mutex<Vec<(String, Value, Instant)>>,
    active: AtomicUsize,
    urls_active: Mutex<Vec<String>>,
    entered: tokio::sync::Notify,
}
struct Active<'a> {
    transport: &'a Transport,
    url: String,
}
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.transport.active.fetch_sub(1, Ordering::SeqCst);
        let mut active = self.transport.urls_active.lock().unwrap();
        let index = active.iter().position(|url| *url == self.url).unwrap();
        active.remove(index);
    }
}
impl ReadHttpTransport for Transport {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> anyhow::Result<HttpReply> {
        let body = request.test_body();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone(), Instant::now()));
        {
            let mut active = self.urls_active.lock().unwrap();
            assert!(
                active.iter().all(|other| other == url.as_str()),
                "resolver overlapped different endpoint lanes"
            );
            active.push(url.to_string());
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active {
            transport: self,
            url: url.to_string(),
        };
        self.entered.notify_one();
        let dbc = body["method"] == "getProgramAccounts"
            && body["params"][0] == meteora_dbc::accounts::PROGRAM.to_string();
        if matches!(self.mode, Mode::Hold) {
            std::future::pending::<()>().await;
        }
        if matches!(self.mode, Mode::FirstRate) && url.host_str() == Some("first.invalid") {
            return Ok(HttpReply::for_test(StatusCode::TOO_MANY_REQUESTS, vec![]));
        }
        if matches!(self.mode, Mode::Unavailable)
            || (dbc && matches!(self.mode, Mode::DbcUnavailable))
        {
            return Ok(HttpReply::for_test(StatusCode::SERVICE_UNAVAILABLE, vec![]));
        }
        if dbc && matches!(self.mode, Mode::DbcInvalid) {
            return Ok(HttpReply::for_test(
                StatusCode::OK,
                serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":body["id"].as_u64().unwrap()+1,"result":[]}),
                )
                .unwrap(),
            ));
        }
        let method = match body["method"].as_str().unwrap() {
            "getAccountInfo" => RpcRequest::GetAccountInfo,
            "getMultipleAccounts" => RpcRequest::GetMultipleAccounts,
            "getEpochInfo" => RpcRequest::GetEpochInfo,
            "getProgramAccounts" => RpcRequest::GetProgramAccounts,
            _ => panic!("unexpected money or unrelated RPC capability in discovery"),
        };
        let result = self
            .fixture
            .respond(method, body["params"].clone())
            .unwrap();
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
            urls_active: Mutex::new(vec![]),
            entered: tokio::sync::Notify::new(),
        },
    ))
}
fn address(key: Pubkey) -> Address {
    Address::from_bytes(key.to_bytes())
}
fn swap_empty(read: &mut ReadRpc, mint: Pubkey) {
    read.absent(pumpswap::get_pool_v2_pda(&mint).unwrap());
    read.absent(pumpswap::get_canonical_pool_pda(&mint));
    read.program_scan(pumpswap::accounts::AMM_PROGRAM, &[]);
}
fn missing_pump(read: &mut ReadRpc, mint: Pubkey) {
    read.absent(pumpfun::get_bonding_curve_pda(&mint).unwrap());
    read.absent(pumpfun::get_fee_sharing_config_pda(&mint).unwrap());
    read.absent(pumpfun::global_constants::GLOBAL_ACCOUNT);
}

#[tokio::test]
async fn pool_discovery_loader_live_pump_priority_and_independent_dbc_failure() {
    for mode in [Mode::Good, Mode::DbcUnavailable, Mode::DbcInvalid] {
        let (mut fixture, mint, _, _) = super::pump::fixture(false, false);
        let (dbc, _, _) = super::dbc::discovery_fixture(mint, false);
        fixture.merge(dbc);
        let r = runtime(fixture.clone(), mode);
        let hint = pool_resolution::resolve(r.clone(), &[lane(0)], address(mint))
            .await
            .unwrap();
        assert_eq!(
            hint,
            PoolRouteHint {
                pool: Pool::Pump,
                mint: address(mint),
                quote_mint: address(sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT),
                pool_id: None
            }
        );
        assert!(r
            .test_transport()
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(_, body, _)| body["params"][0] != pumpswap::accounts::AMM_PROGRAM.to_string()));
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        fixture.assert_read_only();
    }
}

#[tokio::test]
async fn pool_discovery_loader_dbc_fallback_keeps_missing_completed_and_migrated_semantics() {
    for (complete, missing, migrated) in [
        (true, false, false),
        (false, true, false),
        (true, false, true),
    ] {
        let (mut fixture, mint, _, _) = super::pump::fixture(complete, false);
        let (dbc, id, quote) = super::dbc::discovery_fixture(mint, migrated);
        fixture.merge(dbc);
        if missing {
            fixture.absent(pumpfun::get_bonding_curve_pda(&mint).unwrap());
        }
        swap_empty(&mut fixture, mint);
        let r = runtime(fixture.clone(), Mode::Good);
        let result = pool_resolution::on_lane(
            r.clone(),
            &lane(0),
            address(mint),
            Instant::now() + Duration::from_secs(3),
        )
        .await;
        if migrated {
            let error = result.unwrap_err();
            assert!(error.contains("pump bonding curve is complete"));
            assert!(error.contains("has migrated"));
        } else {
            assert_eq!(
                result.unwrap(),
                PoolRouteHint {
                    pool: Pool::MeteoraDbc,
                    mint: address(mint),
                    quote_mint: address(quote),
                    pool_id: Some(address(id))
                }
            );
        }
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        fixture.assert_read_only();
    }
}

const FLAG: &str = "SOL_BOT_DISCOVERY_CASE";
const DONE: &str = "POOL_DISCOVERY_DONE";
#[test]
fn pool_discovery_loader_cold_swap_and_empty_routes_use_isolated_sdk_caches() {
    for case in ["swap", "empty"] {
        let child = crate::adapters::test_support::IsolatedTestProcess::spawn(
            "adapters::sdk::protocol_params::tests::discovery::pool_discovery_loader_child",
            FLAG,
            case,
        );
        child.await_line(DONE);
        child.finish();
    }
}

#[tokio::test]
async fn pool_discovery_loader_child() {
    let Ok(case) = std::env::var(FLAG) else {
        return;
    };
    let (mut fixture, id, pool) = super::pump_swap::fixture(false, 1000);
    missing_pump(&mut fixture, pool.base_mint);
    fixture.program_scan(meteora_dbc::accounts::PROGRAM, &[]);
    swap_empty(&mut fixture, pool.base_mint);
    if case == "swap" {
        fixture.program_scan(pumpswap::accounts::AMM_PROGRAM, &[id]);
    }
    let r = runtime(fixture.clone(), Mode::Good);
    let result = pool_resolution::resolve(r.clone(), &[lane(0)], address(pool.base_mint)).await;
    if case == "swap" {
        assert_eq!(
            result.unwrap(),
            PoolRouteHint {
                pool: Pool::PumpAmm,
                mint: address(pool.base_mint),
                quote_mint: address(pool.quote_mint),
                pool_id: Some(address(id))
            }
        );
        let calls = r.test_transport().calls.lock().unwrap();
        assert!(calls
            .iter()
            .any(|(_, body, _)| body["method"] == "getMultipleAccounts"
                && body["params"][0].as_array().unwrap().len() == 6));
        assert_eq!(
            calls
                .iter()
                .filter(|(_, body, _)| body["method"] == "getProgramAccounts"
                    && body["params"][0] == pumpswap::accounts::AMM_PROGRAM.to_string())
                .count(),
            6
        );
    } else {
        let error = result.unwrap_err().to_string();
        assert_eq!(
            error,
            "pool resolution failed across 1 RPC lanes: pool not found"
        );
    }
    fixture.assert_read_only();
    println!("{DONE}");
}

#[tokio::test]
async fn pool_discovery_loader_serial_lane_failover_and_cooldown_respect_selected_order() {
    let (mut fixture, mint, _, _) = super::pump::fixture(false, false);
    fixture.program_scan(meteora_dbc::accounts::PROGRAM, &[]);
    let r = runtime(fixture.clone(), Mode::FirstRate);
    let hint =
        pool_resolution::resolve_ordered(r.clone(), &[lane(0), lane(1)], address(mint), &[0, 1])
            .await
            .unwrap();
    assert_eq!(hint.pool, Pool::Pump);
    let calls = r.test_transport().calls.lock().unwrap();
    let first_backup = calls
        .iter()
        .position(|(url, _, _)| url == &Url::parse(&urls()[1]).unwrap().to_string())
        .unwrap();
    assert!(first_backup >= 1);
    assert!(calls[first_backup..]
        .iter()
        .all(|(url, _, _)| url == &Url::parse(&urls()[1]).unwrap().to_string()));
    drop(calls);
    assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
    fixture.assert_read_only();
}

struct Backend {
    runtime: Arc<SharedReadRuntime<Transport>>,
}
impl PoolReadBackend for Backend {
    async fn resolve(&self, mint: Address) -> anyhow::Result<PoolRouteHint> {
        pool_resolution::resolve(self.runtime.clone(), &[lane(0)], mint).await
    }
    async fn warmup(&self, _: &PoolRouteHint) -> anyhow::Result<()> {
        panic!("unexpected warmup")
    }
    async fn sol_hop(&self, _: Address) -> anyhow::Result<Address> {
        panic!("unexpected two-hop discovery")
    }
}
#[derive(Default)]
struct Cache {
    hints: Mutex<Vec<PoolRouteHint>>,
}
impl ParamCacheWriter for Arc<Cache> {
    fn record(&self, hint: &PoolRouteHint) -> String {
        self.hints.lock().unwrap().push(hint.clone());
        "same-hint".into()
    }
}

#[tokio::test]
async fn pool_discovery_loader_failed_resolution_never_records_hint_or_false_empty() {
    for mode in [Mode::Unavailable, Mode::Hold] {
        let r = runtime(ReadRpc::default(), mode);
        let cache = Arc::new(Cache::default());
        let service = PoolQueryService::new(Backend { runtime: r.clone() }, cache.clone());
        let started = Instant::now();
        let error = service
            .resolve(Address::from_bytes([1; 32]))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("pool resolution failed across 1 RPC lanes:"));
        assert!(!error.contains("pool not found"));
        assert!(!error.contains("fixture-secret"));
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(cache.hints.lock().unwrap().is_empty());
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        let a = r.discovery_slot().unwrap();
        let b = r.discovery_slot().unwrap();
        drop((a, b));
    }
}

#[tokio::test]
async fn pool_discovery_loader_route_cache_records_only_returned_original_hint() {
    let (mut fixture, mint, _, _) = super::pump::fixture(false, false);
    fixture.program_scan(meteora_dbc::accounts::PROGRAM, &[]);
    let r = runtime(fixture.clone(), Mode::Good);
    let cache = Arc::new(Cache::default());
    let service = PoolQueryService::new(Backend { runtime: r }, cache.clone());
    let hint = service.resolve(address(mint)).await.unwrap();
    assert_eq!(*cache.hints.lock().unwrap(), vec![hint.clone()]);
    assert_eq!(hint.pool_id, None);
    assert_eq!(hint.mint, address(mint));
    fixture.assert_read_only();
}

#[tokio::test]
async fn pool_discovery_loader_lane_selection_and_diagnostics_never_expose_endpoints() {
    let mask = [false, true, false, true, true];
    assert_eq!(pool_resolution::lane_order(&mask, false), [1, 3, 0, 2, 4]);
    assert_eq!(pool_resolution::lane_order(&mask, true), [3, 1, 0, 2, 4]);
    assert_eq!(pool_resolution::lane_order(&[true], true), [0]);
    let r = runtime(ReadRpc::default(), Mode::Good);
    let error = pool_resolution::resolve(r.clone(), &[], Address::from_bytes([1; 32]))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "no Default RPC lane configured for pool resolution"
    );
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
    for (input, expected) in [
        (
            "Account not found; https://fixture.invalid/?key=fixture-secret",
            "pool not found",
        ),
        ("No Meteora DBC pool found", "RPC unavailable"),
        ("429 fixture-secret", "rate limited"),
        ("timed out fixture-secret", "timeout"),
    ] {
        assert_eq!(pool_resolution::failure_kind(input), expected);
    }
}
