use super::*;
use crate::adapters::rpc::{
    read_http::{HttpReply, ReadHttpTransport, ReadRequest},
    read_runtime::SharedReadRuntime,
};
use reqwest::{StatusCode, Url};
use solana_commitment_config::CommitmentConfig;
use tokio::sync::Notify;

const PRIMARY: &str = "http://native-transfer.invalid/primary";
const BACKUP: &str = "http://native-transfer.invalid/backup";

#[derive(Clone)]
pub(super) struct Endpoint {
    pub(super) rpc: MemoryRpc,
    pub(super) url: &'static str,
}
impl Endpoint {
    pub(super) fn client(&self, commitment: CommitmentConfig) -> Arc<RpcClient> {
        Arc::new(RpcClient::new_sender(
            self.clone(),
            RpcClientConfig {
                commitment_config: commitment,
                ..Default::default()
            },
        ))
    }
}
impl RpcSender for Endpoint {
    fn send<'life0, 'async_trait>(
        &'life0 self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = RpcResult<Value>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        self.rpc.send(request, params)
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        self.rpc.get_transport_stats()
    }
    fn url(&self) -> String {
        self.url.into()
    }
}

#[derive(Clone)]
struct Transport {
    primary: Endpoint,
    backup: Endpoint,
    entered: Arc<Notify>,
}
impl Transport {
    fn new(primary: MemoryRpc, backup: MemoryRpc) -> Self {
        Self {
            primary: Endpoint {
                rpc: primary,
                url: PRIMARY,
            },
            backup: Endpoint {
                rpc: backup,
                url: BACKUP,
            },
            entered: Arc::default(),
        }
    }
    fn runtime(&self) -> SharedReadRuntime<Self> {
        SharedReadRuntime::with_test_transport(&[PRIMARY.into(), BACKUP.into()], self.clone())
    }
    fn endpoints(&self, commitment: CommitmentConfig) -> Vec<Arc<RpcClient>> {
        vec![
            self.primary.client(commitment),
            self.backup.client(commitment),
        ]
    }
}
impl ReadHttpTransport for Transport {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        assert!(matches!(
            request,
            ReadRequest::SolTransferPreparation { .. }
        ));
        let body = request.test_body();
        let method = match body["method"].as_str().unwrap() {
            "getBalance" => RpcRequest::GetBalance,
            "getLatestBlockhash" => RpcRequest::GetLatestBlockhash,
            "getFeeForMessage" => RpcRequest::GetFeeForMessage,
            "getMinimumBalanceForRentExemption" => RpcRequest::GetMinimumBalanceForRentExemption,
            other => panic!("forbidden transfer read method {other}"),
        };
        let rpc = if url.as_str() == PRIMARY {
            &self.primary.rpc
        } else {
            assert_eq!(url.as_str(), BACKUP);
            &self.backup.rpc
        };
        self.entered.notify_one();
        let result = rpc.send(method, body["params"].clone()).await?;
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":result})).unwrap(),
        ))
    }
}

fn calls(rpc: &MemoryRpc) -> Vec<(RpcRequest, Value)> {
    rpc.calls.lock().unwrap().clone()
}

fn assert_same(prepared: &PreparedSolTransfer, original: &PreparedSolTransfer) {
    assert_eq!(prepared.snapshot, original.snapshot);
    assert_eq!(prepared.message, original.message);
    assert_eq!(prepared.blockhash, original.blockhash);
    assert_eq!(prepared.rent_exempt_minimum, original.rent_exempt_minimum);
    assert_eq!(prepared.fixed_lamports, original.fixed_lamports);
    assert_eq!(prepared.percent, original.percent);
    prepared.ensure_source(&source()).unwrap();
    assert_eq!(
        Transaction::new_unsigned(prepared.message.clone()).signatures,
        [Signature::default()]
    );
}

#[tokio::test]
async fn sol_transfer_adapter_native_amount_fee_rent_commitment_and_message_match_original() {
    for commitment in [
        CommitmentConfig::processed(),
        CommitmentConfig::confirmed(),
        CommitmentConfig::finalized(),
    ] {
        for balance in [1_000_000, 9_007_199_254_740_993, u64::MAX] {
            for (fixed, percent) in [
                (Some(1), None),
                (Some(94_120), None),
                (None, Some(50)),
                (None, Some(100)),
            ] {
                let original_rpc = MemoryRpc {
                    balance,
                    ..Default::default()
                };
                let original = prepare_with_rpcs(
                    source(),
                    destination(),
                    fixed,
                    percent,
                    std::iter::once(original_rpc.client_with_commitment(commitment)),
                    PREPARE_TIMEOUT,
                )
                .await
                .unwrap();
                let transport = Transport::new(
                    MemoryRpc {
                        balance,
                        ..Default::default()
                    },
                    MemoryRpc::default(),
                );
                let runtime = transport.runtime();
                let rpc = transport.primary.client(commitment);
                let prepared = preparation::prepare(
                    source(),
                    destination(),
                    fixed,
                    percent,
                    std::iter::once(rpc.clone()),
                    &runtime,
                    PREPARE_TIMEOUT,
                )
                .await
                .unwrap();
                assert_same(&prepared, &original);
                assert!(Arc::ptr_eq(&prepared.rpc, &rpc));
                assert_eq!(calls(&transport.primary.rpc), calls(&original_rpc));
                assert!(transport.backup.rpc.methods().is_empty());
                transport.primary.rpc.read_only();
                original_rpc.read_only();
            }
        }
    }
}

#[tokio::test]
async fn sol_transfer_adapter_rpc_failure_uses_original_whole_lane_fallback_not_partial_evidence() {
    for method in [
        RpcRequest::GetBalance,
        RpcRequest::GetLatestBlockhash,
        RpcRequest::GetFeeForMessage,
        RpcRequest::GetMinimumBalanceForRentExemption,
    ] {
        let original_primary = MemoryRpc {
            fail: Some(method),
            ..Default::default()
        };
        let original_backup = MemoryRpc::default();
        let original = prepare_with_rpcs(
            source(),
            destination(),
            Some(1),
            None,
            vec![original_primary.client(), original_backup.client()].into_iter(),
            PREPARE_TIMEOUT,
        )
        .await
        .unwrap();
        let transport = Transport::new(
            MemoryRpc {
                fail: Some(method),
                ..Default::default()
            },
            MemoryRpc::default(),
        );
        let runtime = transport.runtime();
        let endpoints = transport.endpoints(CommitmentConfig::processed());
        let selected = endpoints[1].clone();
        let prepared = preparation::prepare(
            source(),
            destination(),
            Some(1),
            None,
            endpoints.into_iter(),
            &runtime,
            PREPARE_TIMEOUT,
        )
        .await
        .unwrap();
        assert_same(&prepared, &original);
        assert!(Arc::ptr_eq(&prepared.rpc, &selected));
        assert_eq!(calls(&transport.primary.rpc), calls(&original_primary));
        assert_eq!(calls(&transport.backup.rpc), calls(&original_backup));
        transport.primary.rpc.read_only();
        transport.backup.rpc.read_only();
    }
}

#[tokio::test]
async fn sol_transfer_adapter_amount_error_is_terminal_after_same_selected_snapshot() {
    for (balance, fixed, percent) in [
        (0, Some(1), None),
        (1_000_000, Some(0), None),
        (1_000_000, None, Some(0)),
        (1_000_000, None, Some(101)),
        (1_000_000, Some(u64::MAX), None),
        (1_000_000, Some(1), Some(50)),
    ] {
        let original_primary = MemoryRpc {
            balance,
            ..Default::default()
        };
        let original_backup = MemoryRpc::default();
        let original = prepare_with_rpcs(
            source(),
            destination(),
            fixed,
            percent,
            vec![original_primary.client(), original_backup.client()].into_iter(),
            PREPARE_TIMEOUT,
        )
        .await
        .err()
        .unwrap();
        let transport = Transport::new(
            MemoryRpc {
                balance,
                ..Default::default()
            },
            MemoryRpc::default(),
        );
        let runtime = transport.runtime();
        let error = preparation::prepare(
            source(),
            destination(),
            fixed,
            percent,
            transport
                .endpoints(CommitmentConfig::processed())
                .into_iter(),
            &runtime,
            PREPARE_TIMEOUT,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.to_string(), original.to_string());
        assert_eq!(calls(&transport.primary.rpc), calls(&original_primary));
        assert!(transport.backup.rpc.methods().is_empty());
        assert!(original_backup.methods().is_empty());
        transport.primary.rpc.read_only();
    }
}

#[tokio::test]
async fn sol_transfer_adapter_self_empty_lanes_and_original_timeout_keep_error_order() {
    let transport = Transport::new(
        MemoryRpc {
            pending: Some(RpcRequest::GetBalance),
            ..Default::default()
        },
        MemoryRpc::default(),
    );
    let runtime = transport.runtime();
    for same in [true, false] {
        let error = preparation::prepare(
            source(),
            if same { source() } else { destination() },
            Some(1),
            None,
            std::iter::empty(),
            &runtime,
            PREPARE_TIMEOUT,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            if same {
                "source and destination wallets must be different"
            } else {
                "unable to prepare SOL transfer: no Default RPC lane configured"
            }
        );
    }
    assert!(transport.primary.rpc.methods().is_empty());
    let error = preparation::prepare(
        source(),
        destination(),
        Some(1),
        None,
        std::iter::once(transport.primary.client(CommitmentConfig::processed())),
        &runtime,
        Duration::from_millis(2),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(
        error.to_string(),
        "unable to prepare SOL transfer: RPC transfer preparation timed out"
    );
    assert_eq!(transport.primary.rpc.methods(), [RpcRequest::GetBalance]);
    assert!(runtime.wsol_quote_slot(PRIMARY).is_ok());
    assert!(transport.backup.rpc.methods().is_empty());
}

#[tokio::test]
async fn sol_transfer_adapter_cancel_retains_no_prepared_or_send_authority() {
    let transport = Transport::new(
        MemoryRpc {
            pending: Some(RpcRequest::GetBalance),
            ..Default::default()
        },
        MemoryRpc::default(),
    );
    let runtime = transport.runtime();
    let rpc = transport.primary.client(CommitmentConfig::processed());
    let mut work = Box::pin(preparation::prepare(
        source(),
        destination(),
        Some(1),
        None,
        std::iter::once(rpc),
        &runtime,
        PREPARE_TIMEOUT,
    ));
    tokio::select! {
        result = &mut work => panic!("pending transfer completed: {}", result.is_ok()),
        _ = transport.entered.notified() => {},
    }
    assert!(runtime.wsol_quote_slot(PRIMARY).is_err());
    drop(work);
    assert!(runtime.wsol_quote_slot(PRIMARY).is_ok());
    assert_eq!(transport.primary.rpc.methods(), [RpcRequest::GetBalance]);
    transport.primary.rpc.read_only();
    assert!(transport.backup.rpc.methods().is_empty());
}

#[tokio::test]
async fn sol_transfer_adapter_failed_original_lane_does_not_fund_from_earlier_balance_or_fee() {
    let original_primary = MemoryRpc {
        balance: 9_000_000,
        fail: Some(RpcRequest::GetMinimumBalanceForRentExemption),
        ..Default::default()
    };
    let original_backup = MemoryRpc {
        balance: 2_000_000,
        ..Default::default()
    };
    let original = prepare_with_rpcs(
        source(),
        destination(),
        None,
        Some(100),
        vec![original_primary.client(), original_backup.client()].into_iter(),
        PREPARE_TIMEOUT,
    )
    .await
    .unwrap();
    let transport = Transport::new(
        MemoryRpc {
            balance: 9_000_000,
            fail: Some(RpcRequest::GetMinimumBalanceForRentExemption),
            ..Default::default()
        },
        MemoryRpc {
            balance: 2_000_000,
            ..Default::default()
        },
    );
    let runtime = transport.runtime();
    let endpoints = transport.endpoints(CommitmentConfig::processed());
    let selected = endpoints[1].clone();
    let prepared = preparation::prepare(
        source(),
        destination(),
        None,
        Some(100),
        endpoints.into_iter(),
        &runtime,
        PREPARE_TIMEOUT,
    )
    .await
    .unwrap();
    assert_same(&prepared, &original);
    assert_eq!(prepared.snapshot.balance_before_lamports, 2_000_000);
    assert_eq!(prepared.snapshot.amount_lamports, 1_995_000);
    assert!(Arc::ptr_eq(&prepared.rpc, &selected));
    assert_eq!(calls(&transport.primary.rpc), calls(&original_primary));
    assert_eq!(calls(&transport.backup.rpc), calls(&original_backup));
    transport.primary.rpc.read_only();
    transport.backup.rpc.read_only();
}
