use super::*;

pub(super) async fn good() {
    let mut c = Case::new(Mode::Good).await;
    let outcome = c.run().await;
    let record = outcome.record.unwrap();
    assert_eq!(record.status, TradeStatus::Submitted);
    assert!(outcome.error.is_none());
    assert_eq!(record.signature, outcome.signature);
    let methods = c
        .trace
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|v| v.0)
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        [
            RpcRequest::GetMultipleAccounts,
            RpcRequest::SimulateTransaction,
            RpcRequest::SimulateTransaction,
            RpcRequest::SendTransaction
        ]
    );
    assert_eq!(c.server.requests.lock().unwrap().len(), 4);
    let sends = c.trace.sent.lock().unwrap().clone();
    assert_eq!(sends.len(), 1);
    let tx: VersionedTransaction = bincode::deserialize(&sends[0]).unwrap();
    let simulations = c.trace.simulations.lock().unwrap().clone();
    assert_eq!(simulations.len(), 2);
    assert_eq!(simulations[1].message, tx.message);
    let row=sqlx::query("SELECT sdk_prepared_json,unsigned_message FROM ordinary_execution_claims WHERE request_id=?")
        .bind(&c.f.record.request_id).fetch_one(&c.pool).await.unwrap();
    let metadata = SdkPreparedRecord::decode(
        row.get("sdk_prepared_json"),
        &row.get::<Vec<u8>, _>("unsigned_message"),
    )
    .unwrap();
    let details = metadata.sell().unwrap().details();
    assert_eq!(details.balance_units, 1000);
    assert_eq!(details.balance_slot, 123);
    assert_eq!(details.input_units, 100);
    assert_eq!(details.minimum_output_units, 102);
    c.unchanged_request().await;
    c.finish().await;
}

pub(super) async fn pre_sign() {
    for mode in [Mode::BadRaw, Mode::ZeroOutput, Mode::BadPreview] {
        let mut c = Case::new(mode).await;
        let outcome = c.run().await;
        assert_eq!(outcome.record.unwrap().status, TradeStatus::Failed);
        assert!(outcome.signature.is_none());
        assert!(outcome.error.is_some());
        assert!(c.trace.sent.lock().unwrap().is_empty());
        assert_eq!(
            c.trace.simulations.lock().unwrap().len(),
            match mode {
                Mode::BadRaw => 0,
                Mode::ZeroOutput => 1,
                _ => 2,
            }
        );
        if mode == Mode::BadRaw {
            assert!(c.trace.calls.lock().unwrap().is_empty());
        }
        let signature: Option<String> = sqlx::query_scalar(
            "SELECT signed_signature FROM ordinary_execution_claims WHERE request_id=?",
        )
        .bind(&c.f.record.request_id)
        .fetch_one(&c.pool)
        .await
        .unwrap();
        assert!(signature.is_none());
        c.unchanged_request().await;
        c.finish().await;
    }
}

pub(super) async fn post_sign() {
    for mode in [Mode::WrongReceipt, Mode::ReceiptWriteFailure] {
        let mut c = Case::new(mode).await;
        let outcome = c.run().await;
        assert!(outcome.signature.is_some());
        assert!(outcome.error.is_some());
        if mode == Mode::WrongReceipt {
            assert_eq!(outcome.record.unwrap().status, TradeStatus::Unknown);
        } else {
            assert!(outcome.record.is_none());
        }
        let parent =
            c.f.state
                .persistence
                .get(&c.f.record.request_id)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(parent.signature, outcome.signature);
        assert_eq!(
            parent.status,
            if mode == Mode::WrongReceipt {
                TradeStatus::Unknown
            } else {
                TradeStatus::Submitting
            }
        );
        assert_eq!(c.trace.sent.lock().unwrap().len(), 1);
        c.unchanged_request().await;
        c.finish().await;
    }
}

pub(super) async fn cancel() {
    for mode in [Mode::CancelProbe, Mode::CancelSend] {
        let mut c = Case::new(mode).await;
        let trace = c.trace.clone();
        let pool = c.pool.clone();
        let execution = c.f.execution.clone();
        let persistence = c.f.state.persistence.clone();
        let task = tokio::spawn(async move { c.run().await });
        tokio::time::timeout(Duration::from_secs(3), trace.entered.notified())
            .await
            .unwrap();
        task.abort();
        assert!(matches!(task.await,Err(error) if error.is_cancelled()));
        assert!(trace.dropped.load(Ordering::SeqCst));
        assert_eq!(
            trace.sent.lock().unwrap().len(),
            usize::from(mode == Mode::CancelSend)
        );
        let row=sqlx::query("SELECT submission_stage,signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
            .bind(execution.reservation().request_id()).fetch_one(&pool).await.unwrap();
        assert_eq!(
            row.get::<&str, _>("submission_stage"),
            if mode == Mode::CancelSend {
                "broadcasting"
            } else {
                "claimed"
            }
        );
        assert_eq!(
            row.get::<Option<String>, _>("signed_signature").is_some(),
            mode == Mode::CancelSend
        );
        let before = trace.calls.lock().unwrap().len();
        assert_eq!(
            persistence
                .ordinary_execution_claims()
                .unwrap()
                .claim(&execution)
                .await
                .unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        assert_eq!(trace.calls.lock().unwrap().len(), before);
        pool.close().await;
    }
}
