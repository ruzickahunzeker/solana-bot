use super::{shared_reads::Transport, *};
use solana_commitment_config::CommitmentConfig;

fn send_requests(rpc: &MemoryRpc) -> Vec<Value> {
    rpc.calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(method, _)| *method == RpcRequest::SendTransaction)
        .map(|(_, params)| params.clone())
        .collect()
}

async fn selected_backup(
    transport: &Transport,
    commitment: CommitmentConfig,
) -> (SignedSolTransfer, Arc<RpcClient>) {
    let runtime = transport.runtime();
    let endpoints = transport.endpoints(commitment);
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
    assert!(prepared.broadcast.is_selected(&selected));
    (synthetic_signed(prepared), selected)
}

#[tokio::test]
async fn sol_transfer_broadcast_ownership_uses_selected_backup_and_original_sdk_request() {
    for (commitment, name) in [
        (CommitmentConfig::processed(), "processed"),
        (CommitmentConfig::confirmed(), "confirmed"),
        (CommitmentConfig::finalized(), "finalized"),
    ] {
        let primary = MemoryRpc {
            fail: Some(RpcRequest::GetMinimumBalanceForRentExemption),
            ..Default::default()
        };
        let backup = MemoryRpc {
            allow_funds: true,
            ..Default::default()
        };
        let transport = Transport::new(primary.clone(), backup.clone());
        let (signed, selected) = selected_backup(&transport, commitment).await;
        assert_eq!(signed.prepared.broadcast.url(), selected.url());
        assert_eq!(
            signed.prepared.broadcast.commitment(),
            selected.commitment()
        );
        let original_signature = signed.submission().signature.clone();
        let original_bytes = signed.submission().bytes.clone();
        let receipt = broadcast_signed(&signed, &source(), SEND_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(receipt.to_string(), original_signature);
        assert_eq!(signed.submission().bytes, original_bytes);
        assert_eq!(signed.submission().signature, original_signature);
        assert_eq!(primary.methods().len(), 4);
        assert_eq!(backup.methods().len(), 5);
        primary.read_only();
        let sent = send_requests(&backup);
        assert_eq!(sent.len(), 1);
        assert!(!backup.methods().contains(&RpcRequest::GetSignatureStatuses));
        let oracle = MemoryRpc {
            allow_funds: true,
            ..Default::default()
        };
        assert_eq!(
            oracle
                .client_with_commitment(commitment)
                .send_transaction(&signed.transaction)
                .await
                .unwrap(),
            receipt
        );
        assert_eq!(sent, send_requests(&oracle));
        assert_eq!(
            &sent[0][0],
            &json!(base64::engine::general_purpose::STANDARD.encode(&original_bytes))
        );
        assert_eq!(
            sent[0][1],
            json!({"encoding":"base64","skipPreflight":false,"preflightCommitment":name,"maxRetries":null,"minContextSlot":null})
        );
    }
}

#[tokio::test]
async fn sol_transfer_broadcast_ownership_guard_prevents_io_to_either_endpoint() {
    let primary = MemoryRpc {
        fail: Some(RpcRequest::GetBalance),
        ..Default::default()
    };
    let backup = MemoryRpc::default();
    let transport = Transport::new(primary.clone(), backup.clone());
    let (mut signed, _) = selected_backup(&transport, CommitmentConfig::processed()).await;
    let error = broadcast_signed(&signed, &destination(), SEND_TIMEOUT)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "prepared SOL transfer belongs to a different wallet"
    );
    signed.submission.bytes.push(0);
    let error = broadcast_signed(&signed, &source(), SEND_TIMEOUT)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "signed SOL transfer does not match its immutable submission"
    );
    assert_eq!(primary.methods(), [RpcRequest::GetBalance]);
    assert_eq!(backup.methods().len(), 4);
    primary.read_only();
    backup.read_only();
}

#[tokio::test]
async fn sol_transfer_broadcast_ownership_uncertain_results_do_not_resend_or_rebind() {
    for fault in 0..3 {
        let primary = MemoryRpc {
            fail: Some(RpcRequest::GetBalance),
            ..Default::default()
        };
        let backup = MemoryRpc {
            allow_funds: true,
            fail: (fault == 0).then_some(RpcRequest::SendTransaction),
            pending: (fault == 1).then_some(RpcRequest::SendTransaction),
            receipt: Signature::from([9; 64]),
            ..Default::default()
        };
        let transport = Transport::new(primary.clone(), backup.clone());
        let (signed, _) = selected_backup(&transport, CommitmentConfig::processed()).await;
        let bytes = signed.submission().bytes.clone();
        let signature = signed.submission().signature.clone();
        let error = broadcast_signed(&signed, &source(), Duration::from_millis(50))
            .await
            .unwrap_err();
        if fault == 1 {
            assert_eq!(error.to_string(), "SOL transfer submission timed out");
        } else {
            assert_eq!(error.to_string(), "SOL transfer submission failed: balance=1000000, amount=1, fee=5000, reserve=900880 lamports");
            if fault == 2 {
                assert!(format!("{error:#}").contains("RPC node returned mismatched signature"));
            }
        }
        assert_eq!(signed.submission().bytes, bytes);
        assert_eq!(signed.submission().signature, signature);
        assert_eq!(send_requests(&backup).len(), 1);
        assert_eq!(backup.methods().len(), 5);
        assert!(!backup.methods().contains(&RpcRequest::GetSignatureStatuses));
        assert_eq!(primary.methods(), [RpcRequest::GetBalance]);
        primary.read_only();
    }
}
