use super::*;
use crate::adapters::test_support::{
    read_rpc_server::{LocalServer, Reply},
    IsolatedTestProcess,
};

const FLAG: &str = "SOL_BOT_OBSERVATION_HTTP_CASE";
const CHILD: &str =
    "adapters::rpc::ordinary_observation::tests::transport::ordinary_transport_child";
const DONE: &str = "ORDINARY_TRANSPORT_CASE_DONE";

fn reader(urls: &[String], interval: u64) -> SingleShotStatusReader {
    SingleShotStatusReader::new(urls, Duration::from_secs(interval)).unwrap()
}
fn assert_safe(error: &str) {
    assert!(!error.contains("fixture-secret"), "{error}");
    assert!(!error.contains("http://"), "{error}");
}

async fn original_request() {
    let server = LocalServer::new(Reply::Success).await;
    let reader = reader(&[server.url.clone()], 1);
    for id in 1..=2 {
        if id == 2 {
            reader.gate.state.lock().unwrap().next_allowed = Instant::now();
        }
        let found = reader
            .lookup_foreground(&signature())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.signature, signature());
        assert_eq!(
            found.commitment,
            crate::domain::reconciliation::ChainCommitment::Finalized
        );
    }
    server.assert_requests(&[1, 2]);
    server.finish().await;
}

async fn rate_limit() {
    let limited = LocalServer::new(Reply::RateLimited(Some("60"))).await;
    let backup = LocalServer::new(Reply::Success).await;
    let reader = reader(&[limited.url.clone(), backup.url.clone()], 1);
    let error = reader
        .lookup_foreground(&signature())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("429"));
    assert_safe(&error);
    limited.assert_requests(&[1]);
    backup.assert_requests(&[]);
    for _ in 0..2 {
        reader.gate.state.lock().unwrap().next_allowed = Instant::now();
        assert!(reader
            .lookup_foreground(&signature())
            .await
            .unwrap()
            .is_some());
    }
    limited.assert_requests(&[1]);
    backup.assert_requests(&[2, 3]);
    limited.finish().await;
    backup.finish().await;
}

async fn retry_after() {
    for (header, seconds) in [
        (Some("0"), 1),
        (Some("999999999"), 300),
        (Some("12"), 12),
        (Some("Wed, 21 Oct 2015 07:28:00 GMT"), 30),
        (Some("fixture-secret"), 30),
        (None, 30),
    ] {
        let server = LocalServer::new(Reply::RateLimited(header)).await;
        let reader = reader(&[server.url.clone()], 1);
        let before = Instant::now();
        let error = reader
            .lookup_foreground(&signature())
            .await
            .err()
            .unwrap()
            .to_string();
        assert_safe(&error);
        assert!(error.contains("429"));
        let after = Instant::now();
        let deadline = reader.gate.state.lock().unwrap().cooldowns[0].unwrap();
        assert!(deadline >= before + Duration::from_secs(seconds));
        assert!(deadline <= after + Duration::from_secs(seconds));
        reader.gate.state.lock().unwrap().next_allowed = Instant::now();
        assert!(reader
            .lookup_foreground(&signature())
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("cooling down"));
        server.assert_requests(&[1]);
        server.finish().await;
    }
}

async fn redirects() {
    let target = LocalServer::new(Reply::Success).await;
    let source = LocalServer::new(Reply::Redirect(target.url.clone())).await;
    let reader = reader(&[source.url.clone()], 1);
    let error = reader
        .lookup_foreground(&signature())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("HTTP error 302"), "{error}");
    assert_safe(&error);
    source.assert_requests(&[1]);
    target.assert_requests(&[]);
    source.finish().await;
    target.finish().await;
}

async fn body_limits() {
    for (reply, accepted) in [
        (Reply::AtLimit, true),
        (Reply::Chunked(MAX_BODY), true),
        (Reply::DeclaredOversize, false),
        (Reply::Chunked(MAX_BODY + 1), false),
    ] {
        let server = LocalServer::new(reply).await;
        let reader = reader(&[server.url.clone()], 1);
        let result = reader.lookup_foreground(&signature()).await;
        if accepted {
            assert!(result.unwrap().is_some());
        } else {
            let error = result.err().unwrap().to_string();
            assert!(error.contains("body exceeds limit"), "{error}");
            assert_safe(&error);
        }
        server.assert_requests(&[1]);
        server.finish().await;
    }
}

async fn timeouts() {
    for reply in [Reply::SlowHeaders, Reply::SlowBody] {
        let server = LocalServer::new(reply).await;
        let reader = reader(&[server.url.clone()], 60);
        let started = Instant::now();
        let error = reader
            .lookup_foreground(&signature())
            .await
            .err()
            .unwrap()
            .to_string();
        assert_safe(&error);
        assert!(started.elapsed() >= Duration::from_secs(1));
        assert!(started.elapsed() < Duration::from_secs(5));
        let state = reader.gate.state.lock().unwrap();
        assert!(!state.in_flight);
        assert!(state.next_allowed > Instant::now());
        drop(state);
        assert!(reader
            .lookup_foreground(&signature())
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("quota busy"));
        server.assert_requests(&[1]);
        server.finish().await;
    }
}

async fn cancellation() {
    let server = LocalServer::new(Reply::SlowHeaders).await;
    let reader = Arc::new(reader(&[server.url.clone()], 60));
    let task = {
        let reader = reader.clone();
        tokio::spawn(async move { reader.lookup_foreground(&signature()).await })
    };
    tokio::time::timeout(Duration::from_secs(3), server.received.notified())
        .await
        .unwrap();
    let reserved = reader.gate.state.lock().unwrap().next_allowed;
    assert!(reader
        .lookup_foreground(&signature())
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("quota busy"));
    server.assert_requests(&[1]);
    assert_eq!(reader.gate.state.lock().unwrap().next_allowed, reserved);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(!reader.gate.state.lock().unwrap().in_flight);
    assert_eq!(reader.gate.state.lock().unwrap().next_allowed, reserved);
    assert_eq!(reader.gate.capacity.available_permits(), 1);
    assert!(reader
        .lookup_foreground(&signature())
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("quota busy"));
    server.assert_requests(&[1]);
    server.finish().await;
}

async fn malformed() {
    for (reply, expected) in [
        (Reply::WrongId, "identity mismatch"),
        (Reply::InvalidJson, "invalid observation JSON-RPC response"),
        (Reply::RpcError, "JSON-RPC error -32005"),
        (Reply::ServerError, "HTTP error 503"),
        (Reply::Disconnect, "transport unavailable"),
    ] {
        let server = LocalServer::new(reply).await;
        let reader = reader(&[server.url.clone()], 1);
        let error = reader
            .lookup_foreground(&signature())
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(expected), "{error}");
        assert_safe(&error);
        server.assert_requests(&[1]);
        server.finish().await;
    }
}

#[tokio::test]
async fn ordinary_transport_child() {
    let Ok(case) = std::env::var(FLAG) else {
        return;
    };
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        assert!(std::env::var_os(key).is_none());
    }
    assert_eq!(std::env::var("NO_PROXY").unwrap(), "*");
    tokio::time::timeout(Duration::from_secs(15), async {
        match case.as_str() {
            "request" => original_request().await,
            "rate_limit" => rate_limit().await,
            "retry_after" => retry_after().await,
            "redirects" => redirects().await,
            "body_limits" => body_limits().await,
            "timeouts" => timeouts().await,
            "cancellation" => cancellation().await,
            "malformed" => malformed().await,
            _ => panic!("unknown isolated transport test case"),
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
fn ordinary_transport_original_signature_and_monotonic_request_ids() {
    run("request");
}
#[test]
fn ordinary_transport_429_is_one_attempt_then_next_query_uses_healthy_lane() {
    run("rate_limit");
}
#[test]
fn ordinary_transport_retry_after_headers_are_parsed_and_bounded() {
    run("retry_after");
}
#[test]
fn ordinary_transport_redirect_does_not_contact_its_target() {
    run("redirects");
}
#[test]
fn ordinary_transport_length_and_chunked_body_limits_match_exact_boundary() {
    run("body_limits");
}
#[test]
fn ordinary_transport_header_and_body_timeouts_do_not_retry_or_refund_budget() {
    run("timeouts");
}
#[test]
fn ordinary_transport_cancel_releases_capacity_without_refunding_pacing() {
    run("cancellation");
}
#[test]
fn ordinary_transport_bad_responses_and_disconnect_do_not_retry_or_echo_secrets() {
    run("malformed");
}
