use super::*;
use crate::adapters::rpc::{
    read_http::{HttpReply, ReadHttpTransport, ReadRequest},
    read_runtime::SharedReadRuntime,
};
use reqwest::{StatusCode, Url};
use solana_commitment_config::CommitmentConfig;

const PRIMARY: &str = "http://native-confirm.invalid/primary";
const BACKUP: &str = "http://native-confirm.invalid/backup";

#[derive(Clone)]
struct ConfirmationFixture {
    rpc: MemoryRpc,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

impl ConfirmationFixture {
    fn new(rpc: MemoryRpc) -> Self {
        Self {
            rpc,
            calls: Arc::default(),
        }
    }

    fn runtime(&self) -> SharedReadRuntime<Self> {
        SharedReadRuntime::with_test_transport(&[PRIMARY.into(), BACKUP.into()], self.clone())
    }

    async fn signed(&self, commitment: CommitmentConfig) -> SignedSolTransfer {
        let sender = shared_reads::Endpoint {
            rpc: self.rpc.clone(),
            url: BACKUP,
        };
        synthetic_signed(
            prepare_with_rpcs(
                source(),
                destination(),
                Some(1),
                None,
                std::iter::once(sender.client(commitment)),
                PREPARE_TIMEOUT,
            )
            .await
            .unwrap(),
        )
    }
}

impl ReadHttpTransport for ConfirmationFixture {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        assert!(matches!(
            request,
            ReadRequest::SolTransferConfirmation { .. }
        ));
        assert_eq!(url.as_str(), BACKUP);
        let body = request.test_body();
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), body.clone()));
        let value = self
            .rpc
            .send(RpcRequest::GetSignatureStatuses, body["params"].clone())
            .await?;
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&json!({
                "jsonrpc":"2.0","id":body["id"],"result":value,
            }))
            .unwrap(),
        ))
    }
}

#[tokio::test]
async fn sol_transfer_confirm_adapter_preserves_selected_endpoint_commitment_and_original_artifact()
{
    for (commitment, expected) in [
        (CommitmentConfig::processed(), true),
        (CommitmentConfig::confirmed(), true),
        (CommitmentConfig::finalized(), false),
    ] {
        let fixture = ConfirmationFixture::new(MemoryRpc {
            allow_funds: true,
            ..Default::default()
        });
        let signed = fixture.signed(commitment).await;
        let signature = signed.submission().signature.parse().unwrap();
        let bytes = signed.submission().bytes.clone();
        let snapshot = signed.snapshot().clone();
        let original = confirm_signed(&signed, &signature, CONFIRM_TIMEOUT).await;
        let runtime = fixture.runtime();
        let actual = confirmation::confirm(&signed, &signature, &runtime, CONFIRM_TIMEOUT).await;
        assert_eq!(actual, original);
        assert_eq!(actual, expected);
        assert_eq!(&signed.submission().bytes, &bytes);
        assert_eq!(signed.snapshot(), &snapshot);
        let calls = fixture.calls.lock().unwrap();
        assert_eq!(
            *calls,
            [(
                BACKUP.to_string(),
                json!({"jsonrpc":"2.0","id":1,"method":"getSignatureStatuses","params":[[signature.to_string()]]})
            )]
        );
        let raw = fixture.rpc.calls.lock().unwrap();
        assert_eq!(
            raw.iter()
                .filter(|(method, _)| *method == RpcRequest::GetSignatureStatuses)
                .map(|(_, params)| params.clone())
                .collect::<Vec<_>>(),
            [calls[0].1["params"].clone(), calls[0].1["params"].clone()]
        );
        assert!(!raw
            .iter()
            .any(|(method, _)| *method == RpcRequest::SendTransaction));
    }
}

#[tokio::test]
async fn sol_transfer_confirm_adapter_signature_mismatch_or_unregistered_lane_never_reads() {
    let fixture = ConfirmationFixture::new(MemoryRpc::default());
    let signed = fixture.signed(CommitmentConfig::processed()).await;
    let runtime = fixture.runtime();
    assert!(
        !confirmation::confirm(
            &signed,
            &Signature::from([8; 64]),
            &runtime,
            CONFIRM_TIMEOUT
        )
        .await
    );
    let runtime = SharedReadRuntime::with_test_transport(&[PRIMARY.into()], fixture.clone());
    assert!(
        !confirmation::confirm(
            &signed,
            &signed.submission().signature.parse().unwrap(),
            &runtime,
            CONFIRM_TIMEOUT
        )
        .await
    );
    assert!(fixture.calls.lock().unwrap().is_empty());
    fixture.rpc.read_only();
}

#[tokio::test]
async fn sol_transfer_confirm_adapter_missing_rpc_error_and_timeout_remain_unconfirmed() {
    for fault in 0..3 {
        let fixture = ConfirmationFixture::new(MemoryRpc {
            allow_funds: true,
            confirmed: false,
            fail: (fault == 1).then_some(RpcRequest::GetSignatureStatuses),
            pending: (fault == 2).then_some(RpcRequest::GetSignatureStatuses),
            ..Default::default()
        });
        let signed = fixture.signed(CommitmentConfig::processed()).await;
        let runtime = fixture.runtime();
        let signature = signed.submission().signature.parse().unwrap();
        let budget = if fault == 2 {
            Duration::from_millis(100)
        } else {
            CONFIRM_TIMEOUT
        };
        assert!(!confirm_signed(&signed, &signature, budget).await);
        assert!(!confirmation::confirm(&signed, &signature, &runtime, budget).await);
        assert_eq!(fixture.calls.lock().unwrap().len(), 1);
        let methods = fixture.rpc.methods();
        assert_eq!(
            methods
                .iter()
                .filter(|method| **method == RpcRequest::GetSignatureStatuses)
                .count(),
            2
        );
        assert!(!methods.contains(&RpcRequest::SendTransaction));
    }
}
