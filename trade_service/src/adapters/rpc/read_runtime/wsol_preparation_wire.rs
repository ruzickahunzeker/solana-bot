use super::wsol_quote_wire::reply;
use super::*;
use crate::{
    adapters::rpc::wsol_quote_reads::tests::Fixture,
    application::wsol_submission_types::WsolDirection,
    sdk_client::wsol_conversion::{
        preparation::{
            self,
            tests::{assert_prepared, quote},
        },
        UnsignedWsolTransaction,
    },
};
use solana_commitment_config::CommitmentConfig;
async fn inspect(
    r: &SharedReadRuntime,
    url: &str,
    f: &Fixture,
    direction: WsolDirection,
    amount: u64,
) -> Result<UnsignedWsolTransaction> {
    preparation::prepare(
        f.owner,
        r,
        url,
        CommitmentConfig::confirmed(),
        &quote(f, direction, amount),
    )
    .await
}
pub(super) async fn success() {
    for (token, direction, amount, mode) in [
        (None, WsolDirection::Wrap, 1_000_000, Reply::Success),
        (
            Some(2_000_000),
            WsolDirection::Wrap,
            1_000_000,
            Reply::AtLimit,
        ),
        (
            Some(2_000_000),
            WsolDirection::Unwrap,
            2_000_000,
            Reply::Chunked(65_536),
        ),
        (
            Some(2_000_000),
            WsolDirection::Unwrap,
            1_000_000,
            Reply::Success,
        ),
    ] {
        let mut f = Fixture::default();
        f.token = token;
        let server = LocalServer::new(reply(&f, None, Reply::Success, mode)).await;
        let r = runtime(&[&server]);
        let p = inspect(&r, &server.url, &f, direction, amount)
            .await
            .unwrap();
        assert_prepared(&p, &f, &quote(&f, direction, amount));
        assert_eq!(count(&server), 5);
        assert_eq!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|v| v["method"].as_str().unwrap().to_string())
                .collect::<Vec<_>>(),
            [
                "getBalance",
                "getAccountInfo",
                "getMinimumBalanceForRentExemption",
                "getLatestBlockhash",
                "getFeeForMessage"
            ]
        );
        assert_eq!(r.slots[11].available_permits(), 1);
        server.finish().await;
    }
}
pub(super) async fn errors() {
    let f = Fixture::default();
    let sink = LocalServer::new(Reply::Success).await;
    for (index, method) in [(0, "getBalance"), (4, "getFeeForMessage")] {
        for mode in [
            Reply::WrongId,
            Reply::InvalidJson,
            Reply::RpcError,
            Reply::ServerError,
            Reply::DeclaredOversize,
            Reply::Chunked(65_537),
            Reply::Disconnect,
            Reply::Redirect(sink.url.clone()),
            Reply::RateLimited(Some("60")),
        ] {
            let limited = matches!(mode, Reply::RateLimited(_));
            let server = LocalServer::new(reply(&f, Some(method), mode, Reply::Success)).await;
            let r = runtime(&[&server, &sink]);
            let err = inspect(&r, &server.url, &f, WsolDirection::Wrap, 1_000_000)
                .await
                .err()
                .unwrap();
            assert!(!format!("{err:?}").contains("fixture-secret"));
            assert_eq!((count(&server), count(&sink)), (index + 1, 0));
            assert_eq!(r.slots[11].available_permits(), 1);
            if limited {
                assert!(r.wsol_quote_slot(&server.url).is_err());
            }
            server.finish().await;
        }
    }
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
        assert!(inspect(&r, &server.url, &f, WsolDirection::Wrap, 1_000_000)
            .await
            .is_err());
        assert_eq!(count(&server), index + 1);
        assert_eq!(r.slots[11].available_permits(), 1);
        server.finish().await;
    }
    let slow = LocalServer::new(reply(
        &f,
        Some("getFeeForMessage"),
        Reply::SlowBody,
        Reply::Success,
    ))
    .await;
    let hash = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&hash, &slow]);
    let task = {
        let r = r.clone();
        let f = f.clone();
        let url = slow.url.clone();
        tokio::spawn(async move { inspect(&r, &url, &f, WsolDirection::Unwrap, 1_000_000).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if count(&slow) == 5 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(r.wsol_quote_slot(&slow.url).is_err());
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(
        r.slots
            .iter()
            .map(|slot| slot.available_permits())
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
    assert_eq!((count(&slow), count(&hash)), (5, 1));
    slow.finish().await;
    hash.finish().await;
}
