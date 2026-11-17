use super::*;

#[tokio::test]
async fn preparation_presence_preserves_v1_program_hint_and_exact_selected_lane_request() {
    let mut foreign = scan(false, 0);
    foreign[0]["pubkey"] = json!(a(99).to_string());
    foreign[0]["account"]["data"]["parsed"] = json!({"arbitrary":"not verified evidence"});
    let mut binary = scan(false, 1000);
    binary[0]["account"]["data"] = json!(["", "base64"]);
    let mut other = scan(false, 1000);
    other[0]["account"]["data"]["program"] = json!("other-program");
    for (value, expected) in [
        (json!([]), false),
        (scan(false, 0), true),
        (scan(true, u64::MAX), true),
        (foreign, true),
        (binary.clone(), false),
        (other, false),
        (json!([binary[0], scan(true, 0)[0]]), true),
    ] {
        let r = runtime(vec![Answer::Value(42, value)]);
        assert_eq!(
            r.preparation_account_exists(&urls()[0], a(1), a(9))
                .await
                .unwrap(),
            expected
        );
        let calls = r.transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, Url::parse(&urls()[0]).unwrap().to_string());
        assert_eq!(
            calls[0].1,
            ReadRequest::TokenAccounts {
                id: 1,
                wallet: a(1),
                mint: a(9)
            }
            .body()
        );
        assert_eq!(r.slots[5].available_permits(), 1);
    }
}

#[tokio::test]
async fn preparation_presence_rejects_invalid_metadata_and_bounded_scan_without_fallback() {
    let good = scan(false, 1000);
    let mut missing = good.clone();
    missing[0]["account"]
        .as_object_mut()
        .unwrap()
        .remove("owner");
    let mut invalid_data = good.clone();
    invalid_data[0]["account"]["data"] = json!(false);
    for answer in [
        Answer::Value(42, json!(null)),
        Answer::Value(42, json!({})),
        Answer::Value(42, missing),
        Answer::Value(42, invalid_data),
        Answer::Value(42, json!(vec![good[0].clone(); 1001])),
        Answer::Limit,
        Answer::Value(0, good),
    ] {
        let r = runtime(vec![answer]);
        let error = r
            .preparation_account_exists(&urls()[0], a(1), a(9))
            .await
            .err()
            .unwrap();
        assert!(fatal(&error));
        assert!(!format!("{error:?}").contains("fixture-secret"));
        assert_eq!(r.transport.calls.lock().unwrap().len(), 1);
        assert_eq!(r.slots[5].available_permits(), 1);
    }
}

#[tokio::test]
async fn preparation_presence_unknown_endpoint_and_transport_failure_are_not_known_absence() {
    let r = runtime(vec![]);
    let error = r
        .preparation_account_exists("https://unknown.invalid/?key=fixture-secret", a(1), a(9))
        .await
        .err()
        .unwrap();
    assert!(retryable(&error));
    assert!(!format!("{error:?}").contains("fixture-secret"));
    assert!(r.transport.calls.lock().unwrap().is_empty());
    let r = runtime(vec![Answer::Unavailable]);
    assert!(retryable(
        &r.preparation_account_exists(&urls()[0], a(1), a(9))
            .await
            .err()
            .unwrap()
    ));
    assert_eq!(r.transport.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn preparation_presence_429_shares_cooldown_with_balance_and_hash_without_lane_switch() {
    let r = runtime(vec![Answer::Rate]);
    assert!(retryable(
        &r.preparation_account_exists(&urls()[0], a(1), a(9))
            .await
            .err()
            .unwrap()
    ));
    assert!(r
        .preparation_account_exists(&urls()[0], a(1), a(9))
        .await
        .is_err());
    assert!(reader(&r).snapshot(a(9)).await.is_err());
    assert_eq!(r.transport.calls.lock().unwrap().len(), 1);
    r.latest_blockhash().await.unwrap();
    let calls = r.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, Url::parse(&urls()[1]).unwrap().to_string());
}

#[tokio::test]
async fn preparation_presence_and_balance_share_one_permit_and_cancel_releases_it() {
    for presence_first in [false, true] {
        let r = runtime(vec![Answer::Hold]);
        let child = r.clone();
        let task = tokio::spawn(async move {
            if presence_first {
                child
                    .preparation_account_exists(&urls()[0], a(1), a(9))
                    .await
                    .map(|_| ())
            } else {
                reader(&child).snapshot(a(9)).await.map(|_| ())
            }
        });
        tokio::time::timeout(Duration::from_secs(1), r.transport.entered.notified())
            .await
            .unwrap();
        let error = if presence_first {
            reader(&r).snapshot(a(9)).await.err().unwrap()
        } else {
            r.preparation_account_exists(&urls()[0], a(1), a(9))
                .await
                .err()
                .unwrap()
        };
        assert!(retryable(&error) && error.to_string().contains("busy"));
        r.latest_blockhash().await.unwrap();
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert_eq!(r.transport.dropped.load(Ordering::SeqCst), 1);
        assert_eq!(r.slots[5].available_permits(), 1);
        assert_eq!(r.transport.calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn preparation_presence_cancel_during_pacing_sends_nothing_and_refunds_no_start() {
    use std::{future::Future, task::Poll};
    let r = runtime(vec![]);
    r.budget.lock().unwrap().next_start[5] = Instant::now() + Duration::from_secs(1);
    let before = r.budget.lock().unwrap().next_start[5];
    let configured = urls();
    let mut pending = Box::pin(r.preparation_account_exists(&configured[0], a(1), a(9)));
    std::future::poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(pending);
    assert_eq!(r.budget.lock().unwrap().next_start[5], before);
    assert_eq!(r.slots[5].available_permits(), 1);
    assert!(r.transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn preparation_presence_deadline_includes_pacing_and_never_retries_a_timed_out_read() {
    let r = runtime(vec![]);
    r.budget.lock().unwrap().next_start[5] = Instant::now() + Duration::from_secs(3);
    assert!(retryable(
        &r.preparation_account_exists(&urls()[0], a(1), a(9))
            .await
            .err()
            .unwrap()
    ));
    assert!(r.transport.calls.lock().unwrap().is_empty());
    let r = runtime(vec![Answer::Hold]);
    let started = Instant::now();
    assert!(retryable(
        &r.preparation_account_exists(&urls()[0], a(1), a(9))
            .await
            .err()
            .unwrap()
    ));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(r.transport.calls.lock().unwrap().len(), 1);
    assert_eq!(r.transport.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(r.slots[5].available_permits(), 1);
}

#[tokio::test]
async fn preparation_presence_shares_start_pacing_with_following_balance_evidence() {
    let r = runtime(vec![
        Answer::Value(42, scan(false, 0)),
        Answer::Value(42, scan(false, 1000)),
        Answer::Value(42, raw_accounts(false, 1000)),
    ]);
    assert!(r
        .preparation_account_exists(&urls()[0], a(1), a(9))
        .await
        .unwrap());
    assert_eq!(
        reader(&r)
            .snapshot(a(9))
            .await
            .unwrap()
            .checked_units(a(1), a(9))
            .unwrap(),
        1000
    );
    let calls = r.transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    for pair in calls.windows(2) {
        assert!(pair[1].2.duration_since(pair[0].2) >= Duration::from_millis(240));
    }
}

#[tokio::test]
async fn preparation_presence_url_aliases_share_cooldown_but_credentials_remain_distinct() {
    let configured = vec![
        "https://ONE.invalid:443/?key=one".into(),
        "https://one.invalid/?key=two".into(),
    ];
    let r = SharedReadRuntime::with_transport(
        &configured,
        Fake::new(vec![Answer::Rate, Answer::Value(42, scan(false, 0))]),
    );
    assert!(r
        .preparation_account_exists(&configured[0], a(1), a(9))
        .await
        .is_err());
    assert!(r
        .preparation_account_exists("https://one.invalid/?key=one", a(1), a(9))
        .await
        .is_err());
    assert!(r
        .preparation_account_exists(&configured[1], a(1), a(9))
        .await
        .unwrap());
    assert_eq!(r.transport.calls.lock().unwrap().len(), 2);
}
