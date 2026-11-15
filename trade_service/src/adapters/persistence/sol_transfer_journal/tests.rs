use super::*;
use crate::{
    config::{WalletStorageConfig, WalletStorageMode},
    domain::{sol_transfer::SolTransferAmount, trade::Address},
    wallet_repository::{SolTransferRecord, WalletRepository},
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Barrier;

fn intent() -> SolTransferIntent {
    SolTransferIntent {
        idempotency_key: " fixture-key ".into(),
        owner_type: "telegram".into(),
        owner_id: "42".into(),
        wallet_id: "fixture-wallet".into(),
        source: Address::from_bytes([1; 32]),
        destination: Address::from_bytes([2; 32]),
        amount: SolTransferAmount::Fixed(1),
    }
}
fn snapshot() -> SolTransferSnapshot {
    SolTransferSnapshot {
        source: intent().source,
        destination: intent().destination,
        amount_lamports: 1,
        fee_lamports: 5000,
        balance_before_lamports: 1_000_000,
        reserve_lamports: 900_880,
    }
}
fn signed() -> SignedSubmission {
    SignedSubmission {
        signature: "synthetic-signature".into(),
        bytes: vec![1, 2, 3],
    }
}
async fn fixture() -> (WalletRepository, SqliteSolTransferJournal, SqlitePool) {
    let (repo, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    let journal = repo.sol_transfer_journal().unwrap();
    (repo, journal, pool)
}
async fn reserve(journal: &SqliteSolTransferJournal, intent: &SolTransferIntent) -> String {
    let SolTransferReservation::Created(entry) = journal.reserve(intent).await.unwrap() else {
        panic!("new reservation required")
    };
    entry.record.transfer_id
}
async fn load(journal: &SqliteSolTransferJournal) -> SolTransferJournalEntry {
    journal
        .lookup(&intent().idempotency_key)
        .await
        .unwrap()
        .unwrap()
}
async fn counts(pool: &SqlitePool) -> (i64, i64) {
    (
        sqlx::query_scalar("SELECT COUNT(*) FROM wallet_sol_transfers")
            .fetch_one(pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT COUNT(*) FROM wallet_sol_transfer_attempts")
            .fetch_one(pool)
            .await
            .unwrap(),
    )
}
fn legacy(intent: &SolTransferIntent) -> SolTransferRecord {
    let (fixed, percent) = intent.amount.legacy_params();
    SolTransferRecord {
        transfer_id: format!("legacy_{}", uuid::Uuid::new_v4().simple()),
        idempotency_key: intent.idempotency_key.clone(),
        owner_type: intent.owner_type.clone(),
        owner_id: intent.owner_id.clone(),
        wallet_id: intent.wallet_id.clone(),
        from_pubkey: intent.source.to_string(),
        to_pubkey: intent.destination.to_string(),
        amount_mode: if fixed.is_some() { "fixed" } else { "percent" }.into(),
        requested_amount_lamports: fixed,
        requested_percent: percent,
        actual_amount_lamports: None,
        fee_lamports: None,
        balance_before_lamports: None,
        status: "processing".into(),
        signature: None,
        error: None,
    }
}

#[test]
fn transfer_journal_codec_binds_every_intent_field_and_preserves_full_integer_values() {
    let base = intent();
    let (json, fingerprint) = codec::encode_intent(&base).unwrap();
    assert_eq!(codec::decode_intent(&json).unwrap(), base);
    assert_eq!(codec::encode_intent(&base).unwrap().1, fingerprint);
    for mode in 0..8 {
        let mut changed = base.clone();
        match mode {
            0 => changed.idempotency_key = "fixture-key".into(),
            1 => changed.owner_type = "other".into(),
            2 => changed.owner_id = "7".into(),
            3 => changed.wallet_id = "other".into(),
            4 => changed.source = Address::from_bytes([3; 32]),
            5 => changed.destination = Address::from_bytes([3; 32]),
            6 => changed.amount = SolTransferAmount::Fixed(u64::MAX),
            7 => changed.amount = SolTransferAmount::Half,
            _ => unreachable!(),
        }
        let (json, hash) = codec::encode_intent(&changed).unwrap();
        assert_ne!(hash, fingerprint);
        assert_eq!(codec::decode_intent(&json).unwrap(), changed);
    }
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    for field in ["schema_version", "operation", "mode"] {
        let original = value[field].clone();
        value[field] = if field == "schema_version" {
            json!(2)
        } else {
            json!("different")
        };
        assert!(codec::decode_intent(&value.to_string()).is_err());
        value[field] = original;
    }
    value["extra"] = json!(true);
    assert!(codec::decode_intent(&value.to_string()).is_err());
    let mut maximum = snapshot();
    maximum.amount_lamports = u64::MAX;
    maximum.balance_before_lamports = u64::MAX;
    assert_eq!(
        codec::decode_snapshot(&codec::encode_snapshot(&maximum).unwrap()).unwrap(),
        maximum
    );
}

#[tokio::test]
async fn transfer_journal_reservation_uses_global_key_and_complete_intent_not_just_owner() {
    let (repo, journal, pool) = fixture().await;
    assert!(journal.lookup("missing").await.unwrap().is_none());
    let id = reserve(&journal, &intent()).await;
    for _ in 0..2 {
        let journal = repo.sol_transfer_journal().unwrap();
        let SolTransferReservation::Existing(entry) = journal.reserve(&intent()).await.unwrap()
        else {
            panic!("must replay")
        };
        assert_eq!(entry.record.transfer_id, id);
        assert_eq!(entry.state, Some(State::Reserved));
    }
    for mode in 0..7 {
        let mut changed = intent();
        match mode {
            0 => changed.owner_type = "other".into(),
            1 => changed.owner_id = "other".into(),
            2 => changed.wallet_id = "other".into(),
            3 => changed.source = Address::from_bytes([3; 32]),
            4 => changed.destination = Address::from_bytes([3; 32]),
            5 => changed.amount = SolTransferAmount::Fixed(2),
            6 => changed.amount = SolTransferAmount::All,
            _ => unreachable!(),
        }
        assert!(matches!(
            journal.reserve(&changed).await.unwrap(),
            SolTransferReservation::Conflict
        ));
    }
    assert!(journal.lookup("fixture-key").await.unwrap().is_none());
    assert_eq!(counts(&pool).await, (1, 1));
}

#[tokio::test]
async fn transfer_journal_reserve_rolls_back_projection_if_attempt_insert_fails() {
    let (_, journal, pool) = fixture().await;
    sqlx::raw_sql("CREATE TRIGGER reject_attempt BEFORE INSERT ON wallet_sol_transfer_attempts BEGIN SELECT RAISE(FAIL,'synthetic attempt failure'); END;").execute(&pool).await.unwrap();
    assert!(journal.reserve(&intent()).await.is_err());
    assert_eq!(counts(&pool).await, (0, 0));
    sqlx::query("DROP TRIGGER reject_attempt")
        .execute(&pool)
        .await
        .unwrap();
    reserve(&journal, &intent()).await;
    assert_eq!(counts(&pool).await, (1, 1));
}

#[tokio::test]
async fn transfer_journal_signed_bytes_and_projection_commit_together_before_broadcast() {
    let (repo, journal, pool) = fixture().await;
    let id = reserve(&journal, &intent()).await;
    sqlx::raw_sql("CREATE TRIGGER reject_projection BEFORE UPDATE ON wallet_sol_transfers BEGIN SELECT RAISE(FAIL,'synthetic projection failure'); END;").execute(&pool).await.unwrap();
    assert!(journal
        .record_signed(&id, &snapshot(), &signed())
        .await
        .is_err());
    let entry = load(&journal).await;
    assert_eq!(entry.state, Some(State::Reserved));
    assert!(entry.record.signature.is_none());
    let bytes: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT signed_transaction FROM wallet_sol_transfer_attempts")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(bytes.is_none());
    sqlx::query("DROP TRIGGER reject_projection")
        .execute(&pool)
        .await
        .unwrap();
    journal
        .record_signed(&id, &snapshot(), &signed())
        .await
        .unwrap();
    let entry = load(&journal).await;
    assert_eq!(entry.state, Some(State::Signed));
    assert_eq!(entry.record.status, "unknown");
    assert_eq!(
        entry.record.signature.as_deref(),
        Some("synthetic-signature")
    );
    let original = repo
        .get_sol_transfer_by_idempotency(&intent().idempotency_key)
        .await
        .unwrap();
    assert_eq!(original.signature, entry.record.signature);
    assert_eq!(original.actual_amount_lamports, Some(1));
    let bytes: Vec<u8> =
        sqlx::query_scalar("SELECT signed_transaction FROM wallet_sol_transfer_attempts")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bytes, signed().bytes);
}

#[tokio::test]
async fn transfer_journal_transitions_are_cas_guarded_and_never_reset_signed_state() {
    let (_, journal, _) = fixture().await;
    let id = reserve(&journal, &intent()).await;
    assert!(journal.record_confirmed(&id).await.is_err());
    assert!(journal
        .record_broadcast(&id, State::Submitted, None)
        .await
        .is_err());
    journal
        .record_signed(&id, &snapshot(), &signed())
        .await
        .unwrap();
    assert!(journal
        .record_signed(&id, &snapshot(), &signed())
        .await
        .is_err());
    assert!(journal.fail_before_sign(&id, "late error").await.is_err());
    assert!(journal
        .record_broadcast(&id, State::Confirmed, None)
        .await
        .is_err());
    assert!(journal
        .record_broadcast(&id, State::Submitted, Some("wrong"))
        .await
        .is_err());
    journal
        .record_broadcast(&id, State::Submitted, None)
        .await
        .unwrap();
    journal.record_confirmed(&id).await.unwrap();
    assert_eq!(load(&journal).await.state, Some(State::Confirmed));
    assert!(journal.record_confirmed(&id).await.is_err());
    assert!(journal.fail_before_sign("missing", "error").await.is_err());
}

#[tokio::test]
async fn transfer_journal_before_sign_failure_and_unknown_are_terminal_replays_with_redacted_errors(
) {
    for before_sign in [false, true] {
        let (_, journal, _) = fixture().await;
        let id = reserve(&journal, &intent()).await;
        if before_sign {
            journal
                .fail_before_sign(&id, "synthetic https://rpc.example.invalid/private")
                .await
                .unwrap();
        } else {
            journal
                .record_signed(&id, &snapshot(), &signed())
                .await
                .unwrap();
            journal
                .record_broadcast(
                    &id,
                    State::Unknown,
                    Some("synthetic https://rpc.example.invalid/private"),
                )
                .await
                .unwrap();
        }
        let SolTransferReservation::Existing(entry) = journal.reserve(&intent()).await.unwrap()
        else {
            panic!("must not retry")
        };
        assert_eq!(
            entry.state,
            Some(if before_sign {
                State::Failed
            } else {
                State::Unknown
            })
        );
        assert_eq!(entry.record.signature.is_some(), !before_sign);
        assert!(!entry.record.error.unwrap().contains("https://"));
        assert!(journal
            .record_signed(&id, &snapshot(), &signed())
            .await
            .is_err());
    }
}

#[tokio::test]
async fn transfer_journal_rejects_bad_snapshot_empty_signature_and_projection_overflow() {
    let (_, journal, _) = fixture().await;
    let id = reserve(&journal, &intent()).await;
    for mode in 0..7 {
        let mut value = snapshot();
        match mode {
            0 => value.source = Address::from_bytes([3; 32]),
            1 => value.destination = Address::from_bytes([3; 32]),
            2 => value.amount_lamports = 2,
            3 => value.reserve_lamports = 0,
            4 => value.balance_before_lamports = 0,
            5 => value.balance_before_lamports = u64::MAX,
            6 => value.fee_lamports = u64::MAX,
            _ => unreachable!(),
        }
        assert!(journal.record_signed(&id, &value, &signed()).await.is_err());
        assert_eq!(load(&journal).await.state, Some(State::Reserved));
    }
    for signed in [
        SignedSubmission {
            signature: String::new(),
            bytes: vec![1],
        },
        SignedSubmission {
            signature: "fixture".into(),
            bytes: vec![],
        },
    ] {
        assert!(journal
            .record_signed(&id, &snapshot(), &signed)
            .await
            .is_err());
    }
}

#[tokio::test]
async fn transfer_journal_legacy_rows_are_observation_only_and_never_backfilled_or_retried() {
    for status in ["processing", "failed", "submitted", "confirmed"] {
        let (repo, journal, pool) = fixture().await;
        let row = legacy(&intent());
        repo.reserve_sol_transfer(&row).await.unwrap();
        if status != "processing" {
            repo.finish_sol_transfer(
                &row.transfer_id,
                status,
                None,
                None,
                None,
                matches!(status, "submitted" | "confirmed").then_some("legacy-signature"),
                (status == "failed").then_some("legacy-error"),
            )
            .await
            .unwrap();
        }
        let before = repo
            .get_sol_transfer_by_idempotency(&intent().idempotency_key)
            .await
            .unwrap();
        journal.initialize().await.unwrap();
        let SolTransferReservation::Existing(entry) = journal.reserve(&intent()).await.unwrap()
        else {
            panic!("legacy must only replay")
        };
        assert!(entry.intent.is_none() && entry.state.is_none());
        assert_eq!(format!("{:?}", entry.record), format!("{before:?}"));
        assert!(journal
            .record_signed(&row.transfer_id, &snapshot(), &signed())
            .await
            .is_err());
        assert_eq!(counts(&pool).await, (1, 0));
    }
}

#[tokio::test]
async fn transfer_journal_canonical_intent_keeps_u64_max_while_legacy_projection_keeps_old_encoding(
) {
    let (repo, journal, pool) = fixture().await;
    let mut request = intent();
    request.amount = SolTransferAmount::Fixed(u64::MAX);
    reserve(&journal, &request).await;
    let entry = load(&journal).await;
    assert_eq!(entry.intent.unwrap(), request);
    assert_eq!(entry.record.requested_amount_lamports, Some(u64::MAX));
    let raw: i64 = sqlx::query_scalar("SELECT requested_amount_lamports FROM wallet_sol_transfers")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(raw, -1);
    assert_eq!(
        repo.get_sol_transfer_by_idempotency(&request.idempotency_key)
            .await
            .unwrap()
            .requested_amount_lamports,
        Some(u64::MAX)
    );
    let json: String = sqlx::query_scalar("SELECT intent_json FROM wallet_sol_transfer_attempts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(json.contains("18446744073709551615"));
}

#[tokio::test]
async fn transfer_journal_recovery_is_idempotent_and_preserves_original_signed_bytes() {
    let (_, journal, pool) = fixture().await;
    for signed_state in [false, true] {
        let mut request = intent();
        request.idempotency_key = format!("key-{signed_state}");
        let id = reserve(&journal, &request).await;
        if signed_state {
            journal
                .record_signed(&id, &snapshot(), &signed())
                .await
                .unwrap();
        }
    }
    journal.initialize().await.unwrap();
    journal.initialize().await.unwrap();
    for signed_state in [false, true] {
        let entry = journal
            .lookup(&format!("key-{signed_state}"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.state, Some(State::Unknown));
        assert_eq!(entry.record.signature.is_some(), signed_state);
        assert_eq!(entry.record.error.as_deref(), Some(RESTARTED));
        assert!(journal
            .record_signed(&entry.record.transfer_id, &snapshot(), &signed())
            .await
            .is_err());
    }
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT signed_transaction FROM wallet_sol_transfer_attempts WHERE signature IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bytes, signed().bytes);
}

#[tokio::test]
async fn transfer_journal_recovery_and_result_updates_roll_back_both_representations_on_failure() {
    let (_, journal, pool) = fixture().await;
    let id = reserve(&journal, &intent()).await;
    journal
        .record_signed(&id, &snapshot(), &signed())
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_projection BEFORE UPDATE ON wallet_sol_transfers BEGIN SELECT RAISE(FAIL,'synthetic update failure'); END;").execute(&pool).await.unwrap();
    assert!(journal.initialize().await.is_err());
    assert!(journal
        .record_broadcast(&id, State::Submitted, None)
        .await
        .is_err());
    let entry = load(&journal).await;
    assert_eq!(entry.state, Some(State::Signed));
    assert!(entry.record.error.is_none());
    sqlx::query("DROP TRIGGER reject_projection")
        .execute(&pool)
        .await
        .unwrap();
    journal
        .record_broadcast(&id, State::Submitted, None)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_projection BEFORE UPDATE ON wallet_sol_transfers BEGIN SELECT RAISE(FAIL,'synthetic update failure'); END;").execute(&pool).await.unwrap();
    assert!(journal.record_confirmed(&id).await.is_err());
    assert_eq!(load(&journal).await.state, Some(State::Submitted));
}

#[tokio::test]
async fn transfer_journal_corrupt_intent_projection_signature_and_snapshot_fail_closed() {
    for sql in [
        "UPDATE wallet_sol_transfer_attempts SET fingerprint=printf('%064d',0)",
        "UPDATE wallet_sol_transfer_attempts SET intent_json='{}'",
        "UPDATE wallet_sol_transfers SET to_pubkey='corrupt'",
        "UPDATE wallet_sol_transfers SET status='confirmed'",
        "UPDATE wallet_sol_transfers SET actual_amount_lamports=2",
        "UPDATE wallet_sol_transfer_attempts SET signature='different'",
        "UPDATE wallet_sol_transfer_attempts SET snapshot_json='{}'",
    ] {
        let (_, journal, pool) = fixture().await;
        let id = reserve(&journal, &intent()).await;
        journal
            .record_signed(&id, &snapshot(), &signed())
            .await
            .unwrap();
        sqlx::query(sql).execute(&pool).await.unwrap();
        assert!(
            journal.lookup(&intent().idempotency_key).await.is_err(),
            "{sql}"
        );
        assert!(journal.reserve(&intent()).await.is_err(), "{sql}");
    }
}

fn file_config() -> (std::path::PathBuf, WalletStorageConfig) {
    let dir = std::env::temp_dir().join(format!("native-journal-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = WalletStorageConfig {
        mode: WalletStorageMode::Database,
        database_url: format!("sqlite://{}", dir.join("wallets.sqlite").display()),
        busy_timeout_ms: 3000,
        passphrase: None,
    };
    (dir, config)
}

#[tokio::test]
async fn transfer_journal_file_restart_recovers_without_keys_and_all_connections_use_full_sync() {
    let (dir, config) = file_config();
    let repo = WalletRepository::new(&config, &[], "fixture-wallet")
        .await
        .unwrap();
    let journal = repo.sol_transfer_journal().unwrap();
    let mut connections = Vec::new();
    for _ in 0..5 {
        let mut connection = journal.pool.acquire().await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("PRAGMA synchronous")
                .fetch_one(&mut *connection)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
                .fetch_one(&mut *connection)
                .await
                .unwrap(),
            1
        );
        connections.push(connection);
    }
    drop(connections);
    let id = reserve(&journal, &intent()).await;
    journal
        .record_signed(&id, &snapshot(), &signed())
        .await
        .unwrap();
    journal.pool.close().await;
    drop(journal);
    drop(repo);
    let repo = WalletRepository::new(&config, &[], "fixture-wallet")
        .await
        .unwrap();
    let journal = repo.sol_transfer_journal().unwrap();
    let entry = load(&journal).await;
    assert_eq!(entry.state, Some(State::Unknown));
    assert_eq!(
        entry.record.signature.as_deref(),
        Some("synthetic-signature")
    );
    let bytes: Vec<u8> =
        sqlx::query_scalar("SELECT signed_transaction FROM wallet_sol_transfer_attempts")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
    assert_eq!(bytes, signed().bytes);
    assert!(matches!(
        journal.reserve(&intent()).await.unwrap(),
        SolTransferReservation::Existing(_)
    ));
    journal.pool.close().await;
    drop(journal);
    drop(repo);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn transfer_journal_cross_pool_concurrency_creates_only_one_atomic_attempt() {
    let (dir, config) = file_config();
    let first = WalletRepository::new(&config, &[], "fixture-wallet")
        .await
        .unwrap();
    let second = WalletRepository::new(&config, &[], "fixture-wallet")
        .await
        .unwrap();
    let first = Arc::new(first.sol_transfer_journal().unwrap());
    let second = Arc::new(second.sol_transfer_journal().unwrap());
    let barrier = Arc::new(Barrier::new(16));
    let mut tasks = Vec::new();
    for n in 0..16 {
        let journal = if n % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            journal.reserve(&intent()).await.unwrap()
        }));
    }
    let mut created = 0;
    for task in tasks {
        match task.await.unwrap() {
            SolTransferReservation::Created(_) => created += 1,
            SolTransferReservation::Existing(entry) => {
                assert_eq!(entry.state, Some(State::Reserved))
            }
            SolTransferReservation::Conflict => panic!("identical intent must replay"),
        }
    }
    assert_eq!(created, 1);
    assert_eq!(counts(&first.pool).await, (1, 1));
    first.pool.close().await;
    second.pool.close().await;
    drop(first);
    drop(second);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn transfer_journal_rejects_unsafe_connection_settings_instead_of_downgrading_durability() {
    let (_, journal, pool) = fixture().await;
    sqlx::query("PRAGMA synchronous=NORMAL")
        .execute(&pool)
        .await
        .unwrap();
    assert!(journal
        .reserve(&intent())
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("synchronous=FULL"));
    assert_eq!(counts(&pool).await, (0, 0));
}

#[tokio::test]
async fn transfer_journal_records_fixed_half_and_all_snapshots_with_exact_original_policy() {
    for (amount, units, reserve_lamports) in [
        (SolTransferAmount::Fixed(1), 1, 900_880),
        (SolTransferAmount::Half, 47_060, 900_880),
        (SolTransferAmount::All, 995_000, 0),
    ] {
        let (_, journal, _) = fixture().await;
        let mut request = intent();
        request.amount = amount;
        let id = reserve(&journal, &request).await;
        let mut value = snapshot();
        value.amount_lamports = units;
        value.reserve_lamports = reserve_lamports;
        journal.record_signed(&id, &value, &signed()).await.unwrap();
        let entry = load(&journal).await;
        assert_eq!(entry.record.actual_amount_lamports, Some(units));
        assert_eq!(entry.intent.unwrap().amount, amount);
    }
}

#[tokio::test]
async fn transfer_journal_initialize_does_not_rewrite_managed_terminal_or_submitted_results() {
    for state in [
        State::Submitted,
        State::Confirmed,
        State::Unknown,
        State::Failed,
    ] {
        let (_, journal, _) = fixture().await;
        let id = reserve(&journal, &intent()).await;
        if state == State::Failed {
            journal
                .fail_before_sign(&id, "synthetic before sign failure")
                .await
                .unwrap();
        } else {
            journal
                .record_signed(&id, &snapshot(), &signed())
                .await
                .unwrap();
            journal
                .record_broadcast(
                    &id,
                    if state == State::Confirmed {
                        State::Submitted
                    } else {
                        state
                    },
                    None,
                )
                .await
                .unwrap();
            if state == State::Confirmed {
                journal.record_confirmed(&id).await.unwrap();
            }
        }
        let before = load(&journal).await;
        journal.initialize().await.unwrap();
        let after = load(&journal).await;
        assert_eq!(after.state, before.state);
        assert_eq!(
            format!("{:?}", after.record),
            format!("{:?}", before.record)
        );
    }
}

#[tokio::test]
async fn transfer_journal_legacy_mismatched_or_unverifiable_content_never_gets_a_new_attempt() {
    for mode in 0..4 {
        let (repo, journal, pool) = fixture().await;
        let mut row = legacy(&intent());
        match mode {
            0 => row.requested_amount_lamports = Some(2),
            1 => row.to_pubkey = format!(" {} ", row.to_pubkey),
            2 => row.amount_mode = "corrupt".into(),
            3 => row.owner_id = "different".into(),
            _ => unreachable!(),
        }
        repo.reserve_sol_transfer(&row).await.unwrap();
        assert!(matches!(
            journal.reserve(&intent()).await.unwrap(),
            SolTransferReservation::Conflict
        ));
        assert_eq!(counts(&pool).await, (1, 0));
    }
}
