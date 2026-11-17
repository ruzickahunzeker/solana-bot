use super::*;
use crate::{
    adapters::rpc::wsol_status::WsolSignatureReader,
    application::ports::wsol_queries::WsolStatusReader,
};
fn reader(r: &Arc<SharedReadRuntime>, server: &LocalServer) -> WsolSignatureReader {
    WsolSignatureReader::new(r.clone(), server.url.clone(), signature().parse().unwrap())
}
pub(super) async fn success() {
    for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(65_536)] {
        let server = LocalServer::new(mode).await;
        let r = runtime(&[&server]);
        let observed = reader(&r, &server).lookup().await.unwrap().unwrap();
        assert_eq!(observed.chain.signature, signature());
        assert_eq!(observed.context_slot, 123);
        assert_eq!(observed.chain.slot, 123);
        assert_eq!(
            observed.outcome_for(&signature()).unwrap(),
            crate::domain::wsol_progress::WsolObservedOutcome::Confirmed
        );
        assert_eq!(count(&server), 1);
        assert_eq!(method(&server, 0), "getSignatureStatuses");
        assert_eq!(r.slots[Class::Observation.index()].available_permits(), 1);
        server.finish().await;
    }
}
pub(super) async fn errors() {
    for mode in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::DeclaredOversize,
        Reply::Chunked(65_537),
        Reply::RateLimited(Some("60")),
    ] {
        let server = LocalServer::new(mode.clone()).await;
        let backup = LocalServer::new(Reply::Success).await;
        let r = runtime(&[&server, &backup]);
        let error = reader(&r, &server).lookup().await.unwrap_err();
        assert!(!format!("{error:?}").contains("fixture-secret"));
        if matches!(mode, Reply::RateLimited(_)) {
            assert!(r.wsol_quote_slot(&server.url).is_err());
            assert!(reader(&r, &server).lookup().await.is_err());
        }
        assert_eq!((count(&server), count(&backup)), (1, 0));
        assert_eq!(r.slots[Class::Observation.index()].available_permits(), 1);
        server.finish().await;
        backup.finish().await;
    }
    let target = LocalServer::new(Reply::Success).await;
    let redirect = LocalServer::new(Reply::Redirect(target.url.clone())).await;
    let r = runtime(&[&redirect, &target]);
    assert!(reader(&r, &redirect).lookup().await.is_err());
    assert_eq!((count(&redirect), count(&target)), (1, 0));
    redirect.finish().await;
    target.finish().await;
}
pub(super) async fn timeout_cancel() {
    for mode in [Reply::SlowHeaders, Reply::SlowBody, Reply::Disconnect] {
        let server = LocalServer::new(mode).await;
        let hash = LocalServer::new(Reply::Success).await;
        let r = runtime(&[&hash, &server]);
        assert!(reader(&r, &server).lookup().await.is_err());
        assert_eq!(count(&server), 1);
        assert_eq!(r.slots[Class::Observation.index()].available_permits(), 1);
        assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
        assert_eq!(count(&hash), 1);
        server.finish().await;
        hash.finish().await;
    }
    let server = LocalServer::new(Reply::SlowBody).await;
    let hash = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&hash, &server]);
    {
        let a = reader(&r, &server);
        let pending = a.lookup();
        tokio::pin!(pending);
        tokio::select! { _ = &mut pending => panic!("slow body completed"), _ = server.received.notified() => {} }
        assert!(reader(&r, &server).lookup().await.is_err());
        assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    }
    assert_eq!(
        r.slots
            .iter()
            .map(|slot| slot.available_permits())
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
    assert_eq!((count(&server), count(&hash)), (1, 1));
    server.finish().await;
    hash.finish().await;
}
