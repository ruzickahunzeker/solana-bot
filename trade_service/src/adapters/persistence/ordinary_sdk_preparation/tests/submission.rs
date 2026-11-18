use super::*;
use crate::{
    adapters::sdk::damm_sell_message::{candidate_fixture_for_wallet, validate_signed_candidate},
    domain::submission::SignedSubmission,
};
use solana_sdk::{
    signature::{Keypair, Signature, Signer},
    transaction::VersionedTransaction,
};

pub(super) async fn signing_case(state: Option<crate::AppState>) -> (Case, Keypair) {
    signing_case_with_defaults(state, false).await
}

pub(super) async fn signing_case_with_defaults(
    state: Option<crate::AppState>,
    defaults: bool,
) -> (Case, Keypair) {
    let signer = Keypair::new();
    let candidate = candidate_fixture_for_wallet(signer.pubkey());
    let mut state = match state {
        Some(state) => state,
        None => fixture(TradePersistenceMode::Durable).await.state,
    };
    if defaults {
        state.edit_fixture_config(|config| {
            config.default_slippage_percent =
                candidate.metadata().sell().unwrap().details().slippage_bps as f64 / 100.0;
            config.default_tip_sol =
                f64::from_bits(candidate.metadata().sell().unwrap().details().tip_sol_bits);
            config.enable_simulate = false;
        });
    }
    let f = admit_candidate_with(state, &candidate, |r| {
        if defaults {
            r.slippage_bps = None;
            r.tip_sol = None;
            r.quote_mint = None;
            r.simulate = None;
            r.execution_method = None;
        }
    })
    .await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original SDK owner required")
    };
    let parent = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&parent).await.unwrap();
    let c = Case {
        store: SqliteOrdinaryExecutionClaims::new(f.state.persistence.pool_for_tests()),
        f,
        owner,
        candidate,
    };
    prepare(&c).await.unwrap();
    (c, signer)
}

fn signed(c: &Case, keypair: &Keypair) -> SignedSubmission {
    let message = c.candidate.message_bytes();
    let signature = keypair.sign_message(&message);
    let transaction = VersionedTransaction {
        signatures: vec![signature],
        message: bincode::deserialize(&message).unwrap(),
    };
    SignedSubmission {
        signature: signature.to_string(),
        bytes: bincode::serialize(&transaction).unwrap(),
    }
}

pub(super) async fn begin(c: &Case) -> Result<()> {
    c.store
        .begin_sdk_signing(
            &c.f.execution,
            &c.owner,
            c.candidate.metadata(),
            &c.candidate.message_bytes(),
        )
        .await
}

pub(super) async fn save_signed(c: &Case, signer: &Keypair) -> SignedSubmission {
    begin(c).await.unwrap();
    let signed = signed(c, signer);
    c.store
        .record_sdk_signed(&c.f.execution, &c.owner, &signed)
        .await
        .unwrap();
    signed
}

#[tokio::test]
async fn sdk_submission_journal_saves_verifiable_original_bytes_then_consumes_one_send_opportunity()
{
    let (c, signer) = signing_case(None).await;
    let expected = save_signed(&c, &signer).await;
    let saved = snapshot(&c).await;
    let stored = c
        .store
        .read_sdk_signed(&c.f.execution, &c.owner)
        .await
        .unwrap();
    assert_eq!(stored.signature, expected.signature);
    assert_eq!(stored.bytes, expected.bytes);
    assert_eq!(snapshot(&c).await, saved);
    let taken = c
        .store
        .take_sdk_broadcast(&c.f.execution, &c.owner, &stored)
        .await
        .unwrap();
    assert_eq!(taken.bytes, expected.bytes);
    let consumed = snapshot(&c).await;
    assert!(c
        .store
        .take_sdk_broadcast(&c.f.execution, &c.owner, &stored)
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, consumed);
    c.store
        .record_sdk_broadcast(&c.f.execution, &c.owner, &taken, true, None)
        .await
        .unwrap();
    let receipt = snapshot(&c).await;
    assert_eq!(receipt.0[0].as_deref(), Some("submitted"));
    assert_eq!(receipt.1["status"], "submitted");
    assert_eq!(receipt.1["signature"], expected.signature);
    assert!(c
        .store
        .record_sdk_broadcast(&c.f.execution, &c.owner, &taken, true, None)
        .await
        .is_err());
    assert!(begin(&c).await.is_err());
    assert_eq!(
        c.store.claim(&c.f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(snapshot(&c).await, receipt);
}

#[tokio::test]
async fn sdk_submission_journal_requires_each_original_stage_and_exact_prepared_metadata() {
    let (c, signer) = signing_case(None).await;
    let bytes = signed(&c, &signer);
    let before = snapshot(&c).await;
    assert!(c
        .store
        .record_sdk_signed(&c.f.execution, &c.owner, &bytes)
        .await
        .is_err());
    assert!(c
        .store
        .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
        .await
        .is_err());
    assert!(c
        .store
        .record_sdk_broadcast(&c.f.execution, &c.owner, &bytes, true, None)
        .await
        .is_err());
    let changed = candidate_fixture_for_wallet(Keypair::new().pubkey());
    assert!(c
        .store
        .begin_sdk_signing(
            &c.f.execution,
            &c.owner,
            changed.metadata(),
            &changed.message_bytes()
        )
        .await
        .is_err());
    let mut changed_message = c.candidate.message_bytes();
    changed_message[0] ^= 1;
    assert!(c
        .store
        .begin_sdk_signing(
            &c.f.execution,
            &c.owner,
            c.candidate.metadata(),
            &changed_message
        )
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
    begin(&c).await.unwrap();
    let before = snapshot(&c).await;
    assert!(begin(&c).await.is_err());
    assert!(prepare(&c).await.is_err());
    assert!(c
        .store
        .read_sdk_signed(&c.f.execution, &c.owner)
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_submission_journal_rejects_invalid_signatures_altered_messages_and_noncanonical_packets(
) {
    let (c, signer) = signing_case(None).await;
    begin(&c).await.unwrap();
    let before = snapshot(&c).await;
    let good = signed(&c, &signer);
    validate_signed_candidate(c.candidate.metadata(), &c.candidate.message_bytes(), &good).unwrap();
    for change in 0..10 {
        let mut changed = SignedSubmission {
            signature: good.signature.clone(),
            bytes: good.bytes.clone(),
        };
        match change {
            0 => changed.signature = Signature::default().to_string(),
            1 => changed.bytes.push(0),
            2 => changed.bytes.truncate(changed.bytes.len() - 1),
            3 => changed.bytes = vec![0; 1233],
            4 => changed.bytes.clear(),
            n => {
                let mut tx: VersionedTransaction = bincode::deserialize(&changed.bytes).unwrap();
                match n {
                    5 => tx.signatures[0] = Signature::default(),
                    6 => {
                        tx.signatures[0] = Keypair::new().sign_message(&c.candidate.message_bytes())
                    }
                    7 => tx.signatures.push(tx.signatures[0]),
                    8 => {
                        let solana_sdk::message::VersionedMessage::Legacy(m) = &mut tx.message
                        else {
                            unreachable!()
                        };
                        m.instructions.last_mut().unwrap().data.push(1);
                        tx.signatures[0] = signer.sign_message(&tx.message.serialize());
                    }
                    9 => {
                        let solana_sdk::message::VersionedMessage::Legacy(m) = &mut tx.message
                        else {
                            unreachable!()
                        };
                        m.recent_blockhash = solana_sdk::hash::Hash::new_unique();
                        tx.signatures[0] = signer.sign_message(&tx.message.serialize());
                    }
                    _ => unreachable!(),
                }
                changed.signature = tx.signatures[0].to_string();
                changed.bytes = bincode::serialize(&tx).unwrap();
            }
        }
        assert!(
            c.store
                .record_sdk_signed(&c.f.execution, &c.owner, &changed)
                .await
                .is_err(),
            "{change}"
        );
        assert_eq!(snapshot(&c).await, before, "{change}");
    }
}

#[tokio::test]
async fn sdk_submission_journal_wrong_owner_or_changed_parent_cannot_advance_any_stage() {
    let (c, signer) = signing_case(None).await;
    let wrong = ExecutionOwner::fixture(&c.f.execution);
    let bytes = signed(&c, &signer);
    let before = snapshot(&c).await;
    assert!(c
        .store
        .begin_sdk_signing(
            &c.f.execution,
            &wrong,
            c.candidate.metadata(),
            &c.candidate.message_bytes()
        )
        .await
        .is_err());
    begin(&c).await.unwrap();
    assert!(c
        .store
        .record_sdk_signed(&c.f.execution, &wrong, &bytes)
        .await
        .is_err());
    c.store
        .record_sdk_signed(&c.f.execution, &c.owner, &bytes)
        .await
        .unwrap();
    let saved = snapshot(&c).await;
    assert!(c
        .store
        .read_sdk_signed(&c.f.execution, &wrong)
        .await
        .is_err());
    assert!(c
        .store
        .take_sdk_broadcast(&c.f.execution, &wrong, &bytes)
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, saved);
    assert_eq!(saved.0[1], before.0[1]);
    for (column, value) in [
        ("wallet_id", "different"),
        ("wallet_pubkey", "different"),
        ("request_hash", "different"),
        ("idempotency_key", "trimmed"),
        ("request_json", "{}"),
        ("signature", "different"),
    ] {
        let original: String =
            sqlx::query_scalar(&format!("SELECT {column} FROM trades WHERE request_id=?"))
                .bind(&c.f.record.request_id)
                .fetch_one(&c.store.pool)
                .await
                .unwrap();
        let mut tx = c.store.pool.begin().await.unwrap();
        sqlx::query(&format!("UPDATE trades SET {column}=? WHERE request_id=?"))
            .bind(value)
            .bind(&c.f.record.request_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let corrupted = snapshot(&c).await;
        assert!(
            c.store
                .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
                .await
                .is_err(),
            "{column}"
        );
        assert_eq!(snapshot(&c).await, corrupted);
        sqlx::query(&format!("UPDATE trades SET {column}=? WHERE request_id=?"))
            .bind(original)
            .bind(&c.f.record.request_id)
            .execute(&c.store.pool)
            .await
            .unwrap();
    }
    assert_eq!(snapshot(&c).await, saved);
}

#[tokio::test]
async fn sdk_submission_journal_unknown_receipt_preserves_original_signature_without_new_send() {
    let (c, signer) = signing_case(None).await;
    let bytes = save_signed(&c, &signer).await;
    let before = snapshot(&c).await;
    assert!(c
        .store
        .record_sdk_broadcast(&c.f.execution, &c.owner, &bytes, false, Some("early"))
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
    c.store
        .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
        .await
        .unwrap();
    c.store
        .record_sdk_broadcast(
            &c.f.execution,
            &c.owner,
            &bytes,
            false,
            Some("https://rpc.invalid/private-token timed out"),
        )
        .await
        .unwrap();
    let before = snapshot(&c).await;
    assert_eq!(before.0[0].as_deref(), Some("unknown"));
    assert_eq!(before.1["signature"], bytes.signature);
    assert!(!before.1["error"]
        .as_str()
        .unwrap()
        .contains("private-token"));
    assert!(c
        .store
        .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
        .await
        .is_err());
    assert!(c
        .store
        .record_sdk_signed(&c.f.execution, &c.owner, &bytes)
        .await
        .is_err());
    assert!(begin(&c).await.is_err());
    assert_eq!(
        c.store.claim(&c.f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_submission_journal_failed_writes_and_bad_readbacks_roll_back_both_rows() {
    for stage in ["signing", "signed", "broadcasting", "submitted"] {
        for trigger in ["deny", "corrupt", "parent"] {
            let (c, signer) = signing_case(None).await;
            let bytes = signed(&c, &signer);
            if stage != "signing" {
                begin(&c).await.unwrap();
            }
            if ["broadcasting", "submitted"].contains(&stage) {
                c.store
                    .record_sdk_signed(&c.f.execution, &c.owner, &bytes)
                    .await
                    .unwrap();
            }
            if stage == "submitted" {
                c.store
                    .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
                    .await
                    .unwrap();
            }
            let statement = match trigger {
                "deny" => format!("CREATE TRIGGER fail_sdk BEFORE UPDATE OF submission_stage ON ordinary_execution_claims WHEN NEW.submission_stage='{stage}' BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END"),
                "corrupt" => format!("CREATE TRIGGER fail_sdk AFTER UPDATE OF submission_stage ON ordinary_execution_claims WHEN NEW.submission_stage='{stage}' BEGIN UPDATE ordinary_execution_claims SET unsigned_message=x'FF' WHERE request_id=NEW.request_id; END"),
                "parent" => format!("CREATE TRIGGER fail_sdk AFTER UPDATE OF submission_stage ON ordinary_execution_claims WHEN NEW.submission_stage='{stage}' BEGIN UPDATE trades SET request_hash='corrupted' WHERE request_id=NEW.request_id; END"),
                _ => unreachable!(),
            };
            sqlx::raw_sql(&statement)
                .execute(&c.store.pool)
                .await
                .unwrap();
            let before = snapshot(&c).await;
            let result = match stage {
                "signing" => begin(&c).await,
                "signed" => {
                    c.store
                        .record_sdk_signed(&c.f.execution, &c.owner, &bytes)
                        .await
                }
                "broadcasting" => c
                    .store
                    .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
                    .await
                    .map(|_| ()),
                "submitted" => {
                    c.store
                        .record_sdk_broadcast(&c.f.execution, &c.owner, &bytes, true, None)
                        .await
                }
                _ => unreachable!(),
            };
            assert!(result.is_err(), "{stage}/{trigger}");
            assert_eq!(snapshot(&c).await, before, "{stage}/{trigger}");
        }
    }
}

#[tokio::test]
async fn sdk_submission_journal_signing_and_send_consumption_allow_only_one_concurrent_caller() {
    let (c, signer) = signing_case(None).await;
    let c = Arc::new(c);
    let mut workers = Vec::new();
    for _ in 0..8 {
        let c = c.clone();
        workers.push(tokio::spawn(async move { begin(&c).await.is_ok() }));
    }
    let mut winners = 0;
    for worker in workers {
        winners += u8::from(worker.await.unwrap());
    }
    assert_eq!(winners, 1);
    let bytes = Arc::new(signed(&c, &signer));
    c.store
        .record_sdk_signed(&c.f.execution, &c.owner, &bytes)
        .await
        .unwrap();
    let mut workers = Vec::new();
    for _ in 0..8 {
        let c = c.clone();
        let bytes = bytes.clone();
        workers.push(tokio::spawn(async move {
            c.store
                .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
                .await
                .is_ok()
        }));
    }
    let mut winners = 0;
    for worker in workers {
        winners += u8::from(worker.await.unwrap());
    }
    assert_eq!(winners, 1);
    assert_eq!(snapshot(&c).await.0[0].as_deref(), Some("broadcasting"));
}

#[tokio::test]
async fn sdk_submission_journal_detects_signed_corruption_even_when_digest_was_recomputed() {
    let (c, signer) = signing_case(None).await;
    let bytes = save_signed(&c, &signer).await;
    for change in [
        "signature",
        "message",
        "unsigned",
        "partial",
        "stage",
        "quote",
    ] {
        let mut tx = c.store.pool.begin().await.unwrap();
        match change {
            "signature" => {
                sqlx::query("UPDATE ordinary_execution_claims SET signed_signature='different' WHERE request_id=?")
                    .bind(&c.f.record.request_id).execute(&mut *tx).await.unwrap();
            }
            "message" => {
                let mut altered: VersionedTransaction = bincode::deserialize(&bytes.bytes).unwrap();
                altered.signatures[0] = Signature::default();
                let altered = bincode::serialize(&altered).unwrap();
                sqlx::query("UPDATE ordinary_execution_claims SET signed_transaction=?,signed_sha256=? WHERE request_id=?")
                    .bind(&altered).bind(sha(&altered)).bind(&c.f.record.request_id).execute(&mut *tx).await.unwrap();
            }
            "unsigned" => {
                sqlx::query("UPDATE ordinary_execution_claims SET unsigned_sha256='changed' WHERE request_id=?")
                    .bind(&c.f.record.request_id).execute(&mut *tx).await.unwrap();
            }
            "partial" => {
                sqlx::query(
                    "UPDATE ordinary_execution_claims SET signed_sha256=NULL WHERE request_id=?",
                )
                .bind(&c.f.record.request_id)
                .execute(&mut *tx)
                .await
                .unwrap();
            }
            "stage" => {
                sqlx::query("UPDATE ordinary_execution_claims SET submission_stage='prepared' WHERE request_id=?")
                    .bind(&c.f.record.request_id).execute(&mut *tx).await.unwrap();
            }
            "quote" => {
                assert!(sqlx::query(
                    "UPDATE ordinary_execution_claims SET bound_quote_json='{}' WHERE request_id=?"
                )
                .persistent(false)
                .bind(&c.f.record.request_id)
                .execute(&mut *tx)
                .await
                .is_err());
                // Separately inject a damaged legacy/external row to exercise the reader.
                sqlx::query("PRAGMA ignore_check_constraints=ON")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                sqlx::query(
                    "UPDATE ordinary_execution_claims SET bound_quote_json='{}' WHERE request_id=?",
                )
                .persistent(false)
                .bind(&c.f.record.request_id)
                .execute(&mut *tx)
                .await
                .unwrap();
            }
            _ => unreachable!(),
        }
        let row = sqlx::query("SELECT * FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&c.f.record.request_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert!(validate_sdk_row(&row).is_err(), "{change}");
        tx.rollback().await.unwrap();
        sqlx::query("PRAGMA ignore_check_constraints=OFF")
            .execute(&c.store.pool)
            .await
            .unwrap();
    }
    assert_eq!(
        c.store
            .read_sdk_signed(&c.f.execution, &c.owner)
            .await
            .unwrap()
            .bytes,
        bytes.bytes
    );
}

#[tokio::test]
async fn sdk_submission_journal_receipt_cannot_replace_bytes_or_downgrade_to_presign_failure() {
    let (c, signer) = signing_case(None).await;
    let bytes = save_signed(&c, &signer).await;
    let before = snapshot(&c).await;
    let other = signed(&c, &Keypair::new());
    assert!(c
        .store
        .take_sdk_broadcast(&c.f.execution, &c.owner, &other)
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
    c.store
        .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
        .await
        .unwrap();
    let before = snapshot(&c).await;
    assert!(c
        .store
        .record_sdk_broadcast(&c.f.execution, &c.owner, &other, true, None)
        .await
        .is_err());
    assert!(c
        .store
        .record_sdk_broadcast(&c.f.execution, &c.owner, &bytes, true, Some("unexpected"))
        .await
        .is_err());
    assert!(c
        .store
        .record_sdk_broadcast(
            &c.f.execution,
            &c.owner,
            &bytes,
            false,
            Some(&"x".repeat(65_537))
        )
        .await
        .is_err());
    assert!(c
        .store
        .fail_before_sign(&c.f.execution, &c.owner, "do not downgrade SDK")
        .await
        .is_err());
    assert_eq!(snapshot(&c).await, before);
}

#[tokio::test]
async fn sdk_submission_journal_file_reopen_preserves_all_stages_without_new_owner_or_send_permit()
{
    use crate::{config::TradePersistenceConfig, persistence::TradePersistence};
    for stage in ["signing", "signed", "broadcasting", "submitted", "unknown"] {
        let dir = std::env::temp_dir().join(format!("sdk-signed-journal-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let config = TradePersistenceConfig {
            mode: TradePersistenceMode::Durable,
            database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
            busy_timeout_ms: 2_000,
        };
        let mut state = crate::http_contract_tests::state().await;
        state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
        let (c, signer) = signing_case(Some(state)).await;
        begin(&c).await.unwrap();
        let bytes = signed(&c, &signer);
        if stage != "signing" {
            c.store
                .record_sdk_signed(&c.f.execution, &c.owner, &bytes)
                .await
                .unwrap();
        }
        if ["broadcasting", "submitted", "unknown"].contains(&stage) {
            c.store
                .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
                .await
                .unwrap();
        }
        if ["submitted", "unknown"].contains(&stage) {
            c.store
                .record_sdk_broadcast(&c.f.execution, &c.owner, &bytes, stage == "submitted", None)
                .await
                .unwrap();
        }
        let before = snapshot(&c).await;
        c.store.pool.close().await;
        let restored = TradePersistence::new(&config).await.unwrap();
        let journal = restored.ordinary_execution_claims().unwrap();
        assert_eq!(
            journal.claim(&c.f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        let parent = restored.get(&c.f.record.request_id).await.unwrap().unwrap();
        assert!(matches!(
            parent.status,
            TradeStatus::Unknown | TradeStatus::Submitted
        ));
        assert!(journal
            .begin_sdk_signing(
                &c.f.execution,
                &c.owner,
                c.candidate.metadata(),
                &c.candidate.message_bytes()
            )
            .await
            .is_err());
        assert!(
            journal
                .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
                .await
                .is_err(),
            "{stage}"
        );
        if stage != "signing" {
            let stored = journal
                .read_sdk_signed(&c.f.execution, &c.owner)
                .await
                .unwrap();
            assert_eq!(stored.signature, bytes.signature);
            assert_eq!(stored.bytes, bytes.bytes);
            assert_eq!(parent.signature.as_deref(), Some(bytes.signature.as_str()));
        } else {
            assert!(parent.signature.is_none());
            assert!(journal
                .read_sdk_signed(&c.f.execution, &c.owner)
                .await
                .is_err());
        }
        let row: (String, String, Option<String>, Option<Vec<u8>>) = sqlx::query_as("SELECT submission_stage,owner_nonce,signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&c.f.record.request_id).fetch_one(&journal.pool).await.unwrap();
        assert_eq!(row.0, stage);
        assert_eq!(Some(row.1), before.0[1]);
        assert_eq!(
            row.2.as_deref(),
            (stage != "signing").then_some(bytes.signature.as_str())
        );
        if stage != "signing" {
            assert_eq!(row.3.unwrap(), bytes.bytes);
        }
        journal.pool.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[tokio::test]
async fn sdk_submission_journal_two_pools_share_the_same_single_signing_and_broadcast_cas() {
    use crate::{config::TradePersistenceConfig, persistence::TradePersistence};
    let dir = std::env::temp_dir().join(format!("sdk-journal-two-pools-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let config = TradePersistenceConfig {
        mode: TradePersistenceMode::Durable,
        database_url: format!("sqlite://{}", dir.join("test.sqlite").display()),
        busy_timeout_ms: 2_000,
    };
    let mut state = crate::http_contract_tests::state().await;
    state.persistence = Arc::new(TradePersistence::new(&config).await.unwrap());
    let (c, signer) = signing_case(Some(state)).await;
    let c = Arc::new(c);
    let second = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&config.database_url)
        .await
        .unwrap();
    let mut workers = Vec::new();
    for i in 0..8 {
        let c = c.clone();
        let pool = if i % 2 == 0 {
            c.store.pool.clone()
        } else {
            second.clone()
        };
        workers.push(tokio::spawn(async move {
            SqliteOrdinaryExecutionClaims::new(pool)
                .begin_sdk_signing(
                    &c.f.execution,
                    &c.owner,
                    c.candidate.metadata(),
                    &c.candidate.message_bytes(),
                )
                .await
                .is_ok()
        }));
    }
    let mut winners = 0;
    for worker in workers {
        winners += u8::from(worker.await.unwrap());
    }
    assert_eq!(winners, 1);
    let bytes = Arc::new(signed(&c, &signer));
    c.store
        .record_sdk_signed(&c.f.execution, &c.owner, &bytes)
        .await
        .unwrap();
    let mut workers = Vec::new();
    for i in 0..8 {
        let c = c.clone();
        let bytes = bytes.clone();
        let pool = if i % 2 == 0 {
            c.store.pool.clone()
        } else {
            second.clone()
        };
        workers.push(tokio::spawn(async move {
            SqliteOrdinaryExecutionClaims::new(pool)
                .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
                .await
                .is_ok()
        }));
    }
    let mut winners = 0;
    for worker in workers {
        winners += u8::from(worker.await.unwrap());
    }
    assert_eq!(winners, 1);
    c.store
        .record_sdk_broadcast(
            &c.f.execution,
            &c.owner,
            &bytes,
            false,
            Some("cancelled after sender entry"),
        )
        .await
        .unwrap();
    let other = SqliteOrdinaryExecutionClaims::new(second.clone());
    assert_eq!(
        other.claim(&c.f.execution).await.unwrap(),
        ExecutionClaim::AlreadyHeld
    );
    assert_eq!(
        other
            .read_sdk_signed(&c.f.execution, &c.owner)
            .await
            .unwrap()
            .bytes,
        bytes.bytes
    );
    assert!(other
        .take_sdk_broadcast(&c.f.execution, &c.owner, &bytes)
        .await
        .is_err());
    second.close().await;
    c.store.pool.close().await;
    std::fs::remove_dir_all(dir).unwrap();
}
