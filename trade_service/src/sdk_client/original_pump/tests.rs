use super::*;
use crate::{
    adapters::test_support::read_rpc_server::{LocalServer, Reply},
    domain::{
        blockhash::Blockhash, execution_policy::BroadcastPolicy,
        pump_native_candidate::PumpNativeCandidateRecord,
    },
    sdk_client::SdkLane,
};
use serde_json::Value;
use sol_trade_sdk::{
    client::{TradeRiskGate, TradingInfrastructure},
    common::InfrastructureConfig,
    swqos::{SwqosType, TradeType},
    trading::MiddlewareManager,
    TradingClient,
};
use solana_client::{
    client_error::ClientError,
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use std::{collections::HashMap, future::Future, pin::Pin, sync::atomic::Ordering};

struct NoRpc(String);
impl RpcSender for NoRpc {
    fn send<'a, 'f>(
        &'a self,
        _: RpcRequest,
        _: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, ClientError>> + Send + 'f>>
    where
        'a: 'f,
        Self: 'f,
    {
        Box::pin(async { panic!("prepared profile classification must not call the SDK RPC") })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        self.0.clone()
    }
}
struct ExistingRiskGate;
impl TradeRiskGate for ExistingRiskGate {
    fn check_buy(&self, _: &sol_trade_sdk::TradeBuyParams) -> Result<()> {
        panic!("classification cannot consume the original SDK risk gate")
    }
}

fn client(url: &str) -> SdkTradeClient {
    let infrastructure = Arc::new(TradingInfrastructure {
        rpc: Arc::new(RpcClient::new_sender(
            NoRpc(url.into()),
            RpcClientConfig::default(),
        )),
        swqos_clients: Arc::new(vec![]),
        config: InfrastructureConfig::new(url.into(), vec![], CommitmentConfig::processed()),
        max_sender_concurrency: 1,
        effective_core_ids: Arc::new(vec![]),
    });
    // Same unfunded public fixture as existing Pump oracles; no SDK initializer or worker.
    let client = TradingClient::from_infrastructure(
        Arc::new(Keypair::new_from_array([11; 32])),
        infrastructure,
        false,
    );
    SdkTradeClient {
        lanes: vec![SdkLane {
            swqos_type: SwqosType::Default,
            is_helius: false,
            client,
        }],
        sol_hop_cache: tokio::sync::Mutex::new(HashMap::new()),
        swqos_lanes: vec!["Default".into()],
        mev_protection: false,
        require_mev_submit: false,
        rpc_count: 1,
        broadcast_policy: BroadcastPolicy::Enabled,
    }
}
fn context() -> SdkExecutionContext {
    SdkExecutionContext {
        blockhash: Blockhash::from_bytes([10; 32]),
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 0,
    }
}

#[tokio::test]
async fn pump_original_factory_unsupported_lane_profiles_return_before_any_rpc_or_claim() {
    let (f, _, _) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let server = LocalServer::new(Reply::ServerError).await;
    let runtime = Arc::new(SharedReadRuntime::new(&[server.url.clone()]).unwrap());
    for unsupported in ["multiple", "v1", "seed", "middleware", "risk"] {
        let mut wallet = client(&server.url);
        let lane = &mut wallet.lanes[0];
        match unsupported {
            "multiple" => {
                let client = lane.client.clone();
                wallet.lanes.push(SdkLane {
                    swqos_type: SwqosType::Default,
                    is_helius: false,
                    client,
                });
            }
            "v1" => lane.client.transaction_version = TradeTransactionVersion::V1,
            "seed" => lane.client.use_seed_optimize = true,
            "middleware" => {
                lane.client.middleware_manager = Some(Arc::new(MiddlewareManager::new()))
            }
            _ => lane.client.risk_gate = Some(Arc::new(ExistingRiskGate)),
        }
        assert!(
            wallet
                .prepare_original_pump(
                    &f.execution,
                    context(),
                    runtime.clone(),
                    f.state.persistence.ordinary_execution_claims().unwrap()
                )
                .await
                .unwrap()
                .is_none(),
            "{unsupported}"
        );
        assert_eq!(
            server.connections.load(Ordering::SeqCst),
            0,
            "{unsupported}"
        );
        assert!(server.requests.lock().unwrap().is_empty());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM ordinary_execution_claims")
                .fetch_one(&f.state.persistence.pool_for_tests())
                .await
                .unwrap(),
            0
        );
    }
    server.finish().await;
}

#[tokio::test]
async fn pump_original_factory_policy_and_wallet_binding_fail_before_reads_not_as_unsupported() {
    let (f, _, _) =
        PumpNativeCandidateRecord::history_fixture(TradeType::Buy, SwqosType::Default).await;
    let server = LocalServer::new(Reply::ServerError).await;
    let runtime = Arc::new(SharedReadRuntime::new(&[server.url.clone()]).unwrap());
    for failure in ["policy", "wallet"] {
        let mut wallet = client(&server.url);
        if failure == "policy" {
            wallet.broadcast_policy = BroadcastPolicy::Disabled;
            wallet.lanes.clear();
        } else {
            wallet.lanes[0].client.payer = Arc::new(Keypair::new_from_array([12; 32]));
        }
        assert!(
            wallet
                .prepare_original_pump(
                    &f.execution,
                    context(),
                    runtime.clone(),
                    f.state.persistence.ordinary_execution_claims().unwrap()
                )
                .await
                .is_err(),
            "{failure}"
        );
        assert_eq!(server.connections.load(Ordering::SeqCst), 0);
    }
    server.finish().await;
}
