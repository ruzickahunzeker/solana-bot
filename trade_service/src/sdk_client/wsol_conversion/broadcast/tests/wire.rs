use super::*;
use crate::adapters::test_support::{
    read_rpc_server::{LocalServer, Reply as WireReply},
    IsolatedTestProcess,
};

const FLAG: &str = "SOL_BOT_WSOL_BROADCAST_WIRE_CASE";
const CHILD: &str =
    "sdk_client::wsol_conversion::broadcast::tests::wire::wsol_broadcast_lane_wire_child";
const DONE: &str = "WSOL_BROADCAST_WIRE_DONE";

#[test]
fn wsol_broadcast_lane_wire_exact_original_http_params_one_and_two_signers() {
    let child = IsolatedTestProcess::spawn(CHILD, FLAG, "success");
    child.await_line(DONE);
    child.finish();
}

#[test]
fn wsol_broadcast_lane_wire_errors_and_cancellation_keep_original_packet() {
    let child = IsolatedTestProcess::spawn(CHILD, FLAG, "errors");
    child.await_line(DONE);
    child.finish();
}

#[tokio::test]
async fn wsol_broadcast_lane_wire_child() {
    let Ok(case) = std::env::var(FLAG) else {
        return;
    };
    assert_eq!(std::env::current_dir().unwrap(), std::env::temp_dir());
    match case.as_str() {
        "success" => success().await,
        "errors" => errors().await,
        _ => panic!("unknown fixture case"),
    }
    println!("{DONE}");
}

fn reply(signature: &str, mode: WireReply) -> WireReply {
    WireReply::WsolBroadcast {
        signature: signature.into(),
        mode: Box::new(mode),
    }
}
fn lane(server: &LocalServer, commitment: CommitmentConfig) -> WsolBroadcastLane {
    WsolBroadcastLane::from_selected(Arc::new(RpcClient::new_with_commitment(
        server.url.clone(),
        commitment,
    )))
}
async fn success() {
    for partial in [false, true] {
        for commitment in [
            CommitmentConfig::processed(),
            CommitmentConfig::confirmed(),
            CommitmentConfig::finalized(),
        ] {
            let signed = fixture::signed(partial);
            let before = signed.signed_bytes().to_vec();
            let original = LocalServer::new(reply(signed.signature(), WireReply::Success)).await;
            let actual = LocalServer::new(reply(signed.signature(), WireReply::Success)).await;
            let expected = RpcClient::new_with_commitment(original.url.clone(), commitment)
                .send_transaction(&signed.transaction)
                .await
                .unwrap();
            let actual_lane = lane(&actual, commitment);
            assert_eq!(
                actual_lane
                    .send(WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap())
                    .await
                    .unwrap(),
                expected
            );
            let old_calls = original.requests.lock().unwrap().clone();
            let calls = actual.requests.lock().unwrap().clone();
            // Compare full envelopes without normalizing IDs or default config fields.
            assert_eq!(calls, old_calls);
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0]["method"], "sendTransaction");
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(calls[0]["params"][0].as_str().unwrap())
                    .unwrap(),
                before
            );
            assert_eq!(signed.signed_bytes(), before);
            original.finish().await;
            actual.finish().await;
        }
    }
}
async fn errors() {
    for partial in [false, true] {
        let signed = fixture::signed(partial);
        let before = signed.signed_bytes().to_vec();
        for (signature, mode) in [
            (Signature::from([8; 64]).to_string(), WireReply::Success),
            ("invalid-signature".into(), WireReply::Success),
            (signed.signature().into(), WireReply::RpcError),
            (signed.signature().into(), WireReply::InvalidJson),
        ] {
            let server = LocalServer::new(reply(&signature, mode)).await;
            assert!(lane(&server, CommitmentConfig::confirmed())
                .send(WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap())
                .await
                .is_err());
            assert_eq!(server.requests.lock().unwrap().len(), 1);
            assert_eq!(signed.signed_bytes(), before);
            server.finish().await;
        }
        let server = LocalServer::new(reply(signed.signature(), WireReply::SlowHeaders)).await;
        let actual_lane = lane(&server, CommitmentConfig::confirmed());
        let mut pending = Box::pin(
            actual_lane.send(WsolBroadcastPacket::checked(&signed, &signed.owner).unwrap()),
        );
        tokio::select! {
            result = &mut pending => panic!("unexpected completion: {result:?}"),
            _ = server.received.notified() => {}
        }
        drop(pending);
        tokio::task::yield_now().await;
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        assert_eq!(signed.signed_bytes(), before);
        server.finish().await;
    }
}
