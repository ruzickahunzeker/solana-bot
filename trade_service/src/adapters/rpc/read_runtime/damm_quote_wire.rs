use super::*;
use crate::adapters::rpc::damm_quote_reads::{
    tests::{self as fixture, Legacy},
    DammQuoteReader, DammQuoteRpc,
};

fn reply(account_mode: Reply, probe_mode: Reply, count: usize) -> Reply {
    Reply::DammQuote {
        accounts: fixture::accounts(count),
        probe: fixture::simulation(count),
        account_mode: Box::new(account_mode),
        probe_mode: Box::new(probe_mode),
    }
}

pub(super) async fn success() {
    for size in [8, 9] {
        for minimum in [None, Some(123)] {
            for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(524_288)] {
                let server = LocalServer::new(reply(mode.clone(), mode, size)).await;
                let r = runtime(&[&server]);
                let reader = DammQuoteReader::new(r.as_ref(), server.url.clone());
                let old = Legacy::new(fixture::accounts(size), fixture::simulation(size));
                let keys = fixture::keys(size as u8);
                let actual = reader.damm_accounts(&keys, minimum).await.unwrap();
                let expected = old.client().damm_accounts(&keys, minimum).await.unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
                let actual = reader
                    .damm_probe(&fixture::transaction(), &keys, 123, true)
                    .await
                    .unwrap();
                let expected = old
                    .client()
                    .damm_probe(&fixture::transaction(), &keys, 123, true)
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
                assert_eq!(count(&server), 2);
                assert_eq!(method(&server, 0), "getMultipleAccounts");
                assert_eq!(method(&server, 1), "simulateTransaction");
                let old = old.calls.lock().unwrap().clone();
                let new = server.requests.lock().unwrap().clone();
                for i in 0..2 {
                    assert_eq!(old[i].1, new[i]["params"]);
                }
                assert_eq!(
                    (
                        r.slots[6].available_permits(),
                        r.slots[10].available_permits()
                    ),
                    (1, 1)
                );
                server.finish().await;
            }
        }
    }
}

pub(super) async fn errors() {
    let sink = LocalServer::new(Reply::Success).await;
    for probe in [false, true] {
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
            let server = LocalServer::new(if probe {
                reply(Reply::Success, mode, 8)
            } else {
                reply(mode, Reply::Success, 8)
            })
            .await;
            let r = runtime(&[&server, &sink]);
            let reader = DammQuoteReader::new(r.as_ref(), server.url.clone());
            let error = if probe {
                reader
                    .damm_probe(&fixture::transaction(), &fixture::keys(8), 123, true)
                    .await
                    .err()
                    .unwrap()
            } else {
                reader
                    .damm_accounts(&fixture::keys(8), Some(123))
                    .await
                    .err()
                    .unwrap()
            };
            assert!(!format!("{error:?}").contains("fixture-secret"));
            assert_eq!((count(&server), count(&sink)), (1, 0));
            assert_eq!(
                (
                    r.slots[6].available_permits(),
                    r.slots[10].available_permits()
                ),
                (1, 1)
            );
            if limited {
                assert!(reader
                    .damm_probe(&fixture::transaction(), &fixture::keys(8), 123, true)
                    .await
                    .is_err());
                assert!(reader
                    .damm_accounts(&fixture::keys(8), Some(123))
                    .await
                    .is_err());
                assert!(r.wsol_quote_slot(&server.url).is_err());
                assert_eq!((count(&server), count(&sink)), (1, 0));
            }
            server.finish().await;
        }
    }
    assert_eq!(
        sink.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    sink.finish().await;
}

pub(super) async fn timeout_cancel() {
    for mode in [Reply::SlowHeaders, Reply::SlowBody] {
        let server = LocalServer::new(reply(mode, Reply::Success, 8)).await;
        let r = runtime(&[&server]);
        let reader = DammQuoteReader::new(r.as_ref(), server.url.clone());
        let start = Instant::now();
        assert!(reader
            .damm_accounts(&fixture::keys(8), Some(123))
            .await
            .is_err());
        assert!(start.elapsed() >= Duration::from_millis(4800));
        assert_eq!(r.slots[6].available_permits(), 1);
        assert_eq!(count(&server), 1);
        server.finish().await;
    }
    for probe in [false, true] {
        let server = LocalServer::new(reply(Reply::SlowHeaders, Reply::SlowHeaders, 8)).await;
        let hash = LocalServer::new(Reply::Success).await;
        let r = runtime(&[&hash, &server]);
        let reader = DammQuoteReader::new(r.as_ref(), server.url.clone());
        {
            let pending = async {
                if probe {
                    reader
                        .damm_probe(&fixture::transaction(), &fixture::keys(8), 123, true)
                        .await
                        .map(|_| ())
                } else {
                    reader
                        .damm_accounts(&fixture::keys(8), Some(123))
                        .await
                        .map(|_| ())
                }
            };
            tokio::pin!(pending);
            tokio::select! {_=&mut pending=>panic!("held quote read completed"),_=server.received.notified()=>{}}
            assert_eq!(r.slots[if probe { 10 } else { 6 }].available_permits(), 0);
            assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
        }
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
}

pub(super) async fn long_budgets() {
    let server = LocalServer::new(reply(
        Reply::DelayedHeaders(2200),
        Reply::DelayedHeaders(5200),
        8,
    ))
    .await;
    let r = runtime(&[&server]);
    let reader = DammQuoteReader::new(r.as_ref(), server.url.clone());
    let start = Instant::now();
    reader
        .damm_accounts(&fixture::keys(8), Some(123))
        .await
        .unwrap();
    assert!(start.elapsed() >= Duration::from_secs(2));
    let start = Instant::now();
    reader
        .damm_probe(&fixture::transaction(), &fixture::keys(8), 123, true)
        .await
        .unwrap();
    assert!(start.elapsed() >= Duration::from_secs(5));
    assert_eq!(count(&server), 2);
    server.finish().await;
}
