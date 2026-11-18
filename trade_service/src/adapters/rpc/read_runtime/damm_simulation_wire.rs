use super::*;
use crate::adapters::rpc::damm_sell_simulation::{
    shared::{self, tests::transaction, DammSimulationReader},
    DammSimulationResult,
};
fn reply(mode: Reply) -> Reply {
    Reply::DammSimulation {
        result: shared::tests::result(),
        mode: Box::new(mode),
    }
}
async fn inspect(r: &SharedReadRuntime, url: &str) -> Result<DammSimulationResult> {
    shared::simulate_unsigned_at_slot(
        &DammSimulationReader::new(r, url.into()),
        &transaction(),
        123,
    )
    .await
}
pub(super) async fn success() {
    for mode in [
        Reply::Success,
        Reply::AtLimit,
        Reply::Chunked(524_288),
        Reply::DelayedHeaders(2200),
    ] {
        let delayed = matches!(mode, Reply::DelayedHeaders(_));
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server]);
        let start = Instant::now();
        assert!(inspect(&r, &server.url).await.unwrap().error.is_none());
        if delayed {
            assert!(start.elapsed() >= Duration::from_secs(2));
        }
        assert_eq!(count(&server), 1);
        assert_eq!(method(&server, 0), "simulateTransaction");
        assert_eq!(r.slots[10].available_permits(), 1);
        server.finish().await;
    }
}
pub(super) async fn errors() {
    let sink = LocalServer::new(Reply::Success).await;
    for mode in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::DeclaredOversize,
        Reply::Chunked(524_289),
        Reply::Disconnect,
        Reply::Redirect(sink.url.clone()),
        Reply::RateLimited(Some("60")),
    ] {
        let limited = matches!(mode, Reply::RateLimited(_));
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server, &sink]);
        let error = inspect(&r, &server.url).await.err().unwrap();
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!((count(&server), count(&sink)), (1, 0));
        assert_eq!(r.slots[10].available_permits(), 1);
        if limited {
            assert!(r.wsol_quote_slot(&server.url).is_err());
            assert!(inspect(&r, &server.url).await.is_err());
            assert_eq!(count(&server), 1);
        }
        server.finish().await;
    }
    assert_eq!(
        sink.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    sink.finish().await;
}
pub(super) async fn timeout_cancel() {
    for mode in [Reply::SlowHeaders, Reply::SlowBody] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server]);
        let start = Instant::now();
        assert!(inspect(&r, &server.url).await.is_err());
        assert!(start.elapsed() >= Duration::from_millis(3800));
        assert_eq!(count(&server), 1);
        assert_eq!(r.slots[10].available_permits(), 1);
        server.finish().await;
    }
    let server = LocalServer::new(reply(Reply::SlowBody)).await;
    let hash = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&hash, &server]);
    let next;
    {
        let pending = inspect(&r, &server.url);
        tokio::pin!(pending);
        tokio::select! {_=&mut pending=>panic!("held simulation completed"),_=server.received.notified()=>{}}
        next = r.budget.lock().unwrap().next_start[10];
        assert!(r.aggregator_simulation_slot(&server.url).is_err());
        assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    }
    assert_eq!(r.budget.lock().unwrap().next_start[10], next);
    assert_eq!(
        r.slots
            .iter()
            .map(|s| s.available_permits())
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
    assert_eq!((count(&server), count(&hash)), (1, 1));
    server.finish().await;
    hash.finish().await;
}
