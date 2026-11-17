use super::*;
use crate::application::ports::ordinary_reconciliation::RecoveredOrdinaryObservations;

#[test]
fn sdk_original_buy_runtime_real_preparation_signing_journal_and_rpc_submit_for_both_assets() {
    child("buy-good");
}
#[test]
fn sdk_original_buy_runtime_invalid_raw_quote_or_final_preview_never_signs_or_submits() {
    child("buy-pre-sign");
}
#[test]
fn sdk_original_buy_runtime_wrong_receipt_or_lost_write_preserves_original_signature() {
    child("buy-post-sign");
}
#[test]
fn sdk_original_buy_runtime_cancellation_during_probe_or_send_never_retries() {
    child("buy-cancel");
}
#[test]
fn sdk_original_buy_runtime_presence_failure_or_cancel_never_reaches_probe_signer_or_send() {
    child("buy-presence");
}
#[test]
fn sdk_original_buy_runtime_parameter_failure_never_reaches_presence_probe_signer_or_send() {
    child("buy-parameters");
}

pub(super) async fn good() {
    for native in [false, true] {
        let mut c = Case::new_buy(Mode::Good, native).await;
        let outcome = c.run().await;
        assert_eq!(
            outcome.record.as_ref().unwrap().status,
            TradeStatus::Submitted,
            "{:?}",
            outcome.error
        );
        assert!(outcome.error.is_none());
        let packets = c.trace.sent.lock().unwrap().clone();
        assert_eq!(packets.len(), 1);
        let tx: VersionedTransaction = bincode::deserialize(&packets[0]).unwrap();
        assert_eq!(outcome.signature, Some(tx.signatures[0].to_string()));
        let simulations = c.trace.simulations.lock().unwrap().clone();
        assert_eq!(simulations.len(), 2);
        assert_eq!(simulations[1].message, tx.message);
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
        let requests = c.server.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0]["method"], "getAccountInfo");
        assert_eq!(requests[1]["method"], "getMultipleAccounts");
        assert_eq!(
            requests[2],
            json!({"jsonrpc":"2.0","id":3,"method":"getTokenAccountsByOwner","params":[c.wallet.wallet().to_string(),{"mint":key(4).to_string()},{"encoding":"jsonParsed","dataSlice":null,"commitment":"processed","minContextSlot":null}]})
        );
        let row=sqlx::query("SELECT sdk_prepared_json,unsigned_message FROM ordinary_execution_claims WHERE request_id=?").bind(&c.f.record.request_id).fetch_one(&c.pool).await.unwrap();
        let metadata = SdkPreparedRecord::decode(
            row.get("sdk_prepared_json"),
            &row.get::<Vec<u8>, _>("unsigned_message"),
        )
        .unwrap();
        let d = metadata.buy().unwrap().details();
        assert_eq!(d.input_units, 100);
        assert_eq!(d.minimum_output_units, 102);
        assert_eq!(
            d.quote_mint,
            if native {
                WSOL_TOKEN_ACCOUNT
            } else {
                USDC_TOKEN_ACCOUNT
            }
            .to_bytes()
        );
        // Capture must accept a real original buy, not merely its scan hint.
        let captured = c
            .journal
            .capture_observation(&c.f.record.request_id)
            .await
            .unwrap();
        assert_eq!(
            Some(captured.context.signature()),
            outcome.signature.as_deref()
        );
        c.unchanged_request().await;
        c.finish().await;
    }
}

pub(super) async fn pre_sign() {
    for native in [false, true] {
        for mode in [Mode::BadRaw, Mode::ZeroOutput, Mode::BadPreview] {
            let mut c = Case::new_buy(mode, native).await;
            let outcome = c.run().await;
            assert_eq!(outcome.record.unwrap().status, TradeStatus::Failed);
            assert!(outcome.error.is_some());
            assert!(outcome.signature.is_none());
            assert!(c.trace.sent.lock().unwrap().is_empty());
            let signatures: Option<String> = sqlx::query_scalar(
                "SELECT signed_signature FROM ordinary_execution_claims WHERE request_id=?",
            )
            .bind(&c.f.record.request_id)
            .fetch_one(&c.pool)
            .await
            .unwrap();
            assert!(signatures.is_none());
            assert_eq!(
                c.trace.simulations.lock().unwrap().len(),
                match mode {
                    Mode::BadRaw => 0,
                    Mode::ZeroOutput => 1,
                    _ => 2,
                }
            );
            c.unchanged_request().await;
            c.finish().await;
        }
    }
}

pub(super) async fn post_sign() {
    for native in [false, true] {
        for mode in [Mode::WrongReceipt, Mode::ReceiptWriteFailure] {
            let mut c = Case::new_buy(mode, native).await;
            let outcome = c.run().await;
            assert!(outcome.signature.is_some());
            assert!(outcome.error.is_some());
            let packets = c.trace.sent.lock().unwrap().clone();
            assert_eq!(packets.len(), 1);
            let tx: VersionedTransaction = bincode::deserialize(&packets[0]).unwrap();
            assert_eq!(outcome.signature, Some(tx.signatures[0].to_string()));
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
            assert_eq!(outcome.record.is_some(), mode == Mode::WrongReceipt);
            let row=sqlx::query("SELECT signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?").bind(&c.f.record.request_id).fetch_one(&c.pool).await.unwrap();
            assert_eq!(
                row.get::<String, _>("signed_signature"),
                tx.signatures[0].to_string()
            );
            assert_eq!(row.get::<Vec<u8>, _>("signed_transaction"), packets[0]);
            c.unchanged_request().await;
            c.finish().await;
        }
    }
}

pub(super) async fn cancel() {
    for native in [false, true] {
        for mode in [Mode::CancelProbe, Mode::CancelSend] {
            let mut c = Case::new_buy(mode, native).await;
            let trace = c.trace.clone();
            let execution = c.f.execution.clone();
            let pool = c.pool.clone();
            let task = tokio::spawn(async move { c.run().await });
            tokio::time::timeout(Duration::from_secs(3), trace.entered.notified())
                .await
                .unwrap();
            task.abort();
            assert!(matches!(task.await,Err(error) if error.is_cancelled()));
            assert!(trace.dropped.load(Ordering::SeqCst));
            let row=sqlx::query("SELECT submission_stage,signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?").bind(execution.reservation().request_id()).fetch_one(&pool).await.unwrap();
            let sending = mode == Mode::CancelSend;
            assert_eq!(trace.sent.lock().unwrap().len(), usize::from(sending));
            assert_eq!(
                row.get::<String, _>("submission_stage"),
                if sending { "broadcasting" } else { "claimed" }
            );
            assert_eq!(
                row.get::<Option<String>, _>("signed_signature").is_some(),
                sending
            );
            if sending {
                assert_eq!(
                    row.get::<Vec<u8>, _>("signed_transaction"),
                    trace.sent.lock().unwrap()[0]
                );
            }
            let before = trace.calls.lock().unwrap().len();
            assert_eq!(
                SqliteOrdinaryExecutionClaims::new(pool.clone())
                    .claim(&execution)
                    .await
                    .unwrap(),
                ExecutionClaim::AlreadyHeld
            );
            tokio::task::yield_now().await;
            assert_eq!(trace.calls.lock().unwrap().len(), before);
            pool.close().await;
        }
    }
}

pub(super) async fn presence_gate() {
    for native in [false, true] {
        for mode in [Mode::PresenceRate, Mode::PresenceInvalid] {
            let mut c = Case::new_buy(mode, native).await;
            let result = c.run().await;
            assert_eq!(result.record.unwrap().status, TradeStatus::Failed);
            assert!(result.error.is_some());
            assert!(result.signature.is_none());
            assert!(c.trace.simulations.lock().unwrap().is_empty());
            assert!(c.trace.sent.lock().unwrap().is_empty());
            assert_eq!(c.server.requests.lock().unwrap().len(), 3);
            let methods = c
                .trace
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|call| call.0)
                .collect::<Vec<_>>();
            assert!(methods.is_empty());
            let row = sqlx::query("SELECT submission_stage,signed_signature FROM ordinary_execution_claims WHERE request_id=?").bind(&c.f.record.request_id).fetch_one(&c.pool).await.unwrap();
            assert_eq!(row.get::<&str, _>("submission_stage"), "failed");
            assert!(row.get::<Option<String>, _>("signed_signature").is_none());
            assert_eq!(
                c.journal.claim(&c.f.execution).await.unwrap(),
                ExecutionClaim::AlreadyHeld
            );
            c.unchanged_request().await;
            c.finish().await;
        }
        let mut c = Case::new_buy(Mode::PresenceHold, native).await;
        let requests = c.server.requests.clone();
        let trace = c.trace.clone();
        let pool = c.pool.clone();
        let execution = c.f.execution.clone();
        let task = tokio::spawn(async move { c.run().await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request["method"] == "getTokenAccountsByOwner")
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert!(trace.simulations.lock().unwrap().is_empty());
        assert!(trace.sent.lock().unwrap().is_empty());
        assert_eq!(requests.lock().unwrap().len(), 3);
        let row = sqlx::query("SELECT submission_stage,signed_signature FROM ordinary_execution_claims WHERE request_id=?").bind(execution.reservation().request_id()).fetch_one(&pool).await.unwrap();
        assert_eq!(row.get::<&str, _>("submission_stage"), "claimed");
        assert!(row.get::<Option<String>, _>("signed_signature").is_none());
        assert_eq!(
            SqliteOrdinaryExecutionClaims::new(pool.clone())
                .claim(&execution)
                .await
                .unwrap(),
            ExecutionClaim::AlreadyHeld
        );
        pool.close().await;
    }
}

pub(super) async fn parameter_gate() {
    for native in [false, true] {
        for mode in [Mode::ParameterRate, Mode::ParameterInvalid] {
            let mut c = Case::new_buy(mode, native).await;
            let result = c.run().await;
            assert_eq!(result.record.unwrap().status, TradeStatus::Failed);
            assert!(result.error.is_some() && result.signature.is_none());
            assert!(c.trace.calls.lock().unwrap().is_empty());
            assert!(c.trace.simulations.lock().unwrap().is_empty());
            assert!(c.trace.sent.lock().unwrap().is_empty());
            let requests = c.server.requests.lock().unwrap().clone();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0]["method"], "getAccountInfo");
            assert_eq!(
                c.journal.claim(&c.f.execution).await.unwrap(),
                ExecutionClaim::AlreadyHeld
            );
            c.unchanged_request().await;
            c.finish().await;
        }
    }
}
