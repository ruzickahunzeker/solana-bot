//! The real SDK controller/journal with independently constructed candidate representations.
use super::*;

mod tape;
use tape::{TapeBackend, TapeCandidate};

#[derive(Clone, Copy, Debug)]
enum Style {
    Native,
    Tape,
}

struct Case {
    execution: Arc<AdmittedTrade>,
    owner: ExecutionOwner,
    journal: Journal,
    control: Arc<Control>,
    signer: Arc<Keypair>,
}

async fn case() -> Case {
    let signer = Arc::new(Keypair::new());
    let state = fixture(TradePersistenceMode::Durable).await.state;
    let request = serde_json::from_value(serde_json::json!({
        "wallet_id":"fixture-wallet", "side":"sell", "pool":"meteora-damm-v2",
        "mint":Address::from_bytes([2;32]).to_string(),
        "quote_mint":crate::domain::trade::WSOL_MINT,
        "pool_id":Address::from_bytes([3;32]).to_string(),
        "sell_percent":100, "slippage_bps":1500, "tip_sol":0.001,
        "use_mev":false, "simulate":false, "execution_method":"sdk",
        "idempotency_key":" independent-sdk-literal "
    }))
    .unwrap();
    let f = admit_request_for_pubkey(state, request, signer.pubkey().to_string()).await;
    let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
        panic!("original SDK owner required")
    };
    let record = f
        .state
        .trades
        .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
        .unwrap();
    f.state.persistence.update(&record).await.unwrap();
    let pool = f.state.persistence.pool_for_tests();
    let control = Arc::new(Control::default());
    Case {
        execution: f.execution,
        owner,
        journal: Journal {
            store: SqliteOrdinaryExecutionClaims::new(pool.clone()),
            pool,
            control: control.clone(),
        },
        control,
        signer,
    }
}

async fn run(style: Style, h: Case) -> Run {
    let control = h.control.clone();
    let outcome = match style {
        Style::Native => {
            let candidate = candidate_fixture_for_wallet(h.signer.pubkey());
            let backend = First(Engine {
                signer: h.signer.clone(),
                control: control.clone(),
                pool: h.journal.pool.clone(),
                request_id: h.execution.reservation().request_id().into(),
                execution: h.execution.clone(),
                alternate_execution: None,
            });
            execute(&h.journal, &backend, &h.execution, h.owner, candidate).await
        }
        Style::Tape => {
            let backend = TapeBackend {
                signer: h.signer.clone(),
                control: control.clone(),
                pool: h.journal.pool.clone(),
                request_id: h.execution.reservation().request_id().into(),
            };
            // Only the original admitted intent and public fixture pool state are inputs.
            let candidate = TapeCandidate::build(&h.execution).unwrap();
            execute(&h.journal, &backend, &h.execution, h.owner, candidate).await
        }
    };
    Run {
        outcome,
        execution: h.execution,
        journal: h.journal,
        control,
    }
}

#[tokio::test]
async fn sdk_independent_backend_constructs_matching_nominal_message_without_candidate_conversion()
{
    let h = case().await;
    let native = candidate_fixture_for_wallet(h.signer.pubkey());
    let tape = TapeCandidate::build(&h.execution).unwrap();
    let message = tape.message().unwrap();
    assert_eq!(message, native.message_bytes());
    assert_eq!(tape.record(), native.metadata());
    assert_eq!(
        tape.record().encode(&message).unwrap(),
        native.metadata().encode(&message).unwrap()
    );
    // Neither construction has released the journal-bound signing permit or acquired a key.
    assert_eq!(h.control.keys.load(Ordering::SeqCst), 0);
    assert_eq!(h.control.signs.load(Ordering::SeqCst), 0);
    assert_eq!(h.control.sends.load(Ordering::SeqCst), 0);
    assert_eq!(stage(&h.journal, &h.execution).await, "claimed");
}

#[tokio::test]
async fn sdk_independent_backend_obeys_original_controller_order_and_single_journal_authority() {
    let mut traces = Vec::new();
    for style in [Style::Native, Style::Tape] {
        let r = run(style, case().await).await;
        let record = r.outcome.record.unwrap();
        assert_eq!(record.status, TradeStatus::Submitted);
        assert_eq!(record.signature, r.outcome.signature);
        assert!(record.error.is_none() && r.outcome.error.is_none());
        assert_eq!(r.control.keys.load(Ordering::SeqCst), 1);
        assert_eq!(r.control.signs.load(Ordering::SeqCst), 1);
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 1);
        assert_eq!(stage(&r.journal, &r.execution).await, "submitted");
        assert_eq!(
            r.journal.store.claim(&r.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        traces.push(r.control.events.lock().unwrap().clone());
    }
    assert_eq!(traces[0], traces[1]);
    let events = &traces[0];
    for (before, after) in [
        ("prepare-after", "read-prepared"),
        ("read-prepared", "begin-signing"),
        ("begin-signing-after", "acquire-signer"),
        ("record-signed-after", "read-signed"),
        ("read-signed", "take-broadcast"),
        ("take-broadcast-after", "broadcast"),
    ] {
        assert!(
            events.iter().position(|v| v == before).unwrap()
                < events.iter().position(|v| v == after).unwrap()
        );
    }
}

#[tokio::test]
async fn sdk_independent_backend_storage_failures_preserve_identical_signing_and_send_boundaries() {
    for failure in [
        "prepare",
        "prepare-after",
        "read-prepared",
        "begin-signing",
        "begin-signing-after",
        "record-signed",
        "record-signed-after",
        "read-signed",
        "take-broadcast",
        "take-broadcast-after",
        "record-broadcast",
        "record-broadcast-after",
    ] {
        let mut outcomes = Vec::new();
        for style in [Style::Native, Style::Tape] {
            let h = case().await;
            *h.control.fail.lock().unwrap() = Some(failure);
            let r = run(style, h).await;
            let signs = r.control.signs.load(Ordering::SeqCst);
            let sends = r.control.sends.load(Ordering::SeqCst);
            assert!(signs <= 1 && sends <= 1, "{style:?}/{failure}");
            if matches!(
                failure,
                "prepare"
                    | "prepare-after"
                    | "read-prepared"
                    | "begin-signing"
                    | "begin-signing-after"
            ) {
                assert_eq!(r.control.keys.load(Ordering::SeqCst), 0);
                assert_eq!(signs, 0);
            }
            if !failure.starts_with("record-broadcast") {
                assert_eq!(sends, 0);
            }
            assert_eq!(
                r.journal.store.claim(&r.execution).await.unwrap(),
                ExecutionClaim::AlreadyHeld
            );
            outcomes.push((
                signs,
                sends,
                r.outcome.signature.is_some(),
                r.outcome.record.as_ref().map(|v| v.status),
                stage(&r.journal, &r.execution).await,
            ));
        }
        assert_eq!(outcomes[0], outcomes[1], "{failure}");
    }
}

#[tokio::test]
async fn sdk_independent_backend_readback_tampering_never_releases_changed_bytes_to_sender() {
    for tamper in ["prepared", "signed", "taken"] {
        let mut traces = Vec::new();
        for style in [Style::Native, Style::Tape] {
            let h = case().await;
            *h.control.tamper.lock().unwrap() = Some(tamper);
            let r = run(style, h).await;
            assert_eq!(r.control.sends.load(Ordering::SeqCst), 0);
            assert_eq!(
                r.control.keys.load(Ordering::SeqCst),
                usize::from(tamper != "prepared")
            );
            assert!(r.outcome.error.is_some());
            assert_eq!(
                r.journal.store.claim(&r.execution).await.unwrap(),
                ExecutionClaim::AlreadyHeld
            );
            traces.push((
                r.outcome.signature.is_some(),
                stage(&r.journal, &r.execution).await,
            ));
        }
        assert_eq!(traces[0], traces[1]);
    }
}

#[tokio::test]
async fn sdk_independent_backend_send_failure_keeps_original_signature_and_forbids_new_owner() {
    for response in [1, 2] {
        for style in [Style::Native, Style::Tape] {
            let h = case().await;
            h.control.response.store(response, Ordering::SeqCst);
            let r = run(style, h).await;
            let record = r.outcome.record.as_ref().unwrap();
            assert_eq!(record.status, TradeStatus::Unknown);
            assert_eq!(record.signature, r.outcome.signature);
            let signature = r.outcome.signature.unwrap();
            assert_eq!(r.control.keys.load(Ordering::SeqCst), 1);
            assert_eq!(r.control.signs.load(Ordering::SeqCst), 1);
            assert_eq!(r.control.sends.load(Ordering::SeqCst), 1);
            assert_eq!(stage(&r.journal, &r.execution).await, "unknown");
            let row = sqlx::query("SELECT signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
                .bind(r.execution.reservation().request_id()).fetch_one(&r.journal.pool).await.unwrap();
            assert_eq!(
                row.try_get::<&str, _>("signed_signature").unwrap(),
                signature
            );
            assert!(!row
                .try_get::<Vec<u8>, _>("signed_transaction")
                .unwrap()
                .is_empty());
            assert!(!r.outcome.error.unwrap().contains("private-token"));
            assert_eq!(
                r.journal.store.claim(&r.execution).await.unwrap(),
                ExecutionClaim::AlreadyHeld
            );
            // Re-observing through the original journal cannot invoke either backend again.
            let captured = r
                .journal
                .store
                .capture_observation(r.execution.reservation().request_id())
                .await
                .unwrap();
            assert_eq!(captured.context.signature(), signature);
            let observed = r
                .journal
                .store
                .read_observation(&captured.context)
                .await
                .unwrap();
            assert_eq!(observed.status, TradeStatus::Unknown);
            assert_eq!(observed.signature.as_deref(), Some(signature.as_str()));
            assert_eq!(r.control.signs.load(Ordering::SeqCst), 1);
            assert_eq!(r.control.sends.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn sdk_independent_backend_cancellation_retains_original_stage_and_signed_evidence() {
    for (gate, expected, signed, sends) in [
        ("prepare-after", "prepared", false, 0),
        ("begin-signing-after", "signing", false, 0),
        ("record-signed-after", "signed", true, 0),
        ("broadcast", "broadcasting", true, 1),
    ] {
        for style in [Style::Native, Style::Tape] {
            let h = case().await;
            let control = h.control.clone();
            let pool = h.journal.pool.clone();
            let execution = h.execution.clone();
            *control.block.lock().unwrap() = Some(gate);
            let task = tokio::spawn(run(style, h));
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                control.reached.notified(),
            )
            .await
            .unwrap();
            task.abort();
            assert!(task.await.err().unwrap().is_cancelled());
            let row = sqlx::query("SELECT submission_stage,signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
                .bind(execution.reservation().request_id()).fetch_one(&pool).await.unwrap();
            assert_eq!(
                row.try_get::<&str, _>("submission_stage").unwrap(),
                expected
            );
            assert_eq!(
                row.try_get::<Option<String>, _>("signed_signature")
                    .unwrap()
                    .is_some(),
                signed
            );
            assert_eq!(
                row.try_get::<Option<Vec<u8>>, _>("signed_transaction")
                    .unwrap()
                    .is_some(),
                signed
            );
            assert_eq!(control.keys.load(Ordering::SeqCst), usize::from(signed));
            assert_eq!(control.signs.load(Ordering::SeqCst), usize::from(signed));
            assert_eq!(control.sends.load(Ordering::SeqCst), sends);
            let store = SqliteOrdinaryExecutionClaims::new(pool);
            assert_eq!(
                store.claim(&execution).await.unwrap(),
                ExecutionClaim::AlreadyHeld
            );
            let before = control.events.lock().unwrap().clone();
            control.release.notify_one();
            tokio::task::yield_now().await;
            assert_eq!(*control.events.lock().unwrap(), before);
        }
    }
}

#[tokio::test]
async fn sdk_independent_backend_rejects_changed_recipe_before_key_and_changed_signed_packet_before_send(
) {
    for (tamper, signs) in [("tape-recipe", 0), ("tape-permit", 0), ("tape-signed", 1)] {
        let h = case().await;
        *h.control.tamper.lock().unwrap() = Some(tamper);
        let r = run(Style::Tape, h).await;
        assert_eq!(r.control.keys.load(Ordering::SeqCst), signs);
        assert_eq!(r.control.signs.load(Ordering::SeqCst), signs);
        assert_eq!(r.control.sends.load(Ordering::SeqCst), 0);
        assert!(r.outcome.record.is_none() && r.outcome.error.is_some());
        if tamper == "tape-permit" {
            assert!(r
                .outcome
                .error
                .as_deref()
                .unwrap()
                .contains("SDK signing permit candidate differs from the original journal"));
        }
        assert_eq!(
            r.journal.store.claim(&r.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
}
