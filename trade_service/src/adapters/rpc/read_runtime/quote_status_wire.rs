use super::*;
use crate::adapters::rpc::read_runtime::quote_status_fixture as fixture;
use crate::{
    adapters::rpc::{
        quote_status::RpcQuoteStatusReader, signature_status::RpcSignatureStatusReader,
    },
    application::ports::reconciliation::SignatureStatusReader,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;

fn reply(mode: Reply) -> Reply {
    Reply::QuoteStatus {
        value: fixture::found(),
        mode: Box::new(mode),
    }
}
fn reader(runtime: Arc<SharedReadRuntime>, servers: &[&LocalServer]) -> RpcQuoteStatusReader {
    RpcQuoteStatusReader::new(runtime, servers.iter().map(|s| s.url.clone()).collect())
}
pub(super) async fn success() {
    for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(65_536)] {
        let original = LocalServer::new(reply(Reply::Success)).await;
        let actual = LocalServer::new(reply(mode)).await;
        let old = RpcSignatureStatusReader {
            rpcs: vec![Arc::new(RpcClient::new_with_commitment(
                original.url.clone(),
                CommitmentConfig::confirmed(),
            ))],
        };
        let expected = old
            .lookup_signature(&fixture::signature())
            .await
            .unwrap()
            .unwrap();
        let r = runtime(&[&actual]);
        let observed = reader(r.clone(), &[&actual])
            .lookup_signature(&fixture::signature())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                observed.signature,
                observed.commitment,
                observed.slot,
                observed.error
            ),
            (
                expected.signature,
                expected.commitment,
                expected.slot,
                expected.error
            )
        );
        let original_body = original.requests.lock().unwrap()[0].clone();
        let actual_body = actual.requests.lock().unwrap()[0].clone();
        // Runtime correlation IDs are global; method/params remain exact, with no policy normalization.
        assert_eq!(actual_body["params"], original_body["params"]);
        assert_eq!(actual_body["method"], original_body["method"]);
        assert_eq!(actual_body["jsonrpc"], "2.0");
        assert!(actual_body["id"].as_u64().is_some());
        assert_eq!((count(&original), count(&actual)), (1, 1));
        assert_eq!(r.slots[1].available_permits(), 1);
        original.finish().await;
        actual.finish().await;
    }
}
pub(super) async fn errors() {
    let unused = LocalServer::new(Reply::Success).await;
    for mode in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::DeclaredOversize,
        Reply::Chunked(65_537),
        Reply::Disconnect,
        Reply::Redirect(unused.url.clone()),
        Reply::RateLimited(Some("60")),
    ] {
        let bad = LocalServer::new(reply(mode)).await;
        let backup = LocalServer::new(reply(Reply::Success)).await;
        let r = runtime(&[&bad, &backup, &unused]);
        assert!(reader(r.clone(), &[&bad, &backup, &unused])
            .lookup_signature(&fixture::signature())
            .await
            .unwrap()
            .is_some());
        assert_eq!((count(&bad), count(&backup), count(&unused)), (1, 1, 0));
        assert_eq!(r.slots[1].available_permits(), 1);
        bad.finish().await;
        backup.finish().await;
    }
    unused.finish().await;
}
pub(super) async fn timeout_cancel_rate() {
    for mode in [Reply::SlowHeaders, Reply::SlowBody] {
        let slow = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&slow]);
        assert_eq!(
            reader(r.clone(), &[&slow])
                .lookup_signature(&fixture::signature())
                .await
                .unwrap_err()
                .to_string(),
            "signature status RPC timed out"
        );
        assert_eq!(count(&slow), 1);
        assert_eq!(r.slots[1].available_permits(), 1);
        slow.finish().await;
    }
    for quote_first in [false, true] {
        let limited = LocalServer::new(Reply::RateLimited(Some("60"))).await;
        let good = LocalServer::new(Reply::Success).await;
        let r = runtime(&[&limited, &good]);
        if quote_first {
            assert!(reader(r.clone(), &[&limited, &good])
                .lookup_signature(&fixture::signature())
                .await
                .unwrap()
                .is_some());
        }
        assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
        if !quote_first {
            assert!(reader(r.clone(), &[&limited, &good])
                .lookup_signature(&fixture::signature())
                .await
                .unwrap()
                .is_some());
        }
        assert_eq!((count(&limited), count(&good)), (1, 2));
        limited.finish().await;
        good.finish().await;
    }
    let slow = LocalServer::new(Reply::SlowHeaders).await;
    let good = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&slow, &good]);
    let status = reader(r.clone(), &[&slow]);
    let sig = fixture::signature();
    let mut pending = Box::pin(status.lookup_signature(&sig));
    tokio::select! { result=&mut pending=>panic!("unexpected completion {result:?}"),_=slow.received.notified()=>{} }
    assert!(r
        .wsol_signature_status(&good.url, &sig.parse().unwrap())
        .await
        .is_err());
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    drop(pending);
    assert_eq!(
        r.slots
            .iter()
            .map(|s| s.available_permits())
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
    assert_eq!((count(&slow), count(&good)), (2, 1));
    slow.finish().await;
    good.finish().await;
}
