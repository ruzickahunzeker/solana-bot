use super::*;
use crate::adapters::rpc::read_runtime::preparation_balance::tests::{a, scan};
use crate::{
    adapters::{
        rpc::preparation_balance::RpcPreparationBalanceRead,
        sdk::preparation_assets::tests::raw_accounts,
    },
    application::ports::preparation_balance::PreparationBalanceRead,
};

fn reply(scan_reply: Reply, raw_reply: Reply, scan_value: Value, raw_value: Value) -> Reply {
    Reply::Preparation {
        scan: Box::new(scan_reply),
        raw: Box::new(raw_reply),
        scan_value,
        raw_value,
    }
}
fn normal(scan_reply: Reply, raw_reply: Reply) -> Reply {
    reply(
        scan_reply,
        raw_reply,
        scan(false, 1000),
        raw_accounts(false, 1000),
    )
}
fn port(server: &LocalServer, runtime: Arc<SharedReadRuntime>) -> RpcPreparationBalanceRead {
    RpcPreparationBalanceRead::new(a(1), server.url.clone(), runtime)
}

pub(super) async fn success() {
    for token2022 in [false, true] {
        let server = LocalServer::new(reply(
            Reply::AtLimit,
            Reply::AtLimit,
            scan(token2022, 9007199254740993),
            raw_accounts(token2022, 9007199254740993),
        ))
        .await;
        let r = runtime(&[&server]);
        let snapshot = port(&server, r).snapshot(a(9)).await.unwrap();
        assert_eq!(
            snapshot.checked_units(a(1), a(9)).unwrap(),
            9007199254740993
        );
        let calls = server.requests.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        assert_eq!(server.connections.load(Ordering::SeqCst), 2);
        assert_eq!(calls[0]["method"], "getTokenAccountsByOwner");
        assert_eq!(calls[1]["method"], "getMultipleAccounts");
        assert_eq!(
            calls[1]["params"][0],
            serde_json::json!(
                crate::adapters::sdk::preparation_assets::input_keys(a(1), a(9))
                    .map(|key| key.to_string())
            )
        );
        assert_eq!(calls[1]["params"][1]["minContextSlot"], 123);
        server.finish().await;
    }
}
pub(super) async fn fallback() {
    let mut unparsed = scan(false, 1000);
    unparsed[0]["account"]["data"] = serde_json::json!(["", "base64"]);
    for (scan_reply, value, minimum) in [
        (Reply::Success, unparsed, Some(123)),
        (Reply::RpcError, scan(false, 1000), None),
        (Reply::ServerError, scan(false, 1000), None),
    ] {
        let server = LocalServer::new(reply(
            scan_reply,
            Reply::Success,
            value,
            raw_accounts(false, 1000),
        ))
        .await;
        let snapshot = port(&server, runtime(&[&server]))
            .snapshot(a(9))
            .await
            .unwrap();
        assert_eq!(snapshot.checked_units(a(1), a(9)).unwrap(), 1000);
        assert_eq!(count(&server), 2);
        assert_eq!(
            server.requests.lock().unwrap()[1]["params"][1]["minContextSlot"],
            serde_json::json!(minimum)
        );
        server.finish().await;
    }
}
pub(super) async fn limits() {
    for (scan_reply, raw_reply, expected) in [
        (Reply::DeclaredOversize, Reply::Success, 1),
        (Reply::Chunked(524_289), Reply::Success, 1),
        (Reply::Success, Reply::DeclaredOversize, 2),
        (Reply::Success, Reply::Chunked(65_537), 2),
        (Reply::WrongId, Reply::Success, 1),
        (Reply::InvalidJson, Reply::Success, 1),
        (Reply::Success, Reply::WrongId, 2),
    ] {
        let server = LocalServer::new(normal(scan_reply, raw_reply)).await;
        let error = port(&server, runtime(&[&server]))
            .snapshot(a(9))
            .await
            .err()
            .unwrap();
        assert!(!error
            .downcast_ref::<crate::domain::preparation_balance::PreparationBalanceFailure>()
            .unwrap()
            .allows_lane_fallback());
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!(count(&server), expected);
        server.finish().await;
    }
    let mut wrong = scan(false, 1000);
    wrong[0]["account"]["data"]["parsed"]["info"]["mint"] = serde_json::json!(a(99).to_string());
    let server = LocalServer::new(reply(
        Reply::Success,
        Reply::Success,
        wrong,
        raw_accounts(false, 1000),
    ))
    .await;
    assert!(port(&server, runtime(&[&server]))
        .snapshot(a(9))
        .await
        .is_err());
    assert_eq!(count(&server), 1);
    server.finish().await;
}
pub(super) async fn timeouts_redirect() {
    for raw_slow in [false, true] {
        let server = LocalServer::new(if raw_slow {
            normal(Reply::Success, Reply::SlowBody)
        } else {
            normal(Reply::SlowHeaders, Reply::SlowHeaders)
        })
        .await;
        let started = Instant::now();
        assert!(port(&server, runtime(&[&server]))
            .snapshot(a(9))
            .await
            .is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(count(&server), 2);
        server.finish().await;
    }
    let target = LocalServer::new(Reply::Success).await;
    let server = LocalServer::new(normal(
        Reply::Redirect(target.url.clone()),
        Reply::Redirect(target.url.clone()),
    ))
    .await;
    assert!(port(&server, runtime(&[&server]))
        .snapshot(a(9))
        .await
        .is_err());
    assert_eq!(count(&server), 2);
    assert_eq!(target.connections.load(Ordering::SeqCst), 0);
    server.finish().await;
    target.finish().await;
}
pub(super) async fn rate_scope() {
    let server = LocalServer::new(normal(Reply::RateLimited(Some("60")), Reply::Success)).await;
    let backup = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&server, &backup]);
    assert!(port(&server, r.clone()).snapshot(a(9)).await.is_err());
    assert!(port(&server, r.clone()).snapshot(a(9)).await.is_err());
    assert_eq!((count(&server), count(&backup)), (1, 0));
    r.latest_blockhash().await.unwrap();
    assert_eq!((count(&server), count(&backup)), (1, 1));
    server.finish().await;
    backup.finish().await;
}
pub(super) async fn priority_cancel() {
    let server = LocalServer::new(normal(Reply::Success, Reply::SlowBody)).await;
    let r = runtime(&[&server]);
    let reader = port(&server, r.clone());
    let task = tokio::spawn(async move { reader.snapshot(a(9)).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while count(&server) < 2 {
            server.received.notified().await;
        }
    })
    .await
    .unwrap();
    assert!(port(&server, r.clone())
        .snapshot(a(9))
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("busy"));
    r.latest_blockhash().await.unwrap();
    assert_eq!(count(&server), 3);
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(r.slots[5].available_permits(), 1);
    server.finish().await;
}
