//! Quote port contract checks with a disposable store, no RPC or signer.
use super::{
    tests::{harness, harness_with_store, input},
    *,
};
use crate::{
    application::wsol_failure::{WsolFailure, WsolFailureKind, WsolFailureStage},
    conversion::ConversionStore,
};
use std::sync::Arc;

pub(crate) fn estimate() -> WsolQuoteEstimate {
    WsolQuoteEstimate {
        amount_lamports: 1_000_000,
        source_balance_lamports: 2_000_000,
        ata_exists: true,
        estimated_network_fee_lamports: 10_000,
        temporary_rent_lamports: 0,
        rent_refund_lamports: 0,
    }
}

#[tokio::test]
async fn wsol_quote_boundary_metadata_must_match_requested_wallet_before_reader_or_persistence() {
    let (service, shared) = harness().await;
    shared.mode(20);
    let error = service.quote(input()).await.err().unwrap();
    assert_eq!(
        error.downcast_ref::<WsolFailure>().unwrap().kind,
        WsolFailureKind::Integrity
    );
    assert_eq!(shared.events(), ["metadata"]);
}

#[tokio::test]
async fn wsol_quote_boundary_rejects_misbound_or_impossible_successful_estimates_before_saving() {
    for field in 0..6 {
        let (service, shared) = harness().await;
        let mut bad = estimate();
        match field {
            0 => bad.amount_lamports = 0,
            1 => bad.amount_lamports += 1,
            2 => bad.source_balance_lamports = 1,
            3 => bad.temporary_rent_lamports = 1,
            4 => bad.rent_refund_lamports = 1,
            _ => bad.estimated_network_fee_lamports = u64::MAX,
        }
        shared.replace_estimate(bad);
        assert!(service.quote(input()).await.is_err(), "field {field}");
        assert_eq!(
            shared.events(),
            ["metadata", "parse_owner", "quote_reader", "estimate:1"]
        );
    }
}

#[test]
fn wsol_quote_boundary_preserves_exact_minimum_budget_and_full_partial_unwrap_invariants() {
    let request = input();
    let mut good = estimate();
    good.source_balance_lamports =
        good.amount_lamports + good.estimated_network_fee_lamports + 100_000;
    good.validate_for(&request).unwrap();
    good.source_balance_lamports -= 1;
    assert!(good.validate_for(&request).is_err());
    for all in [false, true] {
        let mut request = input();
        request.direction = WsolDirection::Unwrap;
        request.all = all;
        request.amount_lamports = (!all).then_some(1_000_000);
        let mut good = estimate();
        if all {
            good.amount_lamports = good.source_balance_lamports;
            good.rent_refund_lamports = 2_039_280;
        } else {
            good.temporary_rent_lamports = 2_039_280;
        }
        good.validate_for(&request).unwrap();
        for field in 0..4 {
            let mut bad = good.clone();
            match field {
                0 => bad.ata_exists = false,
                1 => bad.amount_lamports -= 1,
                2 if all => bad.temporary_rent_lamports = 1,
                2 => bad.rent_refund_lamports = 1,
                _ => bad.estimated_network_fee_lamports = u64::MAX,
            }
            assert!(
                bad.validate_for(&request).is_err(),
                "all={all}, field {field}"
            );
        }
    }
}

#[tokio::test]
async fn wsol_quote_boundary_typed_phase_failures_never_guess_from_text_or_publish_submission_context(
) {
    for (mode, kind, stage, saved) in [
        (
            1,
            WsolFailureKind::Unavailable,
            WsolFailureStage::QuoteMetadata,
            false,
        ),
        (
            21,
            WsolFailureKind::Unavailable,
            WsolFailureStage::QuoteMetadata,
            false,
        ),
        (
            22,
            WsolFailureKind::Rejected,
            WsolFailureStage::QuoteMetadata,
            false,
        ),
        (
            4,
            WsolFailureKind::Integrity,
            WsolFailureStage::QuoteMetadata,
            false,
        ),
        (
            5,
            WsolFailureKind::Unavailable,
            WsolFailureStage::QuoteEstimate,
            false,
        ),
        (
            7,
            WsolFailureKind::Unavailable,
            WsolFailureStage::QuoteEstimate,
            false,
        ),
        (
            23,
            WsolFailureKind::Rejected,
            WsolFailureStage::QuoteEstimate,
            false,
        ),
        (
            12,
            WsolFailureKind::OutcomeUnknown,
            WsolFailureStage::QuoteSave,
            false,
        ),
        (
            24,
            WsolFailureKind::OutcomeUnknown,
            WsolFailureStage::QuoteSave,
            true,
        ),
        (
            25,
            WsolFailureKind::OutcomeUnknown,
            WsolFailureStage::QuoteSave,
            true,
        ),
    ] {
        let (service, shared) = harness().await;
        shared.mode(mode);
        let error = service.quote(input()).await.err().unwrap();
        let failure = error.downcast_ref::<WsolFailure>().unwrap();
        assert_eq!((failure.kind, failure.stage), (kind, stage));
        assert!(failure.submission_id.is_none() && failure.signature.is_none());
        assert_eq!(shared.saved_quote().is_some(), saved);
        if let Some(quote) = shared.saved_quote() {
            assert_eq!(
                shared.store.get_quote(&quote.quote_id).await.unwrap(),
                Some((&quote).into())
            );
        }
        assert!(shared.store.get_by_key("key").await.unwrap().is_none());
    }
}

#[tokio::test]
async fn wsol_quote_boundary_cancellation_before_or_after_quote_commit_never_reserves_or_replays() {
    for after_commit in [false, true] {
        let dir = std::env::temp_dir().join(format!("wsol-quote-cancel-{}", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", dir.join("test.sqlite").display());
        let (service, shared) =
            harness_with_store(Arc::new(ConversionStore::new(&url).await.unwrap())).await;
        let gate = if after_commit {
            shared.pause_quote_save()
        } else {
            shared.pause_estimate()
        };
        let task = tokio::spawn({
            let service = service.clone();
            async move { service.quote(input()).await }
        });
        gate.wait_until_entered().await;
        let saved = shared.saved_quote();
        assert_eq!(saved.is_some(), after_commit);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(shared.store.get_by_key("key").await.unwrap().is_none());
        drop(service);
        drop(shared);
        let store = ConversionStore::new(&url).await.unwrap();
        if let Some(quote) = saved {
            assert_eq!(
                store.get_quote(&quote.quote_id).await.unwrap(),
                Some((&quote).into())
            );
        }
        assert!(store.get_by_key("key").await.unwrap().is_none());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversion_quotes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, i64::from(after_commit));
        pool.close().await;
        drop(store);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
