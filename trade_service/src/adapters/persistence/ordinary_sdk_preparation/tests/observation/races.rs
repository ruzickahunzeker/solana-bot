use super::*;

#[tokio::test]
async fn sdk_recovery_finality_two_sqlite_pools_allow_only_one_conflicting_final_result() {
    let dir = std::env::temp_dir().join(format!("sdk-finality-race-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 2000,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let (c, key) = signing_case(Some(state)).await;
    let signed = save_signed(&c, &key).await;
    stage(&c, &signed, "broadcasting").await;
    let before = immutable(&c.store.pool, &c.f.record.request_id).await;
    let second = TradePersistence::new(&config).await.unwrap();
    let other = second.ordinary_execution_claims().unwrap();
    let a = c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    let b = other
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    let success = observation(&signed.signature, ChainCommitment::Finalized, None);
    let failure = observation(
        &signed.signature,
        ChainCommitment::Finalized,
        Some("synthetic finalized failure"),
    );
    let (left, right) = tokio::join!(
        c.store.record_captured_finality(&a.context, &success),
        other.record_captured_finality(&b.context, &failure),
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let expected = if left.is_ok() {
        TradeStatus::Confirmed
    } else {
        TradeStatus::Failed
    };
    assert_eq!(
        c.store.read_observation(&a.context).await.unwrap().status,
        expected
    );
    assert_eq!(
        other.read_observation(&b.context).await.unwrap().status,
        expected
    );
    assert_eq!(
        immutable(&c.store.pool, &c.f.record.request_id).await,
        before
    );
    assert_eq!(
        c.store.claim(&c.f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    c.store.pool.close().await;
    second.pool_for_tests().close().await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn sdk_recovery_finality_lost_write_ack_is_read_back_without_another_rpc_or_execution() {
    struct LostAck(SqliteOrdinaryExecutionClaims);
    impl RecoveredOrdinaryObservations for LostAck {
        async fn capture_observation(
            &self,
            id: &str,
        ) -> Result<crate::application::ordinary_observation_context::CapturedOrdinaryObservation>
        {
            self.0.capture_observation(id).await
        }
        async fn read_observation(
            &self,
            context: &crate::application::ordinary_observation_context::OrdinaryObservationContext,
        ) -> Result<TradeRecordV1> {
            self.0.read_observation(context).await
        }
        async fn record_captured_finality(
            &self,
            context: &crate::application::ordinary_observation_context::OrdinaryObservationContext,
            value: &ChainObservation,
        ) -> Result<TradeRecordV1> {
            self.0.record_captured_finality(context, value).await?;
            anyhow::bail!("synthetic lost finality acknowledgement")
        }
    }
    let (c, key) = signing_case(None).await;
    let signed = save_signed(&c, &key).await;
    let captured = c
        .store
        .capture_observation(&c.f.record.request_id)
        .await
        .unwrap();
    let reader = Reader::new(Some(observation(
        &signed.signature,
        ChainCommitment::Finalized,
        None,
    )));
    let journal = LostAck(SqliteOrdinaryExecutionClaims::new(c.store.pool.clone()));
    assert!(
        reconcile_captured(&journal, &reader, &captured.context, Duration::from_secs(1))
            .await
            .is_err()
    );
    let before = immutable(&c.store.pool, &c.f.record.request_id).await;
    let final_read =
        reconcile_captured(&journal, &reader, &captured.context, Duration::from_secs(1))
            .await
            .unwrap();
    assert_eq!(final_read.record.status, TradeStatus::Confirmed);
    assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        immutable(&c.store.pool, &c.f.record.request_id).await,
        before
    );
    assert_eq!(
        c.store.claim(&c.f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
}
