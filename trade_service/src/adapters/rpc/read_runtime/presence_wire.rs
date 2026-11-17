use super::*;
use crate::adapters::rpc::read_runtime::preparation_balance::tests::a;

async fn exists(r: &SharedReadRuntime, url: &str) -> anyhow::Result<bool> {
    r.preparation_account_exists(url, a(1), a(9)).await
}

// Frozen V1 SDK read/predicate, without constructing any TradingClient or signer.
async fn legacy(url: String) -> bool {
    use solana_account_decoder::UiAccountData;
    use solana_client::{nonblocking::rpc_client::RpcClient, rpc_request::TokenAccountsFilter};
    let rpc = RpcClient::new_with_commitment(
        url,
        solana_commitment_config::CommitmentConfig::processed(),
    );
    rpc.get_token_accounts_by_owner(
        &solana_sdk::pubkey::Pubkey::new_from_array(a(1).to_bytes()),
        TokenAccountsFilter::Mint(solana_sdk::pubkey::Pubkey::new_from_array(a(9).to_bytes())),
    )
    .await
    .unwrap()
    .into_iter()
    .any(|account| match account.account.data {
        UiAccountData::Json(parsed) => {
            parsed.program == "spl-token" || parsed.program == "spl-token-2022"
        }
        _ => false,
    })
}

pub(super) async fn parity() {
    for (reply, expected) in [
        (Reply::Success, true),
        (Reply::QuoteEmpty, false),
        (Reply::QuoteUnparsed, false),
        (Reply::QuoteMixed, true),
        (Reply::QuoteWrongMint, true),
        (Reply::AtLimit, true),
        (Reply::Chunked(524_288), true),
    ] {
        let old = LocalServer::new(reply.clone()).await;
        let new = LocalServer::new(reply).await;
        assert_eq!(legacy(old.url.clone()).await, expected);
        assert_eq!(exists(&runtime(&[&new]), &new.url).await.unwrap(), expected);
        assert_eq!((count(&old), count(&new)), (1, 1));
        let old_body = old.requests.lock().unwrap()[0].clone();
        let new_body = new.requests.lock().unwrap()[0].clone();
        assert_eq!(old_body["method"], new_body["method"]);
        assert_eq!(old_body["params"], new_body["params"]);
        old.finish().await;
        new.finish().await;
    }
}

pub(super) async fn limits() {
    for reply in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::Disconnect,
        Reply::DeclaredOversize,
        Reply::Chunked(524_289),
    ] {
        let bad = LocalServer::new(reply).await;
        let backup = LocalServer::new(Reply::Success).await;
        let r = runtime(&[&bad, &backup]);
        let error = exists(&r, &bad.url).await.err().unwrap();
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!((count(&bad), count(&backup)), (1, 0));
        bad.finish().await;
        backup.finish().await;
    }
}

pub(super) async fn timeouts() {
    for reply in [Reply::SlowHeaders, Reply::SlowBody] {
        let slow = LocalServer::new(reply).await;
        let r = runtime(&[&slow]);
        let started = Instant::now();
        assert!(exists(&r, &slow.url).await.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(count(&slow), 1);
        assert_eq!(r.slots[5].available_permits(), 1);
        slow.finish().await;
    }
    let sink = LocalServer::new(Reply::Success).await;
    let redirect = LocalServer::new(Reply::Redirect(sink.url.clone())).await;
    let r = runtime(&[&redirect, &sink]);
    assert!(exists(&r, &redirect.url).await.is_err());
    assert_eq!((count(&redirect), count(&sink)), (1, 0));
    redirect.finish().await;
    sink.finish().await;
}

pub(super) async fn rate_cancel() {
    let limited = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let r = runtime(&[&limited, &backup]);
    assert!(exists(&r, &limited.url).await.is_err());
    assert!(exists(&r, &limited.url).await.is_err());
    assert!(r
        .preparation_balance(&limited.url, a(1), a(9))
        .await
        .is_err());
    assert_eq!((count(&limited), count(&backup)), (1, 0));
    r.latest_blockhash().await.unwrap();
    assert_eq!((count(&limited), count(&backup)), (1, 1));
    limited.finish().await;
    backup.finish().await;

    let slow = LocalServer::new(Reply::Preparation {
        scan: Box::new(Reply::SlowHeaders),
        raw: Box::new(Reply::Success),
        scan_value: serde_json::json!([]),
        raw_value: serde_json::json!([]),
    })
    .await;
    let r = runtime(&[&slow]);
    let child = r.clone();
    let url = slow.url.clone();
    let task = tokio::spawn(async move { exists(&child, &url).await });
    tokio::time::timeout(Duration::from_secs(1), slow.received.notified())
        .await
        .unwrap();
    assert!(exists(&r, &slow.url)
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("busy"));
    assert!(r
        .preparation_balance(&slow.url, a(1), a(9))
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("busy"));
    r.latest_blockhash().await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert_eq!(r.slots[5].available_permits(), 1);
    assert_eq!(count(&slow), 2);
    assert_eq!(method(&slow, 0), "getTokenAccountsByOwner");
    assert_eq!(method(&slow, 1), "getLatestBlockhash");
    slow.finish().await;
}
