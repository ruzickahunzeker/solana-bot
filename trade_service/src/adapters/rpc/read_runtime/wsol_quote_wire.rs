use super::*;
use crate::{
    adapters::{rpc::wsol_quote_reads::tests::Fixture, sdk::wsol_quote},
    application::{wsol_queries::WsolQuoteEstimate, wsol_submission_types::WsolDirection},
};
fn reply(f: &Fixture, method: Option<&'static str>, failure: Reply, normal: Reply) -> Reply {
    Reply::WsolQuote {
        values: [
            "getBalance",
            "getAccountInfo",
            "getMinimumBalanceForRentExemption",
            "getLatestBlockhash",
            "getFeeForMessage",
        ]
        .into_iter()
        .map(|method| (method.to_string(), f.result(method)))
        .collect(),
        fail_method: method,
        failure: Box::new(failure),
        normal: Box::new(normal),
    }
}
async fn inspect(
    r: &SharedReadRuntime,
    url: &str,
    f: &Fixture,
    direction: WsolDirection,
    all: bool,
) -> Result<WsolQuoteEstimate> {
    wsol_quote::estimate(
        r,
        url,
        solana_commitment_config::CommitmentConfig::confirmed(),
        f.owner,
        &Fixture::input(direction, if all { None } else { Some(1_000_000) }, all),
    )
    .await
}
pub(super) async fn success() {
    for (token, direction, all, mode) in [
        (None, WsolDirection::Wrap, false, Reply::Success),
        (Some(2_000_000), WsolDirection::Wrap, false, Reply::AtLimit),
        (
            Some(2_000_000),
            WsolDirection::Unwrap,
            true,
            Reply::Chunked(65_536),
        ),
        (
            Some(2_000_000),
            WsolDirection::Unwrap,
            false,
            Reply::Success,
        ),
    ] {
        let mut f = Fixture::default();
        f.token = token;
        let server = LocalServer::new(reply(&f, None, Reply::Success, mode)).await;
        let r = runtime(&[&server]);
        let report = inspect(&r, &server.url, &f, direction, all).await.unwrap();
        assert_eq!(
            report.amount_lamports,
            if all { 2_000_000 } else { 1_000_000 }
        );
        assert_eq!(report.estimated_network_fee_lamports, 5000);
        assert_eq!(count(&server), 5);
        let calls = server.requests.lock().unwrap();
        assert_eq!(calls[0]["params"][0], f.owner.to_string());
        assert_eq!(calls[1]["params"][0], f.ata().to_string());
        assert_eq!(calls[2]["params"], serde_json::json!([165]));
        assert_eq!(calls[3]["method"], "getLatestBlockhash");
        assert_eq!(calls[4]["method"], "getFeeForMessage");
        drop(calls);
        assert_eq!(r.slots[11].available_permits(), 1);
        server.finish().await;
    }
}
pub(super) async fn errors() {
    let f = Fixture::default();
    let sink = LocalServer::new(Reply::Success).await;
    for (index, method) in [
        (0, "getBalance"),
        (1, "getAccountInfo"),
        (4, "getFeeForMessage"),
    ] {
        for mode in [
            Reply::WrongId,
            Reply::InvalidJson,
            Reply::RpcError,
            Reply::DeclaredOversize,
            Reply::Chunked(65_537),
            Reply::Redirect(sink.url.clone()),
        ] {
            let server = LocalServer::new(reply(&f, Some(method), mode, Reply::Success)).await;
            let r = runtime(&[&server, &sink]);
            let err = inspect(&r, &server.url, &f, WsolDirection::Wrap, false)
                .await
                .unwrap_err();
            assert!(!format!("{err:?}").contains("fixture-secret"));
            assert_eq!((count(&server), count(&sink)), (index + 1, 0));
            assert_eq!(r.slots[11].available_permits(), 1);
            server.finish().await;
        }
    }
    for (index, method) in [(0, "getBalance"), (4, "getFeeForMessage")] {
        let server = LocalServer::new(reply(
            &f,
            Some(method),
            Reply::RateLimited(Some("60")),
            Reply::Success,
        ))
        .await;
        let r = runtime(&[&server, &sink]);
        assert!(inspect(&r, &server.url, &f, WsolDirection::Wrap, false)
            .await
            .is_err());
        r.latest_blockhash().await.unwrap();
        assert_eq!(count(&server), index + 1);
        server.finish().await;
    }
    assert_eq!(count(&sink), 2);
    sink.finish().await;
}
pub(super) async fn timeout_cancel() {
    let f = Fixture::default();
    for (index, method, mode) in [
        (0, "getBalance", Reply::SlowHeaders),
        (4, "getFeeForMessage", Reply::SlowBody),
    ] {
        let server = LocalServer::new(reply(&f, Some(method), mode, Reply::Success)).await;
        let r = runtime(&[&server]);
        assert!(inspect(&r, &server.url, &f, WsolDirection::Wrap, false)
            .await
            .is_err());
        assert_eq!(count(&server), index + 1);
        assert_eq!(r.slots[11].available_permits(), 1);
        server.finish().await;
    }
    for (index, method) in [(0, "getBalance"), (4, "getFeeForMessage")] {
        let slow =
            LocalServer::new(reply(&f, Some(method), Reply::SlowHeaders, Reply::Success)).await;
        let r = runtime(&[&slow]);
        let task = {
            let r = r.clone();
            let f = f.clone();
            let url = slow.url.clone();
            tokio::spawn(async move { inspect(&r, &url, &f, WsolDirection::Wrap, false).await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if count(&slow) == index + 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(r.slots[11].available_permits(), 0);
        r.latest_blockhash().await.unwrap();
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert_eq!(r.slots[11].available_permits(), 1);
        assert_eq!(count(&slow), index + 2);
        slow.finish().await;
    }
}
