use super::*;
use crate::adapters::rpc::sol_transfer_confirmation::SolTransferConfirmationReader;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;

fn reply(mode: Reply) -> Reply {
    Reply::SolTransferConfirmation {
        value: json!({"context":{"slot":42},"value":[{
            "slot":42,"confirmations":1,"status":{"Ok":null},"err":null,"confirmationStatus":"confirmed",
        }]}),
        mode: Box::new(mode),
    }
}

async fn confirm(r: &SharedReadRuntime, url: &str) -> Result<bool> {
    SolTransferConfirmationReader::new(
        r,
        url.into(),
        Signature::from([7; 64]),
        CommitmentConfig::confirmed(),
    )
    .confirm(Duration::from_secs(15))
    .await
}

pub(super) async fn success() {
    for mode in [
        Reply::Success,
        Reply::AtLimit,
        Reply::Chunked(65_536),
        Reply::DelayedHeaders(2200),
    ] {
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server]);
        assert!(confirm(&r, &server.url).await.unwrap());
        assert_eq!(
            *server.requests.lock().unwrap(),
            [json!({
                "jsonrpc":"2.0","id":1,"method":"getSignatureStatuses",
                "params":[[Signature::from([7;64]).to_string()]],
            })]
        );
        assert_eq!(r.slots[Class::Observation.index()].available_permits(), 1);
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
        Reply::Chunked(65_537),
        Reply::Disconnect,
        Reply::Redirect(sink.url.clone()),
        Reply::RateLimited(Some("60")),
    ] {
        let limited = matches!(mode, Reply::RateLimited(_));
        let server = LocalServer::new(reply(mode)).await;
        let r = runtime(&[&server, &sink]);
        let error = confirm(&r, &server.url).await.unwrap_err();
        assert!(!format!("{error:#}").contains("fixture-secret"));
        assert_eq!((count(&server), count(&sink)), (1, 0));
        assert_eq!(r.slots[Class::Observation.index()].available_permits(), 1);
        if limited {
            assert!(r
                .wsol_signature_status(&server.url, &Signature::from([7; 64]))
                .await
                .is_err());
            assert_eq!(count(&server), 1);
        }
        server.finish().await;
    }
    sink.finish().await;
}

pub(super) async fn cancel() {
    let slow = LocalServer::new(reply(Reply::SlowBody)).await;
    let hash = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&hash, &slow]);
    let work = {
        let r = r.clone();
        let url = slow.url.clone();
        tokio::spawn(async move { confirm(&r, &url).await })
    };
    tokio::time::timeout(Duration::from_secs(2), slow.received.notified())
        .await
        .unwrap();
    assert_eq!(r.slots[Class::Observation.index()].available_permits(), 0);
    assert!(r
        .wsol_signature_status(&slow.url, &Signature::from([7; 64]))
        .await
        .is_err());
    assert_eq!(r.latest_blockhash().await.unwrap().to_bytes(), [7; 32]);
    work.abort();
    assert!(work.await.unwrap_err().is_cancelled());
    assert_eq!(r.slots[Class::Observation.index()].available_permits(), 1);
    assert_eq!((count(&slow), count(&hash)), (1, 1));
    slow.finish().await;
    hash.finish().await;
}
