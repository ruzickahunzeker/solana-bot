use super::*;

#[tokio::test]
async fn sdk_application_submission_preserves_validated_finalized_outcome_ahead_of_late_receipt() {
    for response in [3, 4] {
        let h = setup().await;
        h.backend
            .0
            .control
            .response
            .store(response, Ordering::SeqCst);
        let r = h.run().await;
        let record = r.outcome.record.unwrap();
        assert_eq!(
            record.status,
            if response == 3 {
                TradeStatus::Confirmed
            } else {
                TradeStatus::Failed
            }
        );
        assert_eq!(record.signature, r.outcome.signature);
        assert_eq!(
            record.error.as_deref(),
            (response == 4).then_some("synthetic finalized failure")
        );
        assert_eq!(r.control.signs.load(Ordering::SeqCst), 1);
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 1);
        assert_eq!(stage(&r.journal, &r.execution).await, "broadcasting");
        assert_eq!(
            r.journal.store.claim(&r.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}

#[tokio::test]
async fn sdk_application_submission_failed_parent_without_readable_finality_stays_unknown() {
    let h = setup().await;
    h.backend.0.control.response.store(4, Ordering::SeqCst);
    *h.backend.0.control.fail.lock().unwrap() = Some("finalized-failure");
    let r = h.run().await;
    assert!(r.outcome.record.is_none());
    assert!(r.outcome.signature.is_some());
    assert_eq!(r.control.signs.load(Ordering::SeqCst), 1);
    assert_eq!(r.control.sends.load(Ordering::SeqCst), 1);
    let parent = sqlx::query("SELECT status,signature FROM trades WHERE request_id=?")
        .bind(r.execution.reservation().request_id())
        .fetch_one(&r.journal.pool)
        .await
        .unwrap();
    assert_eq!(parent.get::<&str, _>("status"), "failed");
    assert_eq!(
        parent.get::<Option<String>, _>("signature"),
        r.outcome.signature
    );
}
