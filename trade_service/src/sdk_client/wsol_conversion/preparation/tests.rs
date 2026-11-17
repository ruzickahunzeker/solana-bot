use super::*;
use crate::{
    adapters::rpc::wsol_quote_reads::{
        tests::{key, runtime, urls, Fake, Fixture, Mode},
        WsolQuoteReadSession,
    },
    application::wsol_failure::WsolFailureKind,
};
use base64::Engine;
use serde_json::{json, Value};
use solana_sdk::{
    hash::Hash,
    signature::{Signature, Signer},
    transaction::Transaction,
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

pub(crate) fn quote(f: &Fixture, direction: Direction, amount: u64) -> WsolQuoteSnapshot {
    let mut q = WsolQuoteSnapshot::from(&crate::conversion::contract_tests::quote());
    q.wallet_pubkey = f.owner.to_string();
    q.direction = direction;
    q.amount_lamports = amount;
    q.source_balance_lamports = if direction == Direction::Wrap {
        f.sol
    } else {
        f.token.unwrap_or(0)
    };
    q.ata_exists = f.token.is_some();
    q.temporary_rent_lamports = if direction == Direction::Unwrap && amount < f.token.unwrap_or(0) {
        f.rent
    } else {
        0
    };
    q.rent_refund_lamports = if direction == Direction::Unwrap && f.token == Some(amount) {
        amount.saturating_add(f.rent).saturating_sub(amount)
    } else {
        0
    };
    q.estimated_network_fee_lamports = 5000;
    q.expires_at_ms = 4_000_000_000_000;
    q
}
async fn inspect(
    r: &Arc<SharedReadRuntime<Fake>>,
    f: &Fixture,
    q: &WsolQuoteSnapshot,
) -> Result<UnsignedWsolTransaction> {
    prepare(f.owner, r, &urls()[0], CommitmentConfig::confirmed(), q).await
}
fn normalized(mut message: Message) -> Message {
    if message.header.num_required_signatures == 2 {
        message.account_keys[1] = key(90);
    }
    message
}
fn normalize_request(method: &str, mut params: Value) -> Value {
    if method == "getAccountInfo" {
        params[1]["encoding"] = json!("base64");
    }
    if method == "getFeeForMessage" {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(params[0].as_str().unwrap())
            .unwrap();
        let message: Message = bincode::deserialize(&bytes).unwrap();
        params[0] = json!(
            base64::engine::general_purpose::STANDARD.encode(normalized(message).serialize())
        );
    }
    json!([method, params])
}
fn evidence(p: &UnsignedWsolTransaction) -> (WsolQuoteSnapshot, u64, u64, u64, u64, u64, Hash) {
    let a = &p.authorization;
    (
        a.quote.clone(),
        a.sol_balance_lamports,
        a.wsol_balance_lamports,
        a.account_rent_lamports,
        a.rent_refund_lamports,
        a.network_fee_lamports,
        a.blockhash,
    )
}
pub(crate) fn assert_prepared(p: &UnsignedWsolTransaction, f: &Fixture, q: &WsolQuoteSnapshot) {
    p.validate(&f.owner, q, 1).unwrap();
    assert_eq!(p.owner, f.owner);
    assert_eq!(p.authorization.quote, *q);
    assert_eq!(p.authorization.sol_balance_lamports, f.sol);
    assert_eq!(p.authorization.wsol_balance_lamports, f.token.unwrap_or(0));
    assert_eq!(p.authorization.account_rent_lamports, f.rent);
    assert_eq!(p.authorization.network_fee_lamports, f.fee.unwrap());
    assert_eq!(p.message.recent_blockhash, Hash::new_from_array([8; 32]));
    let transaction = Transaction::new_unsigned(p.message.clone());
    assert!(transaction
        .signatures
        .iter()
        .all(|s| *s == Signature::default()));
    let partial = q.direction == Direction::Unwrap && q.amount_lamports < f.token.unwrap_or(0);
    assert_eq!(p.temporary_signer.is_some(), partial);
    assert_eq!(
        p.message.header.num_required_signatures,
        if partial { 2 } else { 1 }
    );
    if let Some(temp) = &p.temporary_signer {
        assert_eq!(p.message.account_keys[1], temp.pubkey());
        assert_ne!(temp.pubkey(), f.owner);
        assert_ne!(temp.pubkey(), f.ata());
    }
}

#[tokio::test]
async fn wsol_preparation_shared_four_shapes_match_original_messages_authorization_and_requests() {
    for (token, direction, amount) in [
        (None, Direction::Wrap, 1_000_000),
        (Some(2_000_000), Direction::Wrap, 1_000_000),
        (Some(2_000_000), Direction::Unwrap, 2_000_000),
        (Some(2_000_000), Direction::Unwrap, 1_000_000),
    ] {
        let mut f = Fixture::default();
        f.token = token;
        let q = quote(&f, direction, amount);
        let legacy = f.legacy();
        let old = super::super::prepare_unsigned_wsol_transaction(f.owner, &legacy.client(), &q)
            .await
            .unwrap();
        let r = runtime(f.clone(), Mode::Good);
        let new = inspect(&r, &f, &q).await.unwrap();
        assert_prepared(&new, &f, &q);
        assert_prepared(&old, &f, &q);
        assert_eq!(
            normalized(new.message.clone()),
            normalized(old.message.clone())
        );
        assert_eq!(evidence(&new), evidence(&old));
        let old_calls = legacy
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(m, p)| normalize_request(m, p.clone()))
            .collect::<Vec<_>>();
        let new_calls = r
            .test_transport()
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, v, _)| normalize_request(v["method"].as_str().unwrap(), v["params"].clone()))
            .collect::<Vec<_>>();
        assert_eq!(old_calls, new_calls);
        assert_eq!(new_calls.len(), 5);
        assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
    }
}
async fn compare_failure(f: &Fixture, q: &WsolQuoteSnapshot, count: usize, kind: WsolFailureKind) {
    let legacy = f.legacy();
    let old = super::super::prepare_unsigned_wsol_transaction(f.owner, &legacy.client(), q)
        .await
        .err()
        .unwrap();
    let r = runtime(f.clone(), Mode::Good);
    let new = inspect(&r, f, q).await.err().unwrap();
    assert_eq!(old.to_string(), new.to_string());
    assert_eq!(new.downcast_ref::<WsolFailure>().unwrap().kind, kind);
    assert_eq!(legacy.calls.lock().unwrap().len(), count);
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), count);
    assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
}
#[tokio::test]
async fn wsol_preparation_shared_changed_snapshot_rejects_at_original_read_without_signing() {
    let f = Fixture::default();
    for (case, count) in [(0, 2), (1, 2), (2, 3), (3, 5), (4, 2), (5, 0)] {
        let mut q = quote(&f, Direction::Unwrap, 1_000_000);
        match case {
            0 => q.ata_exists = false,
            1 => q.source_balance_lamports -= 1,
            2 => q.temporary_rent_lamports -= 1,
            3 => q.estimated_network_fee_lamports -= 1,
            4 => {
                q.amount_lamports = 2_000_000;
                q.rent_refund_lamports = 1;
            }
            _ => q.wallet_pubkey = key(99).to_string(),
        }
        compare_failure(
            &f,
            &q,
            count,
            if case == 5 {
                WsolFailureKind::Integrity
            } else {
                WsolFailureKind::Rejected
            },
        )
        .await;
    }
}
#[tokio::test]
async fn wsol_preparation_shared_original_token_identity_native_and_pack_failures_are_integrity() {
    for corrupt in 1..=5 {
        let mut f = Fixture::default();
        f.corrupt = corrupt;
        compare_failure(
            &f,
            &quote(&f, Direction::Unwrap, 1_000_000),
            2,
            WsolFailureKind::Integrity,
        )
        .await;
    }
}
#[tokio::test]
async fn wsol_preparation_shared_exact_fee_rent_reserve_budget_overflow_and_u64_are_preserved() {
    for (sol, rent, fee, ok) in [
        (3_144_280, 2_039_280, 5000, true),
        (3_144_279, 2_039_280, 5000, false),
        (u64::MAX, u64::MAX, 5000, false),
        (u64::MAX, 0, 0, true),
    ] {
        let mut f = Fixture::default();
        f.token = None;
        f.sol = sol;
        f.rent = rent;
        f.fee = Some(fee);
        let q = quote(&f, Direction::Wrap, 1_000_000);
        if !ok {
            compare_failure(&f, &q, 5, WsolFailureKind::Rejected).await;
            continue;
        }
        let legacy = f.legacy();
        let old = super::super::prepare_unsigned_wsol_transaction(f.owner, &legacy.client(), &q)
            .await
            .unwrap();
        let r = runtime(f.clone(), Mode::Good);
        let new = inspect(&r, &f, &q).await.unwrap();
        assert_eq!(evidence(&new), evidence(&old));
        assert_prepared(&new, &f, &q);
    }
    let mut f = Fixture::default();
    f.sol = u64::MAX;
    f.token = Some(u64::MAX);
    f.rent = 0;
    f.fee = Some(0);
    let q = quote(&f, Direction::Unwrap, u64::MAX);
    let r = runtime(f.clone(), Mode::Good);
    let p = inspect(&r, &f, &q).await.unwrap();
    assert_prepared(&p, &f, &q);
    assert_eq!(p.authorization.wsol_balance_lamports, u64::MAX);
}
#[tokio::test]
async fn wsol_preparation_shared_partial_unwrap_retains_original_temporary_signer_until_consumed() {
    let f = Fixture::default();
    let q = quote(&f, Direction::Unwrap, 1_000_000);
    let r = runtime(f.clone(), Mode::Good);
    let p = inspect(&r, &f, &q).await.unwrap();
    assert_prepared(&p, &f, &q);
    let temp = p.temporary_signer.as_ref().unwrap().pubkey();
    let expected = p.message.clone();
    let (message, signer) = p.into_signing_parts(&f.owner, &q, 1).unwrap();
    assert_eq!(message, expected);
    assert_eq!(signer.unwrap().pubkey(), temp);
    assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
    assert!(r
        .test_transport()
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(_, v, _)| v["method"] != "sendTransaction" && v["method"] != "simulateTransaction"));
}
#[tokio::test]
async fn wsol_preparation_shared_rechecks_all_five_reads_after_quote_without_cached_evidence() {
    let f = Fixture::default();
    let r = runtime(f.clone(), Mode::Good);
    crate::adapters::sdk::wsol_quote::estimate(
        &r,
        &urls()[0],
        CommitmentConfig::confirmed(),
        f.owner,
        &Fixture::input(Direction::Wrap, Some(1_000_000), false),
    )
    .await
    .unwrap();
    let q = quote(&f, Direction::Wrap, 1_000_000);
    let p = inspect(&r, &f, &q).await.unwrap();
    assert_prepared(&p, &f, &q);
    let calls = r.test_transport().calls.lock().unwrap();
    assert_eq!(calls.len(), 10);
    for n in 0..5 {
        assert_eq!(calls[n].1["method"], calls[n + 5].1["method"]);
        assert_ne!(calls[n].1["id"], calls[n + 5].1["id"]);
    }
}
#[tokio::test]
async fn wsol_preparation_shared_missing_fee_is_unavailable_but_explicit_zero_is_not_invented() {
    for fee in [None, Some(0)] {
        let mut f = Fixture::default();
        f.fee = fee;
        let q = quote(&f, Direction::Wrap, 1_000_000);
        let old =
            super::super::prepare_unsigned_wsol_transaction(f.owner, &f.legacy().client(), &q)
                .await;
        let r = runtime(f.clone(), Mode::Good);
        let new = inspect(&r, &f, &q).await;
        assert_eq!(new.is_ok(), fee.is_some());
        assert_eq!(new.is_ok(), old.is_ok());
        if let Ok(p) = new {
            assert_prepared(&p, &f, &q);
            assert_eq!(p.authorization.network_fee_lamports, 0);
        }
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 5);
    }
}
#[tokio::test]
async fn wsol_preparation_shared_each_stage_transport_identity_size_and_429_fail_without_fallback()
{
    for n in 0..5 {
        for mode in [
            Mode::WrongId(n),
            Mode::Oversize(n),
            Mode::Error(n),
            Mode::Rate(n),
        ] {
            let f = Fixture::default();
            let q = quote(&f, Direction::Wrap, 1_000_000);
            let r = runtime(f.clone(), mode);
            let error = inspect(&r, &f, &q).await.err().unwrap();
            assert!(!format!("{error:?}").contains("private transport detail"));
            let calls = r.test_transport().calls.lock().unwrap();
            assert_eq!(calls.len(), n + 1);
            assert!(calls.iter().all(|(url, _, _)| url == &urls()[0]));
            drop(calls);
            if matches!(mode, Mode::Rate(_)) {
                assert!(r.wsol_quote_slot(&urls()[0]).is_err());
            } else {
                assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
            }
        }
    }
}
#[tokio::test]
async fn wsol_preparation_shared_each_stage_cancel_releases_quote_permit_without_blocking_hash() {
    for n in 0..5 {
        let f = Fixture::default();
        let q = quote(&f, Direction::Unwrap, 1_000_000);
        let r = runtime(f.clone(), Mode::Hold(n));
        let task = {
            let r = r.clone();
            let f = f.clone();
            tokio::spawn(async move { inspect(&r, &f, &q).await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if r.test_transport().calls.lock().unwrap().len() == n + 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(r.wsol_quote_slot(&urls()[0]).is_err());
        assert!(r.parameter_slot().is_ok());
        r.latest_blockhash().await.unwrap();
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
        assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), n + 2);
    }
}
#[tokio::test]
async fn wsol_preparation_shared_busy_unknown_endpoint_and_wrong_wallet_send_nothing() {
    let f = Fixture::default();
    let q = quote(&f, Direction::Wrap, 1_000_000);
    let r = runtime(f.clone(), Mode::Good);
    let scope =
        WsolQuoteReadSession::new(&r, &urls()[0], CommitmentConfig::confirmed(), f.owner).unwrap();
    assert!(inspect(&r, &f, &q).await.is_err());
    let mut bad = q.clone();
    bad.wallet_pubkey = key(99).to_string();
    let err = inspect(&r, &f, &bad).await.err().unwrap();
    assert_eq!(
        err.downcast_ref::<WsolFailure>().unwrap().kind,
        WsolFailureKind::Integrity
    );
    drop(scope);
    assert!(prepare(
        f.owner,
        &r,
        "https://unregistered.example.invalid",
        CommitmentConfig::confirmed(),
        &q
    )
    .await
    .is_err());
    assert!(r.test_transport().calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn wsol_preparation_shared_first_and_last_timeout_or_late_poll_never_return_candidate() {
    for n in [0, 4] {
        for mode in [Mode::Hold(n), Mode::Late(n)] {
            let f = Fixture::default();
            let q = quote(&f, Direction::Wrap, 1_000_000);
            let r = runtime(f.clone(), mode);
            let start = Instant::now();
            assert!(inspect(&r, &f, &q).await.is_err());
            assert!(start.elapsed() < Duration::from_secs(4));
            assert_eq!(r.test_transport().calls.lock().unwrap().len(), n + 1);
            assert_eq!(r.test_transport().active.load(Ordering::SeqCst), 0);
            assert!(r.wsol_quote_slot(&urls()[0]).is_ok());
        }
    }
}
#[tokio::test]
async fn wsol_preparation_shared_produced_candidate_rejects_quote_message_and_signer_substitution()
{
    for case in 0..7 {
        let f = Fixture::default();
        let q = quote(&f, Direction::Unwrap, 1_000_000);
        let r = runtime(f.clone(), Mode::Good);
        let mut p = inspect(&r, &f, &q).await.unwrap();
        let mut requested = q.clone();
        match case {
            0 => requested.amount_lamports += 1,
            1 => requested.wallet_id.push('x'),
            2 => requested.owner_id.push('x'),
            3 => p.message.recent_blockhash = Hash::default(),
            4 => p.message.instructions[0].data.push(0),
            5 => p.temporary_signer = None,
            _ => p.message.account_keys[1] = key(99),
        }
        assert!(p.into_signing_parts(&f.owner, &requested, 1).is_err());
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 5);
    }
}
