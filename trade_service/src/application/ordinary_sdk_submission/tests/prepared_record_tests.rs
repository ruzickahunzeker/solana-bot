use super::*;
use crate::adapters::sdk::damm_probe::test_support::buy_record_fixture;

#[tokio::test]
async fn sdk_application_submission_buy_record_cannot_enter_sell_journal_or_acquire_a_signer() {
    let h = setup().await;
    let (buy, message) = buy_record_fixture().await;
    let metadata = SdkPreparedRecord::DammBuy(buy);
    metadata.validate(&message).unwrap();
    let control = h.backend.0.control.clone();
    let result = execute(
        &h.journal,
        &Second(h.backend.0),
        &h.execution,
        h.owner,
        PlainCandidate { metadata, message },
    )
    .await;
    assert!(result.error.unwrap().contains("not DAMM sell"));
    assert!(result.signature.is_none());
    assert_eq!(control.signs.load(Ordering::SeqCst), 0);
    assert_eq!(control.keys.load(Ordering::SeqCst), 0);
    assert_eq!(control.sends.load(Ordering::SeqCst), 0);
    let events = control.events.lock().unwrap().clone();
    assert!(!events
        .iter()
        .any(|e| e == "prepare" || e == "begin-signing" || e == "acquire-signer"));
    assert_eq!(stage(&h.journal, &h.execution).await, "failed");
    let json: Option<String> = sqlx::query_scalar(
        "SELECT sdk_prepared_json FROM ordinary_execution_claims WHERE request_id=?",
    )
    .bind(h.execution.reservation().request_id())
    .fetch_one(&h.journal.pool)
    .await
    .unwrap();
    assert!(json.is_none());
}

#[tokio::test]
async fn sdk_controlled_signing_nominal_buy_record_cannot_create_or_replace_original_sell_permit() {
    let h = setup().await;
    let message = h.candidate.message_bytes();
    let permit =
        SdkSigningPermit::new(&h.execution, &h.owner, h.candidate.metadata(), &message).unwrap();
    let (buy, buy_message) = buy_record_fixture().await;
    let buy = SdkPreparedRecord::DammBuy(buy);
    assert!(SdkSigningPermit::new(&h.execution, &h.owner, &buy, &buy_message).is_err());
    assert!(permit.validate(&buy, &buy_message).is_err());
    permit.validate(h.candidate.metadata(), &message).unwrap();
    assert_eq!(h.backend.0.control.keys.load(Ordering::SeqCst), 0);
    assert_eq!(stage(&h.journal, &h.execution).await, "claimed");
}
