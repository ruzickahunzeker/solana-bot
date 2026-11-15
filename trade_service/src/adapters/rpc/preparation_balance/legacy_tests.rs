use super::*;
use crate::adapters::sdk::preparation_assets::tests::{mint_bytes, raw_accounts, ui};
use serde_json::{json, Value};
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tokio::sync::Notify;

#[derive(Clone)]
struct MemoryRpc {
    value: Value,
    raw: Value,
    slot: u64,
    raw_slot: u64,
    block: bool,
    delay: bool,
    fail: bool,
    fail_scan: bool,
    fail_raw: bool,
    block_raw: bool,
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    entered: Arc<Notify>,
    dropped: Arc<AtomicBool>,
    raw_entered: Arc<Notify>,
    raw_dropped: Arc<AtomicBool>,
}
struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl MemoryRpc {
    fn new(value: Value) -> Self {
        let raw = if value.as_array().is_some_and(Vec::is_empty) {
            let mut raw = raw_accounts(false, 0);
            raw[1] = Value::Null;
            raw
        } else {
            raw_accounts(false, 1000)
        };
        Self {
            value,
            raw,
            slot: 42,
            raw_slot: 42,
            block: false,
            delay: false,
            fail: false,
            fail_scan: false,
            fail_raw: false,
            block_raw: false,
            calls: Arc::default(),
            entered: Arc::new(Notify::new()),
            dropped: Arc::default(),
            raw_entered: Arc::new(Notify::new()),
            raw_dropped: Arc::default(),
        }
    }
    fn reader(&self) -> RpcPreparationBalanceRead {
        RpcPreparationBalanceRead::new(
            a(1),
            Arc::new(RpcClient::new_sender(
                self.clone(),
                RpcClientConfig::default(),
            )),
        )
    }
}
impl RpcSender for MemoryRpc {
    fn send<'life0, 'async_trait>(
        &'life0 self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, ClientError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            assert!(
                matches!(
                    request,
                    RpcRequest::GetTokenAccountsByOwner | RpcRequest::GetMultipleAccounts
                ),
                "read port must not query bare ATA numbers, execute or sign"
            );
            self.calls.lock().unwrap().push((request, params));
            let _drop = DropFlag(if request == RpcRequest::GetMultipleAccounts {
                self.raw_dropped.clone()
            } else {
                self.dropped.clone()
            });
            self.entered.notify_one();
            if request == RpcRequest::GetMultipleAccounts {
                self.raw_entered.notify_one();
            }
            if self.block || (self.block_raw && request == RpcRequest::GetMultipleAccounts) {
                std::future::pending::<()>().await;
            }
            if self.delay && request == RpcRequest::GetTokenAccountsByOwner {
                tokio::time::sleep(Duration::from_millis(3200)).await;
            }
            if self.fail
                || (self.fail_scan && request == RpcRequest::GetTokenAccountsByOwner)
                || (self.fail_raw && request == RpcRequest::GetMultipleAccounts)
            {
                return Err(ClientErrorKind::Custom(
                    "https://fixture.invalid/?key=fixture-secret".into(),
                )
                .into());
            }
            Ok(if request == RpcRequest::GetTokenAccountsByOwner {
                json!({"context":{"slot":self.slot},"value":self.value})
            } else {
                json!({"context":{"slot":self.raw_slot},"value":self.raw})
            })
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "memory://preparation-balance-test-only".into()
    }
}
fn a(byte: u8) -> Address {
    Address::from_bytes([byte; 32])
}
fn account(token2022: bool, units: u64) -> Value {
    let program = if token2022 {
        spl_token_2022_interface::ID
    } else {
        spl_token_interface::ID
    };
    let wallet = Pubkey::new_from_array(a(1).to_bytes());
    let mint = Pubkey::new_from_array(a(9).to_bytes());
    let key = sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
        &wallet, &mint, &program,
    );
    json!({"pubkey":key.to_string(),"account":{"lamports":1,"owner":program.to_string(),"executable":false,"rentEpoch":0,"space":165,
        "data":{"program":if token2022{"spl-token-2022"}else{"spl-token"},"space":165,"parsed":{"type":"account","info":{"mint":mint.to_string(),"owner":wallet.to_string(),"state":"initialized","tokenAmount":{"amount":units.to_string(),"decimals":6,"uiAmount":null,"uiAmountString":units.to_string()}}}}}})
}
fn fatal(error: &anyhow::Error) -> bool {
    !error
        .downcast_ref::<PreparationBalanceFailure>()
        .unwrap()
        .allows_lane_fallback()
}

#[tokio::test]
async fn preparation_balance_rpc_single_scan_uses_public_keys_and_preserves_classic_token2022_units(
) {
    for token2022 in [false, true] {
        let mut rpc = MemoryRpc::new(json!([account(token2022, 9007199254740993)]));
        rpc.raw = raw_accounts(token2022, 9007199254740993);
        let reader = rpc.reader();
        let snapshot = reader.snapshot(a(9)).await.unwrap();
        assert_eq!(
            snapshot.checked_units(a(1), a(9)).unwrap(),
            9007199254740993
        );
        let input = snapshot.checked_input(a(1), a(9)).unwrap();
        assert_eq!(
            input.account.to_bytes(),
            preparation_assets::input_keys(a(1), a(9))[if token2022 { 2 } else { 1 }].to_bytes()
        );
        assert_eq!(input.decimals, 6);
        let calls = rpc.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].1,
            json!([a(1).to_string(),{"mint":a(9).to_string()},{"encoding":"jsonParsed","dataSlice":null,"commitment":"processed","minContextSlot":null}])
        );
        assert_eq!(calls[1].0, RpcRequest::GetMultipleAccounts);
        assert_eq!(
            calls[1].1,
            json!([
                preparation_assets::input_keys(a(1), a(9)).map(|key| key.to_string()),
                {"encoding":"base64","dataSlice":null,"commitment":"processed","minContextSlot":42}
            ])
        );
    }
}

#[tokio::test]
async fn preparation_balance_rpc_empty_scan_is_zero_but_not_a_spend_authorization() {
    let rpc = MemoryRpc::new(json!([]));
    let snapshot = rpc.reader().snapshot(a(9)).await.unwrap();
    assert_eq!(snapshot.checked_units(a(1), a(9)).unwrap(), 0);
    assert_eq!(rpc.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn preparation_balance_rpc_rejects_wrong_mint_wallet_program_state_and_noncanonical_input() {
    for (pointer, value) in [
        ("/0/account/data/parsed/info/mint", json!(a(99).to_string())),
        (
            "/0/account/data/parsed/info/owner",
            json!(a(99).to_string()),
        ),
        (
            "/0/account/owner",
            json!(spl_token_2022_interface::ID.to_string()),
        ),
        ("/0/account/data/parsed/info/state", json!("frozen")),
        ("/0/pubkey", json!(a(99).to_string())),
        (
            "/0/account/data/parsed/info/tokenAmount/amount",
            json!("18446744073709551616"),
        ),
    ] {
        let mut values = json!([account(false, 1000)]);
        *values.pointer_mut(pointer).unwrap() = value;
        let rpc = MemoryRpc::new(values);
        let error = rpc.reader().snapshot(a(9)).await.err().unwrap();
        assert!(fatal(&error));
        assert_eq!(rpc.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn preparation_balance_rpc_duplicate_or_partial_scan_cannot_hide_behind_raw_fallback() {
    let mut raw = account(false, 1000);
    raw["account"]["data"] = json!(["", "base64"]);
    for values in [
        json!([account(false, 1000), raw]),
        json!([account(false, 1000), account(false, 1000)]),
    ] {
        let rpc = MemoryRpc::new(values);
        assert!(fatal(&rpc.reader().snapshot(a(9)).await.err().unwrap()));
        assert_eq!(rpc.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn preparation_balance_rpc_invalid_response_or_slot_is_fatal_not_provider_retry() {
    for (values, slot) in [(json!([null]), 42), (json!([account(false, 1000)]), 0)] {
        let mut rpc = MemoryRpc::new(values);
        rpc.slot = slot;
        assert!(fatal(&rpc.reader().snapshot(a(9)).await.err().unwrap()));
    }
}

#[tokio::test]
async fn preparation_balance_rpc_account_limit_is_checked_after_successful_sdk_decode() {
    let rpc = MemoryRpc::new(Value::Array(vec![account(false, 1000); 1001]));
    let error = rpc.reader().snapshot(a(9)).await.err().unwrap();
    assert!(fatal(&error));
    assert!(error.to_string().contains("exceeds account limit"));
    assert_eq!(rpc.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn preparation_balance_rpc_zero_auxiliary_account_preserves_units_but_not_mixed_metadata() {
    let mut auxiliary = account(false, 0);
    auxiliary["pubkey"] = json!(a(99).to_string());
    let rpc = MemoryRpc::new(json!([account(false, 1000), auxiliary.clone()]));
    let snapshot = rpc.reader().snapshot(a(9)).await.unwrap();
    assert_eq!(snapshot.checked_units(a(1), a(9)).unwrap(), 1000);
    assert_eq!(rpc.calls.lock().unwrap().len(), 2);

    let mut other_precision = auxiliary;
    other_precision["account"]["data"]["parsed"]["info"]["tokenAmount"]["decimals"] = json!(9);
    for other in [other_precision, account(true, 0)] {
        let rpc = MemoryRpc::new(json!([account(false, 1000), other]));
        assert!(fatal(&rpc.reader().snapshot(a(9)).await.err().unwrap()));
        assert_eq!(rpc.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn preparation_balance_rpc_transport_error_is_redacted_and_only_preparation_retryable() {
    let mut rpc = MemoryRpc::new(json!([]));
    rpc.fail = true;
    let error = rpc.reader().snapshot(a(9)).await.err().unwrap();
    assert!(!fatal(&error));
    assert!(!error.to_string().contains("fixture-secret"));
    assert_eq!(rpc.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn preparation_balance_rpc_unavailable_or_all_unparsed_scan_uses_verified_raw_accounts_only()
{
    for token2022 in [false, true] {
        for unavailable in [false, true] {
            let mut unparsed = account(token2022, 1000);
            unparsed["account"]["data"] = json!(["", "base64"]);
            let mut rpc = MemoryRpc::new(json!([unparsed]));
            rpc.fail_scan = unavailable;
            rpc.raw = raw_accounts(token2022, 1000);
            let snapshot = rpc.reader().snapshot(a(9)).await.unwrap();
            assert_eq!(snapshot.checked_units(a(1), a(9)).unwrap(), 1000);
            let calls = rpc.calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[1].0, RpcRequest::GetMultipleAccounts);
            assert_eq!(
                calls[1].1[1]["minContextSlot"],
                if unavailable { Value::Null } else { json!(42) }
            );
        }
    }
}

#[tokio::test]
async fn preparation_balance_rpc_scan_raw_amount_precision_or_program_mismatch_is_fatal() {
    for raw in [raw_accounts(false, 999), raw_accounts(true, 1000), {
        let mut raw = raw_accounts(false, 1000);
        raw[0] = ui(spl_token_interface::ID, &mint_bytes(9));
        raw
    }] {
        let mut rpc = MemoryRpc::new(json!([account(false, 1000)]));
        rpc.raw = raw;
        let error = rpc.reader().snapshot(a(9)).await.err().unwrap();
        assert!(fatal(&error));
        assert!(error.to_string().contains("disagree"));
        assert_eq!(rpc.calls.lock().unwrap().len(), 2);
    }
    let mut rpc = MemoryRpc::new(json!([]));
    rpc.raw = raw_accounts(false, 1000);
    assert!(
        fatal(&rpc.reader().snapshot(a(9)).await.err().unwrap()),
        "empty scan cannot hide positive raw input"
    );
}

#[tokio::test]
async fn preparation_balance_rpc_raw_context_and_shape_are_fatal_not_another_fallback() {
    for (slot, raw) in [
        (0, raw_accounts(false, 1000)),
        (41, raw_accounts(false, 1000)),
        (42, json!([])),
        (42, json!([null, null, null])),
        (42, json!([{}, null, null])),
    ] {
        let mut rpc = MemoryRpc::new(json!([account(false, 1000)]));
        rpc.raw_slot = slot;
        rpc.raw = raw;
        assert!(fatal(&rpc.reader().snapshot(a(9)).await.err().unwrap()));
        assert_eq!(rpc.calls.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn preparation_balance_rpc_raw_unavailable_remains_pre_entry_retryable_and_redacted() {
    let mut rpc = MemoryRpc::new(json!([account(false, 1000)]));
    rpc.fail_raw = true;
    let error = rpc.reader().snapshot(a(9)).await.err().unwrap();
    assert!(!fatal(&error));
    assert!(!format!("{error:?}").contains("fixture-secret"));
    assert_eq!(rpc.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn preparation_balance_rpc_cancel_raw_read_drops_it_without_next_rpc_or_signer() {
    let mut rpc = MemoryRpc::new(json!([account(false, 1000)]));
    rpc.block_raw = true;
    let reader = rpc.reader();
    let task = tokio::spawn(async move { reader.snapshot(a(9)).await });
    tokio::time::timeout(Duration::from_secs(1), rpc.raw_entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(rpc.raw_dropped.load(Ordering::SeqCst));
    assert_eq!(rpc.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn preparation_balance_rpc_scan_and_raw_share_total_deadline_without_timestamp_renewal() {
    let mut rpc = MemoryRpc::new(json!([account(false, 1000)]));
    rpc.delay = true;
    rpc.block_raw = true;
    let started = Instant::now();
    assert!(rpc.reader().snapshot(a(9)).await.is_err());
    assert!(started.elapsed() >= Duration::from_millis(3900));
    assert!(started.elapsed() < Duration::from_secs(6));
    assert!(rpc.raw_dropped.load(Ordering::SeqCst));
    assert_eq!(rpc.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn preparation_balance_rpc_slow_result_does_not_receive_a_new_fresh_timestamp() {
    let mut rpc = MemoryRpc::new(json!([account(false, 1000)]));
    rpc.delay = true;
    let error = rpc.reader().snapshot(a(9)).await.err().unwrap();
    assert!(fatal(&error));
    assert!(error.to_string().contains("expired"));
}

#[tokio::test]
async fn preparation_balance_rpc_cancel_drops_query_without_ata_or_background_work() {
    let mut rpc = MemoryRpc::new(json!([]));
    rpc.block = true;
    let reader = rpc.reader();
    let task = tokio::spawn(async move { reader.snapshot(a(9)).await });
    tokio::time::timeout(Duration::from_secs(1), rpc.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(rpc.dropped.load(Ordering::SeqCst));
    assert_eq!(rpc.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn preparation_balance_rpc_deadline_cancels_sdk_read_and_does_not_claim_wire_budget() {
    let mut rpc = MemoryRpc::new(json!([]));
    rpc.block = true;
    let started = Instant::now();
    assert!(rpc.reader().snapshot(a(9)).await.is_err());
    assert!(started.elapsed() >= Duration::from_millis(3900));
    assert!(rpc.dropped.load(Ordering::SeqCst));
    assert_eq!(rpc.calls.lock().unwrap().len(), 1);
}
