use super::*;
use crate::adapters::rpc::{
    read_runtime::quote_status_fixture as f,
    sdk_signature::{observe_sdk_signature, observe_sdk_signature_shared},
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
fn reply(mode: Reply) -> Reply {
    Reply::QuoteStatus {
        value: f::found(),
        mode: Box::new(mode),
    }
}
fn released(r: &SharedReadRuntime) {
    assert_eq!(
        r.slots
            .iter()
            .map(|s| s.available_permits())
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
}
pub(super) async fn success() {
    for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(65_536)] {
        let old = LocalServer::new(reply(Reply::Success)).await;
        let actual = LocalServer::new(reply(mode)).await;
        let rpc = Arc::new(RpcClient::new_with_commitment(
            old.url.clone(),
            CommitmentConfig::confirmed(),
        ));
        let expected = observe_sdk_signature(rpc, &f::signature(), 1500)
            .await
            .unwrap()
            .unwrap();
        let r = runtime(&[&actual]);
        let found = observe_sdk_signature_shared(&r, &actual.url, &f::signature(), 1500)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (found.signature, found.slot, found.commitment, found.error),
            (
                expected.signature,
                expected.slot,
                expected.commitment,
                expected.error
            )
        );
        assert_eq!(
            old.requests.lock().unwrap()[0]["params"],
            actual.requests.lock().unwrap()[0]["params"]
        );
        assert_eq!((count(&old), count(&actual)), (1, 1));
        released(&r);
        old.finish().await;
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
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server, &unused]);
        let error = observe_sdk_signature_shared(&r, &server.url, &f::signature(), 1500)
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!((count(&server), count(&unused)), (1, 0));
        released(&r);
        server.finish().await;
    }
    unused.finish().await;
}
pub(super) async fn budgets_cancel() {
    for mode in [Reply::SlowHeaders, Reply::SlowBody] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server]);
        assert!(
            observe_sdk_signature_shared(&r, &server.url, &f::signature(), 30)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(count(&server), 1);
        released(&r);
        server.finish().await;
    }
    let good = LocalServer::new(reply(Reply::Success)).await;
    let hash = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&hash, &good]);
    assert!(
        observe_sdk_signature_shared(&r, &good.url, &f::signature(), 1500)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        observe_sdk_signature_shared(&r, &good.url, &f::signature(), 30)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    assert_eq!((count(&good), count(&hash)), (1, 1));
    released(&r);
    good.finish().await;
    hash.finish().await;

    let slow = LocalServer::new(reply(Reply::SlowHeaders)).await;
    let hash = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&hash, &slow]);
    let sig = f::signature();
    let mut pending = Box::pin(observe_sdk_signature_shared(&r, &slow.url, &sig, 10_000));
    tokio::select! { result=&mut pending=>panic!("unexpected {result:?}"),_=slow.received.notified()=>{} }
    assert!(r
        .wsol_signature_status(&hash.url, &sig.parse().unwrap())
        .await
        .is_err());
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    drop(pending);
    released(&r);
    assert_eq!((count(&slow), count(&hash)), (1, 1));
    slow.finish().await;
    hash.finish().await;
}
