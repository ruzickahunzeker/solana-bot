use super::*;
use crate::adapters::test_support::{
    read_rpc_server::{LocalServer, Reply},
    IsolatedTestProcess,
};

const FLAG: &str = "SOL_BOT_SHARED_READ_WIRE_CASE";
const CHILD: &str = "adapters::rpc::read_runtime::tests::wire_tests::shared_read_wire_child";
const DONE: &str = "SHARED_READ_WIRE_DONE";

#[path = "preparation_wire.rs"]
mod preparation_wire;

fn runtime(servers: &[&LocalServer]) -> Arc<SharedReadRuntime> {
    Arc::new(
        SharedReadRuntime::new(
            &servers
                .iter()
                .map(|server| server.url.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
}
fn count(server: &LocalServer) -> usize {
    server.requests.lock().unwrap().len()
}
fn method(server: &LocalServer, index: usize) -> String {
    server.requests.lock().unwrap()[index]["method"]
        .as_str()
        .unwrap()
        .into()
}

async fn hash_rate() {
    let limited = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&limited, &backup]);
    assert_eq!(
        RpcBlockhashReader::new(runtime.clone())
            .latest_blockhash()
            .await
            .unwrap()
            .to_bytes(),
        [7; 32]
    );
    assert_eq!((count(&limited), count(&backup)), (1, 1));
    let reader = SingleShotStatusReader::with_runtime(runtime, Duration::from_secs(1)).unwrap();
    assert!(reader
        .lookup_foreground(&signature())
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("cooling down"));
    assert_eq!(count(&limited), 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(reader
        .lookup_foreground(&signature())
        .await
        .unwrap()
        .is_some());
    assert_eq!((count(&limited), count(&backup)), (1, 2));
    assert_eq!(method(&backup, 0), "getLatestBlockhash");
    assert_eq!(method(&backup, 1), "getSignatureStatuses");
    limited.finish().await;
    backup.finish().await;
}

async fn status_rate() {
    let limited = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&limited, &backup]);
    let reader =
        SingleShotStatusReader::with_runtime(runtime.clone(), Duration::from_secs(1)).unwrap();
    assert!(reader
        .lookup_foreground(&signature())
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("429"));
    assert_eq!((count(&limited), count(&backup)), (1, 0));
    assert_eq!(
        runtime.latest_blockhash().await.unwrap().to_bytes(),
        [7; 32]
    );
    assert_eq!((count(&limited), count(&backup)), (1, 1));
    assert_eq!(method(&limited, 0), "getSignatureStatuses");
    assert_eq!(method(&backup, 0), "getLatestBlockhash");
    limited.finish().await;
    backup.finish().await;
}

async fn priority() {
    let slow = LocalServer::new(Reply::SlowHeaders).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&slow, &backup]);
    let reader = Arc::new(
        SingleShotStatusReader::with_runtime(runtime.clone(), Duration::from_secs(1)).unwrap(),
    );
    let status = {
        let reader = reader.clone();
        tokio::spawn(async move { reader.lookup_foreground(&signature()).await })
    };
    tokio::time::timeout(Duration::from_secs(1), slow.received.notified())
        .await
        .unwrap();
    let hash = tokio::time::timeout(Duration::from_millis(1800), runtime.latest_blockhash())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hash.to_bytes(), [7; 32]);
    assert!(!status.is_finished());
    status.abort();
    assert!(status.await.unwrap_err().is_cancelled());
    assert_eq!((count(&slow), count(&backup)), (2, 1));
    assert_eq!(method(&slow, 0), "getSignatureStatuses");
    assert_eq!(method(&slow, 1), "getLatestBlockhash");
    slow.finish().await;
    backup.finish().await;
}

async fn bad_replies() {
    for reply in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::Disconnect,
    ] {
        let bad = LocalServer::new(reply).await;
        let backup = LocalServer::new(Reply::Success).await;
        let runtime = runtime(&[&bad, &backup]);
        assert_eq!(
            runtime.latest_blockhash().await.unwrap().to_bytes(),
            [7; 32]
        );
        assert_eq!((count(&bad), count(&backup)), (1, 1));
        let first = bad.requests.lock().unwrap()[0]["id"].as_u64().unwrap();
        let second = backup.requests.lock().unwrap()[0]["id"].as_u64().unwrap();
        assert!(second > first);
        bad.finish().await;
        backup.finish().await;
    }
}

struct CompletedReader {
    reader: RpcBlockhashReader,
    done: Arc<Notify>,
}
impl LatestBlockhashReader for CompletedReader {
    async fn latest_blockhash(&self) -> Result<Blockhash> {
        let result = self.reader.latest_blockhash().await;
        self.done.notify_one();
        result
    }
}
async fn cache_preserved() {
    use crate::blockhash_cache::BlockhashCache;
    for existing in [false, true] {
        let bad = LocalServer::new(Reply::WrongId).await;
        let runtime = runtime(&[&bad]);
        let cache = Arc::new(if existing {
            BlockhashCache::with_snapshot_for_tests(Blockhash::from_bytes([9; 32]), 100)
        } else {
            BlockhashCache::default()
        });
        let done = Arc::new(Notify::new());
        let guard = cache.clone().spawn_refresher(
            CompletedReader {
                reader: RpcBlockhashReader::new(runtime),
                done: done.clone(),
            },
            10_000,
        );
        tokio::time::timeout(Duration::from_secs(2), done.notified())
            .await
            .unwrap();
        guard.shutdown().await.unwrap();
        assert!(cache.get_fresh(50).is_none());
        if existing {
            let snapshot = cache.get_fresh(u64::MAX).unwrap();
            assert_eq!(snapshot.hash.to_bytes(), [9; 32]);
            assert_eq!(snapshot.updated_at_ms, 100);
        } else {
            assert!(cache.age_ms().is_none());
        }
        assert_eq!(count(&bad), 1);
        bad.finish().await;
    }
}

async fn body_limits() {
    for (reply, good) in [
        (Reply::AtLimit, true),
        (Reply::Chunked(65_536), true),
        (Reply::DeclaredOversize, false),
        (Reply::Chunked(65_537), false),
    ] {
        let first = LocalServer::new(reply).await;
        let backup = LocalServer::new(Reply::Success).await;
        let runtime = runtime(&[&first, &backup]);
        assert_eq!(
            runtime.latest_blockhash().await.unwrap().to_bytes(),
            [7; 32]
        );
        assert_eq!((count(&first), count(&backup)), (1, usize::from(!good)));
        first.finish().await;
        backup.finish().await;
    }
}

async fn all_cooled() {
    let one = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let two = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let runtime = runtime(&[&one, &two]);
    assert!(runtime.latest_blockhash().await.is_err());
    let before = runtime.budget.lock().unwrap().cooldowns.clone();
    assert!(runtime.latest_blockhash().await.is_err());
    let reader =
        SingleShotStatusReader::with_runtime(runtime.clone(), Duration::from_secs(1)).unwrap();
    assert!(reader.lookup_foreground(&signature()).await.is_err());
    assert_eq!((count(&one), count(&two)), (1, 1));
    assert_eq!(runtime.budget.lock().unwrap().cooldowns, before);
    one.finish().await;
    two.finish().await;
}

async fn readiness_rate_scope() {
    let limited = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&limited, &backup]);
    let restricted = vec![limited.url.clone()];
    assert!(!runtime.readiness_ping(&restricted).await);
    let cooldowns = runtime.budget.lock().unwrap().cooldowns.clone();
    assert_eq!((count(&limited), count(&backup)), (1, 0));
    assert!(!runtime.readiness_ping(&restricted).await);
    assert!(runtime
        .signature_status(&Url::parse(&limited.url).unwrap(), 456, &signature())
        .await
        .is_err());
    assert!(runtime.latest_blockhash().await.is_ok());
    assert_eq!((count(&limited), count(&backup)), (1, 1));
    assert_eq!(runtime.budget.lock().unwrap().cooldowns, cooldowns);
    let request = limited.requests.lock().unwrap()[0].clone();
    assert_eq!(request["method"], "getLatestBlockhash");
    assert_eq!(request["params"], json!([{"commitment":"processed"}]));
    limited.finish().await;
    backup.finish().await;
}

async fn readiness_hash_cooldown() {
    let limited = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&limited, &backup]);
    runtime.latest_blockhash().await.unwrap();
    let cooldowns = runtime.budget.lock().unwrap().cooldowns.clone();
    assert!(!runtime.readiness_ping(&[limited.url.clone()]).await);
    assert!(runtime.readiness_ping(&[backup.url.clone()]).await);
    assert_eq!((count(&limited), count(&backup)), (1, 2));
    assert_eq!(runtime.budget.lock().unwrap().cooldowns, cooldowns);
    limited.finish().await;
    backup.finish().await;
}

async fn readiness_busy_priority_cancel() {
    let slow = LocalServer::new(Reply::SlowHeaders).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&slow, &backup]);
    let candidates = vec![slow.url.clone(), backup.url.clone()];
    let ready = {
        let runtime = runtime.clone();
        let candidates = candidates.clone();
        tokio::spawn(async move { runtime.readiness_ping(&candidates).await })
    };
    tokio::time::timeout(Duration::from_secs(1), slow.received.notified())
        .await
        .unwrap();
    assert!(!tokio::time::timeout(
        Duration::from_millis(200),
        runtime.readiness_ping(&candidates)
    )
    .await
    .unwrap());
    assert_eq!((count(&slow), count(&backup)), (1, 0));
    let hash = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.latest_blockhash().await })
    };
    tokio::time::timeout(Duration::from_secs(1), slow.received.notified())
        .await
        .unwrap();
    assert_eq!(count(&slow), 2);
    let next = runtime.budget.lock().unwrap().next_start[2];
    ready.abort();
    assert!(ready.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.budget.lock().unwrap().next_start[2], next);
    assert_eq!(runtime.slots[2].available_permits(), 1);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), hash)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .to_bytes(),
        [7; 32]
    );
    assert_eq!((count(&slow), count(&backup)), (2, 1));
    slow.finish().await;
    backup.finish().await;
}

async fn readiness_bad_replies() {
    for reply in [
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::Disconnect,
        Reply::DeclaredOversize,
        Reply::Chunked(65_537),
    ] {
        let bad = LocalServer::new(reply).await;
        let backup = LocalServer::new(Reply::Success).await;
        let runtime = runtime(&[&bad, &backup]);
        // Even a healthy configured node cannot rescue an out-of-scope execution lane.
        assert!(!runtime.readiness_ping(&[bad.url.clone()]).await);
        assert_eq!((count(&bad), count(&backup)), (1, 0));
        assert!(
            runtime
                .readiness_ping(&[bad.url.clone(), backup.url.clone()])
                .await
        );
        assert_eq!((count(&bad), count(&backup)), (2, 1));
        bad.finish().await;
        backup.finish().await;
    }
}

fn balance_addresses() -> Vec<crate::domain::trade::Address> {
    (0..201)
        .map(|byte| crate::domain::trade::Address::from_bytes([byte; 32]))
        .collect()
}
fn balance_expected(wallets: &[crate::domain::trade::Address]) -> Vec<u64> {
    wallets
        .iter()
        .map(|wallet| match wallet.to_bytes()[0] {
            1 => u64::MAX,
            byte => u64::from(byte),
        })
        .collect()
}
async fn native_pages() {
    use crate::{
        adapters::rpc::wallet_reads::RpcWalletReader,
        application::ports::wallet_reads::WalletReadPort,
    };
    let server = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&server]);
    let reader = RpcWalletReader::new(
        crate::domain::trade::Address::from_bytes([9; 32]),
        vec![Arc::new(
            solana_client::nonblocking::rpc_client::RpcClient::new(server.url.clone()),
        )],
        runtime,
    );
    let wallets = balance_addresses();
    assert_eq!(
        reader.wallet_balances(&wallets).await.unwrap(),
        balance_expected(&wallets)
    );
    assert_eq!(count(&server), 3);
    let pages = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request["params"][0].as_array().unwrap().len())
        .collect::<Vec<_>>();
    assert_eq!(pages, vec![100, 100, 1]);
    server.finish().await;
}
async fn native_partial() {
    let partial = LocalServer::new(Reply::FailAfter(1)).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&partial, &backup]);
    let wallets = balance_addresses();
    assert_eq!(
        runtime
            .native_balances(&[partial.url.clone(), backup.url.clone()], &wallets)
            .await
            .unwrap(),
        balance_expected(&wallets)
    );
    assert_eq!((count(&partial), count(&backup)), (2, 3));
    assert_eq!(
        partial.requests.lock().unwrap()[0]["params"][0],
        backup.requests.lock().unwrap()[0]["params"][0]
    );
    partial.finish().await;
    backup.finish().await;
}
async fn native_rate() {
    let limited = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&limited, &backup]);
    let wallets = balance_addresses();
    runtime
        .native_balances(&[limited.url.clone(), backup.url.clone()], &wallets[..2])
        .await
        .unwrap();
    let cooldowns = runtime.budget.lock().unwrap().cooldowns.clone();
    runtime.latest_blockhash().await.unwrap();
    assert!(!runtime.readiness_ping(&[limited.url.clone()]).await);
    assert!(runtime
        .signature_status(&Url::parse(&limited.url).unwrap(), 456, &signature())
        .await
        .is_err());
    assert!(runtime
        .native_balances(&[limited.url.clone()], &wallets[..2])
        .await
        .is_err());
    assert_eq!((count(&limited), count(&backup)), (1, 2));
    assert_eq!(runtime.budget.lock().unwrap().cooldowns, cooldowns);
    limited.finish().await;
    backup.finish().await;
}
async fn native_bad() {
    let target = LocalServer::new(Reply::Success).await;
    for reply in [
        Reply::WrongAccountCount,
        Reply::WrongId,
        Reply::InvalidJson,
        Reply::RpcError,
        Reply::ServerError,
        Reply::Disconnect,
        Reply::DeclaredOversize,
        Reply::Chunked(65_537),
        Reply::Redirect(target.url.clone()),
    ] {
        let bad = LocalServer::new(reply).await;
        let backup = LocalServer::new(Reply::Success).await;
        let runtime = runtime(&[&bad, &backup]);
        let wallets = balance_addresses();
        let error = runtime
            .native_balances(&[bad.url.clone()], &wallets[..2])
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("fixture-secret") && !error.contains("http"));
        assert_eq!((count(&bad), count(&backup)), (1, 0));
        assert_eq!(
            runtime
                .native_balances(&[bad.url.clone(), backup.url.clone()], &wallets[..2])
                .await
                .unwrap(),
            balance_expected(&wallets[..2])
        );
        assert_eq!((count(&bad), count(&backup)), (2, 1));
        bad.finish().await;
        backup.finish().await;
    }
    assert_eq!(count(&target), 0);
    assert_eq!(
        target.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    target.finish().await;
}
async fn native_priority() {
    let slow = LocalServer::new(Reply::SlowHeaders).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&slow, &backup]);
    let task = {
        let runtime = runtime.clone();
        let url = slow.url.clone();
        tokio::spawn(async move {
            runtime
                .native_balances(&[url], &balance_addresses()[..2])
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), slow.received.notified())
        .await
        .unwrap();
    assert!(tokio::time::timeout(
        Duration::from_millis(200),
        runtime.native_balances(&[slow.url.clone()], &balance_addresses()[..2])
    )
    .await
    .unwrap()
    .is_err());
    let next_start = runtime.budget.lock().unwrap().next_start[3];
    runtime.latest_blockhash().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.budget.lock().unwrap().next_start[3], next_start);
    assert_eq!(runtime.slots[3].available_permits(), 1);
    assert_eq!((count(&slow), count(&backup)), (2, 1));
    slow.finish().await;
    backup.finish().await;
}

fn quote_wallet() -> crate::domain::trade::Address {
    crate::domain::trade::Address::from_bytes([1; 32])
}
fn quote_mint() -> crate::domain::trade::Address {
    crate::domain::trade::Address::from_bytes([9; 32])
}
fn quote_reader(
    runtime: Arc<SharedReadRuntime>,
    servers: &[&LocalServer],
) -> crate::adapters::rpc::wallet_reads::RpcWalletReader {
    crate::adapters::rpc::wallet_reads::RpcWalletReader::new(
        quote_wallet(),
        servers
            .iter()
            .map(|server| {
                Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new(
                    server.url.clone(),
                ))
            })
            .collect(),
        runtime,
    )
}
async fn quote_success() {
    use crate::application::ports::wallet_reads::WalletReadPort;
    let server = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&server]);
    let rpc = Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new(
        server.url.clone(),
    ));
    let weak = Arc::downgrade(&rpc);
    let reader = crate::adapters::rpc::wallet_reads::RpcWalletReader::new(
        quote_wallet(),
        vec![rpc],
        runtime,
    );
    assert!(
        weak.upgrade().is_none(),
        "reader must retain URLs, not SDK RPC authority"
    );
    assert_eq!(
        reader.quote_balance(quote_mint()).await.unwrap(),
        crate::domain::wallet::QuoteBalance {
            decimals: 6,
            balance_units: 9007199254741000,
            balance_lamports: 123456789
        }
    );
    assert_eq!(count(&server), 3);
    assert_eq!(
        (method(&server, 0), method(&server, 1), method(&server, 2)),
        (
            "getTokenSupply".into(),
            "getTokenAccountsByOwner".into(),
            "getBalance".into()
        )
    );
    server.finish().await;
}
async fn quote_fallbacks() {
    use crate::application::ports::wallet_reads::WalletReadPort;
    for (reply, units, requests) in [
        (Reply::QuoteScan(Box::new(Reply::QuoteEmpty)), 0, 3),
        (Reply::QuoteScan(Box::new(Reply::QuoteUnparsed)), 23, 4),
        (Reply::QuoteScan(Box::new(Reply::RpcError)), 23, 4),
        (Reply::QuoteAtaFirstError, 23, 5),
    ] {
        let server = LocalServer::new(reply).await;
        let reader = quote_reader(runtime(&[&server]), &[&server]);
        assert_eq!(
            reader
                .quote_balance(quote_mint())
                .await
                .unwrap()
                .balance_units,
            units
        );
        assert_eq!(count(&server), requests);
        assert_eq!(method(&server, requests - 1), "getBalance");
        if requests >= 4 {
            assert_eq!(method(&server, 2), "getTokenAccountBalance");
        }
        if requests == 5 {
            assert_eq!(method(&server, 3), "getTokenAccountBalance");
            let requests = server.requests.lock().unwrap();
            assert_ne!(requests[2]["params"][0], requests[3]["params"][0]);
        }
        server.finish().await;
    }
}
async fn quote_invalid_limits() {
    use crate::application::ports::wallet_reads::WalletReadPort;
    for (reply, success) in [
        (Reply::AtLimit, true),
        (Reply::Chunked(524_288), true),
        (Reply::DeclaredOversize, false),
        (Reply::Chunked(524_289), false),
        (Reply::InvalidJson, false),
        (Reply::WrongId, false),
        (Reply::QuoteWrongMint, false),
        (Reply::QuoteMixed, false),
    ] {
        let server = LocalServer::new(Reply::QuoteScan(Box::new(reply))).await;
        let reader = quote_reader(runtime(&[&server]), &[&server]);
        let result = reader.quote_balance(quote_mint()).await;
        assert_eq!(result.is_ok(), success);
        if let Err(error) = result {
            assert!(!error.to_string().contains("fixture-secret"));
        }
        assert_eq!(
            count(&server),
            if success { 3 } else { 2 },
            "invalid/oversized scans cannot be hidden by ATA fallback"
        );
        server.finish().await;
    }
}
async fn quote_timeouts_redirect() {
    use crate::application::ports::wallet_reads::WalletReadPort;
    let target = LocalServer::new(Reply::Success).await;
    for reply in [
        Reply::SlowHeaders,
        Reply::SlowBody,
        Reply::Redirect(target.url.clone()),
    ] {
        let server = LocalServer::new(Reply::QuoteScan(Box::new(reply))).await;
        let reader = quote_reader(runtime(&[&server]), &[&server]);
        assert_eq!(
            reader
                .quote_balance(quote_mint())
                .await
                .unwrap()
                .balance_units,
            23
        );
        assert_eq!(count(&server), 4);
        assert_eq!(method(&server, 2), "getTokenAccountBalance");
        server.finish().await;
    }
    assert_eq!(
        target.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    target.finish().await;
}
async fn quote_rate_scope() {
    use crate::application::ports::wallet_reads::WalletReadPort;
    let limited =
        LocalServer::new(Reply::QuoteScan(Box::new(Reply::RateLimited(Some("60"))))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let runtime = runtime(&[&limited, &backup]);
    let reader = quote_reader(runtime.clone(), &[&limited, &backup]);
    assert!(reader.quote_balance(quote_mint()).await.is_err());
    assert_eq!((count(&limited), count(&backup)), (2, 0));
    let cooldowns = runtime.budget.lock().unwrap().cooldowns.clone();
    assert!(reader.quote_balance(quote_mint()).await.is_err());
    assert!(!runtime.readiness_ping(&[limited.url.clone()]).await);
    assert!(runtime
        .native_balances(&[limited.url.clone()], &[quote_wallet()])
        .await
        .is_err());
    assert!(runtime
        .signature_status(&Url::parse(&limited.url).unwrap(), 123, &signature())
        .await
        .is_err());
    runtime.latest_blockhash().await.unwrap();
    assert_eq!((count(&limited), count(&backup)), (2, 1));
    assert_eq!(runtime.budget.lock().unwrap().cooldowns, cooldowns);
    limited.finish().await;
    backup.finish().await;
}
async fn quote_priority() {
    let server = LocalServer::new(Reply::QuoteScan(Box::new(Reply::SlowHeaders))).await;
    let runtime = runtime(&[&server]);
    let task = {
        let runtime = runtime.clone();
        let url = server.url.clone();
        tokio::spawn(async move {
            runtime
                .quote_balance(&url, quote_wallet(), quote_mint())
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while count(&server) < 2 {
            server.received.notified().await;
        }
    })
    .await
    .unwrap();
    let next = runtime.budget.lock().unwrap().next_start[4];
    assert!(tokio::time::timeout(
        Duration::from_millis(200),
        runtime.quote_balance(&server.url, quote_wallet(), quote_mint())
    )
    .await
    .unwrap()
    .is_err());
    runtime.latest_blockhash().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(runtime.slots[4].available_permits(), 1);
    assert_eq!(runtime.budget.lock().unwrap().next_start[4], next);
    assert_eq!(count(&server), 3);
    server.finish().await;
}

#[tokio::test]
async fn shared_read_wire_child() {
    let Ok(case) = std::env::var(FLAG) else {
        return;
    };
    assert_eq!(std::env::var("NO_PROXY").unwrap(), "*");
    tokio::time::timeout(Duration::from_secs(15), async {
        match case.as_str() {
            "hash_rate" => hash_rate().await,
            "status_rate" => status_rate().await,
            "priority" => priority().await,
            "bad" => bad_replies().await,
            "cache" => cache_preserved().await,
            "body" => body_limits().await,
            "cooled" => all_cooled().await,
            "readiness_rate" => readiness_rate_scope().await,
            "readiness_hash_cooldown" => readiness_hash_cooldown().await,
            "readiness_priority" => readiness_busy_priority_cancel().await,
            "readiness_bad" => readiness_bad_replies().await,
            "native_pages" => native_pages().await,
            "native_partial" => native_partial().await,
            "native_rate" => native_rate().await,
            "native_bad" => native_bad().await,
            "native_priority" => native_priority().await,
            "quote_success" => quote_success().await,
            "quote_fallbacks" => quote_fallbacks().await,
            "quote_invalid_limits" => quote_invalid_limits().await,
            "quote_timeouts_redirect" => quote_timeouts_redirect().await,
            "quote_rate_scope" => quote_rate_scope().await,
            "quote_priority" => quote_priority().await,
            "preparation_success" => preparation_wire::success().await,
            "preparation_fallback" => preparation_wire::fallback().await,
            "preparation_limits" => preparation_wire::limits().await,
            "preparation_timeouts" => preparation_wire::timeouts_redirect().await,
            "preparation_rate" => preparation_wire::rate_scope().await,
            "preparation_priority" => preparation_wire::priority_cancel().await,
            _ => panic!("unknown shared read wire fixture"),
        }
    })
    .await
    .unwrap();
    println!("{DONE}");
}
fn run(case: &str) {
    let child = IsolatedTestProcess::spawn(CHILD, FLAG, case);
    child.await_line(DONE);
    child.finish();
}
#[test]
fn shared_read_wire_hash_429_is_shared_with_status() {
    run("hash_rate");
}
#[test]
fn shared_read_wire_status_429_is_shared_with_hash() {
    run("status_rate");
}
#[test]
fn shared_read_wire_pending_status_does_not_starve_hash_and_timeout_uses_backup() {
    run("priority");
}
#[test]
fn shared_read_wire_bad_replies_use_serial_backup_without_retries() {
    run("bad");
}
#[test]
fn shared_read_wire_bad_hash_never_refreshes_old_cache_timestamp_or_empty_cache() {
    run("cache");
}
#[test]
fn shared_read_wire_hash_body_limits_are_bounded() {
    run("body");
}
#[test]
fn shared_read_wire_all_cooled_does_not_issue_more_requests_or_extend_deadlines() {
    run("cooled");
}

#[test]
fn readiness_shared_read_wire_429_respects_execution_subset_and_other_profiles() {
    run("readiness_rate");
}
#[test]
fn readiness_shared_read_wire_hash_cooldown_is_visible_without_extending_it() {
    run("readiness_hash_cooldown");
}
#[test]
fn readiness_shared_read_wire_busy_and_cancel_cannot_starve_critical_refresh() {
    run("readiness_priority");
}
#[test]
fn readiness_shared_read_wire_bad_replies_fail_closed_or_use_only_allowed_backup() {
    run("readiness_bad");
}

#[test]
fn native_balance_shared_read_wire_pages_use_production_reader_without_sdk_native_calls() {
    run("native_pages");
}
#[test]
fn native_balance_shared_read_wire_partial_failure_never_mixes_nodes() {
    run("native_partial");
}
#[test]
fn native_balance_shared_read_wire_rate_limit_is_shared_across_profiles() {
    run("native_rate");
}
#[test]
fn native_balance_shared_read_wire_bad_count_body_identity_and_redirect_fail_closed() {
    run("native_bad");
}
#[test]
fn native_balance_shared_read_wire_busy_cancel_does_not_starve_critical_refresh() {
    run("native_priority");
}

#[test]
fn quote_balance_shared_read_wire_production_reader_uses_urls_only_and_exact_three_requests() {
    run("quote_success");
}
#[test]
fn quote_balance_shared_read_wire_empty_unparsed_rpc_errors_and_both_ata_paths() {
    run("quote_fallbacks");
}
#[test]
fn quote_balance_shared_read_wire_invalid_or_oversized_scans_cannot_hide_in_fallback() {
    run("quote_invalid_limits");
}
#[test]
fn quote_balance_shared_read_wire_timeouts_have_no_hidden_retry_and_redirect_target_is_unused() {
    run("quote_timeouts_redirect");
}
#[test]
fn quote_balance_shared_read_wire_429_never_widens_first_node_and_is_shared() {
    run("quote_rate_scope");
}
#[test]
fn quote_balance_shared_read_wire_busy_cancel_preserves_critical_capacity() {
    run("quote_priority");
}

#[test]
fn preparation_shared_read_wire_real_port_exact_two_requests_and_body_boundaries() {
    run("preparation_success");
}
#[test]
fn preparation_shared_read_wire_fallback_is_raw_only_and_keeps_minimum_context() {
    run("preparation_fallback");
}
#[test]
fn preparation_shared_read_wire_bad_identity_and_streaming_limits_have_no_hidden_fallback() {
    run("preparation_limits");
}
#[test]
fn preparation_shared_read_wire_headers_body_timeout_and_redirect_have_no_hidden_retry() {
    run("preparation_timeouts");
}
#[test]
fn preparation_shared_read_wire_429_is_shared_without_widening_current_lane() {
    run("preparation_rate");
}
#[test]
fn preparation_shared_read_wire_busy_cancel_releases_capacity_without_blocking_hash() {
    run("preparation_priority");
}
