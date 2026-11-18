use super::*;
use crate::adapters::rpc::sol_transfer_reads::SolTransferReadSession;
use base64::Engine;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{hash::Hash, message::Message, pubkey::Pubkey};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

fn reply(fail_method: Option<&'static str>, failure: Reply, normal: Reply) -> Reply {
    Reply::SolTransferPreparation {
        values: [
            ("getBalance".into(), json!({"context":{"slot":123},"value":1_000_000})),
            ("getLatestBlockhash".into(), json!({"context":{"slot":123},"value":{"blockhash":Hash::new_from_array([6;32]).to_string(),"lastValidBlockHeight":999}})),
            ("getFeeForMessage".into(), json!({"context":{"slot":123},"value":5000})),
            ("getMinimumBalanceForRentExemption".into(), json!(890_880)),
        ].into_iter().collect(),
        fail_method,
        failure: Box::new(failure),
        normal: Box::new(normal),
    }
}

async fn inspect(r: &SharedReadRuntime, url: &str) -> Result<(u64, Hash, u64, u64)> {
    SolTransferReadSession::new(r, url, CommitmentConfig::confirmed(), key(1), key(2))?
        .snapshot()
        .await
}

pub(super) async fn success() {
    for mode in [Reply::Success, Reply::AtLimit, Reply::Chunked(65_536)] {
        let server = LocalServer::new(reply(None, Reply::Success, mode)).await;
        let r = runtime(&[&server]);
        let snapshot = inspect(&r, &server.url).await.unwrap();
        assert_eq!(
            snapshot,
            (1_000_000, Hash::new_from_array([6; 32]), 5000, 890_880)
        );
        assert_eq!(count(&server), 4);
        {
            let requests = server.requests.lock().unwrap();
            assert_eq!(
                requests
                    .iter()
                    .map(|r| r["method"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                [
                    "getBalance",
                    "getLatestBlockhash",
                    "getFeeForMessage",
                    "getMinimumBalanceForRentExemption"
                ]
            );
            assert_eq!(
                requests[0]["params"],
                json!([key(1).to_string(),{"commitment":"confirmed"}])
            );
            assert_eq!(requests[1]["params"], json!([{"commitment":"confirmed"}]));
            assert_eq!(requests[3]["params"], json!([0]));
            let message = Message::new_with_blockhash(
                &[solana_system_interface::instruction::transfer(
                    &key(1),
                    &key(2),
                    1,
                )],
                Some(&key(1)),
                &snapshot.1,
            );
            assert_eq!(
                requests[2]["params"],
                json!([base64::engine::general_purpose::STANDARD.encode(message.serialize()),{"commitment":"confirmed"}])
            );
            for (index, request) in requests.iter().enumerate() {
                assert_eq!(request["id"], index + 1);
            }
        }
        assert_eq!(r.slots[11].available_permits(), 1);
        server.finish().await;
    }
}

pub(super) async fn errors() {
    let sink = LocalServer::new(Reply::Success).await;
    for (index, method) in [(0, "getBalance"), (3, "getMinimumBalanceForRentExemption")] {
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
            let server = LocalServer::new(reply(Some(method), mode, Reply::Success)).await;
            let r = runtime(&[&server, &sink]);
            let error = inspect(&r, &server.url).await.unwrap_err();
            assert!(!format!("{error:#}").contains("fixture-secret"));
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

pub(super) async fn deadlines() {
    // A legitimate response after 2s must still fit the original whole-lane 5s budget.
    let server = LocalServer::new(reply(
        Some("getBalance"),
        Reply::DelayedHeaders(2200),
        Reply::Success,
    ))
    .await;
    let r = runtime(&[&server]);
    assert!(inspect(&r, &server.url).await.is_ok());
    assert_eq!(count(&server), 4);
    server.finish().await;

    let slow = LocalServer::new(reply(None, Reply::Success, Reply::DelayedHeaders(1400))).await;
    let r = runtime(&[&slow]);
    let started = Instant::now();
    assert!(inspect(&r, &slow.url).await.is_err());
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(started.elapsed() < Duration::from_secs(7));
    assert_eq!(count(&slow), 4);
    assert_eq!(r.slots[11].available_permits(), 1);
    slow.finish().await;

    let slow = LocalServer::new(reply(Some("getBalance"), Reply::SlowBody, Reply::Success)).await;
    let hash = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&hash, &slow]);
    let work = {
        let r = r.clone();
        let url = slow.url.clone();
        tokio::spawn(async move { inspect(&r, &url).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while count(&slow) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(r.wsol_quote_slot(&slow.url).is_err());
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    work.abort();
    assert!(work.await.unwrap_err().is_cancelled());
    assert_eq!(
        r.slots
            .iter()
            .map(|slot| slot.available_permits())
            .collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1]
    );
    assert_eq!((count(&slow), count(&hash)), (1, 1));
    slow.finish().await;
    hash.finish().await;
}
