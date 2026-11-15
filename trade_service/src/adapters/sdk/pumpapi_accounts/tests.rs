//! Public messages, synthetic lookup tables and a socket-free RPC. No signer or broadcast.
use super::*;
use serde_json::{json, Value};
use solana_address_lookup_table_interface::state::LookupTableMeta;
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{
        v0::{Message as V0Message, MessageAddressTableLookup},
        Message, VersionedMessage,
    },
};
use std::{
    borrow::Cow,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

async fn simulate_unsigned(
    rpc: &RpcClient,
    wallet: Address,
    transaction: VersionedTransaction,
) -> anyhow::Result<crate::application::trade_execution::TradeExecutionResult> {
    crate::adapters::sdk::pumpapi_simulation::simulate_unsigned(
        rpc,
        wallet,
        transaction,
        &crate::application::trade_command::test_support::command(),
    )
    .await
}

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}
#[derive(Clone)]
struct ReadRpc {
    values: Vec<Value>,
    slot: u64,
    error: Option<String>,
    gate: Option<Arc<Gate>>,
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
}
impl ReadRpc {
    fn new(values: Vec<Value>) -> Self {
        Self {
            values,
            slot: 42,
            error: None,
            gate: None,
            calls: Arc::new(Mutex::new(vec![])),
        }
    }
    fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
    }
    fn methods(&self) -> Vec<RpcRequest> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(method, _)| *method)
            .collect()
    }
}
impl RpcSender for ReadRpc {
    fn send<'life0, 'async_trait>(
        &'life0 self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ClientError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            self.calls.lock().unwrap().push((request, params));
            match request {
                RpcRequest::GetMultipleAccounts => {
                    if let Some(gate) = &self.gate {
                        gate.entered.notify_one();
                        gate.release.notified().await;
                    }
                    if let Some(error) = &self.error {
                        return Err(ClientErrorKind::Custom(error.clone()).into());
                    }
                    Ok(json!({"context":{"slot":self.slot},"value":self.values}))
                }
                RpcRequest::SimulateTransaction => {
                    Ok(json!({"context":{"slot":self.slot},"value":{
                        "err":null,"logs":[],"accounts":null,"unitsConsumed":10,
                        "returnData":null,"innerInstructions":[],"replacementBlockhash":null
                    }}))
                }
                _ => Err(ClientErrorKind::Custom("forbidden account-view RPC".into()).into()),
            }
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats {
            request_count: self.calls.lock().unwrap().len(),
            ..Default::default()
        }
    }
    fn url(&self) -> String {
        "memory://pumpapi-accounts".into()
    }
}

fn owner() -> Address {
    Address::from_bytes([1; 32])
}
fn key(byte: u8) -> Pubkey {
    Pubkey::from([byte; 32])
}
fn transaction(lookups: Option<Vec<MessageAddressTableLookup>>) -> VersionedTransaction {
    let legacy = Message::new_with_blockhash(
        &[Instruction {
            program_id: key(3),
            accounts: vec![
                AccountMeta::new(key(1), true),
                AccountMeta::new(key(2), false),
            ],
            data: vec![7],
        }],
        Some(&key(1)),
        &Hash::new_from_array([8; 32]),
    );
    let message = match lookups {
        None => VersionedMessage::Legacy(legacy),
        Some(address_table_lookups) => VersionedMessage::V0(V0Message {
            header: legacy.header,
            account_keys: legacy.account_keys,
            recent_blockhash: legacy.recent_blockhash,
            instructions: legacy.instructions,
            address_table_lookups,
        }),
    };
    VersionedTransaction {
        signatures: vec![Signature::default()],
        message,
    }
}
fn lookup(table: u8, writable: Vec<u8>, readonly: Vec<u8>) -> MessageAddressTableLookup {
    MessageAddressTableLookup {
        account_key: key(table),
        writable_indexes: writable,
        readonly_indexes: readonly,
    }
}
fn data(meta: LookupTableMeta, addresses: Vec<Pubkey>) -> Vec<u8> {
    AddressLookupTable {
        meta,
        addresses: Cow::Owned(addresses),
    }
    .serialize_for_tests()
    .unwrap()
}
fn ui(data: Vec<u8>) -> Value {
    json!({"owner":program::id().to_string(),"lamports":2_039_280,"executable":false,"rentEpoch":0,
        "space":data.len(),"data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"]})
}
fn table(addresses: Vec<Pubkey>) -> Value {
    ui(data(
        LookupTableMeta {
            last_extended_slot: 41,
            ..Default::default()
        },
        addresses,
    ))
}
fn fixture() -> (ReadRpc, VersionedTransaction) {
    (
        ReadRpc::new(vec![table(vec![key(4), key(5)])]),
        transaction(Some(vec![lookup(9, vec![0], vec![1])])),
    )
}

#[tokio::test]
async fn pumpapi_h2b2a_legacy_and_v0_without_lookups_keep_static_privileges_without_rpc() {
    for lookups in [None, Some(vec![])] {
        let transport = ReadRpc::new(vec![]);
        let tx = transaction(lookups);
        let before = bincode::serialize(&tx).unwrap();
        let view = resolve_unsigned_accounts(&transport.client(), owner(), &tx)
            .await
            .unwrap();
        assert_eq!(view.lookup_slot(), None);
        assert_eq!(
            view.accounts
                .iter()
                .map(|a| (a.address, a.signer, a.writable))
                .collect::<Vec<_>>(),
            vec![
                (owner(), true, true),
                (Address::from_bytes([2; 32]), false, true),
                (Address::from_bytes([3; 32]), false, false)
            ]
        );
        view.ensure_bound(owner(), &tx).unwrap();
        assert_eq!(bincode::serialize(&tx).unwrap(), before);
        assert!(transport.methods().is_empty());
    }
}

#[tokio::test]
async fn pumpapi_h2b2a_two_tables_use_one_read_and_runtime_writable_then_readonly_order() {
    let transport = ReadRpc::new(vec![
        table(vec![key(4), key(5)]),
        table(vec![key(6), key(7)]),
    ]);
    let tx = transaction(Some(vec![
        lookup(9, vec![1], vec![0]),
        lookup(10, vec![0], vec![1]),
    ]));
    let view = resolve_unsigned_accounts(&transport.client(), owner(), &tx)
        .await
        .unwrap();
    assert_eq!(view.lookup_slot(), Some(42));
    assert_eq!(
        view.accounts
            .iter()
            .map(|a| a.address.to_bytes())
            .collect::<Vec<_>>(),
        vec![[1; 32], [2; 32], [3; 32], [5; 32], [6; 32], [4; 32], [7; 32]]
    );
    assert!(view.accounts[3..5].iter().all(|a| a.writable && !a.signer));
    assert!(view.accounts[5..].iter().all(|a| !a.writable && !a.signer));
    assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
    assert_eq!(
        transport.calls.lock().unwrap()[0].1,
        json!([
            [key(9).to_string(),key(10).to_string()],
            {"encoding":"base64","dataSlice":null,"commitment":"processed","minContextSlot":null}
        ])
    );
}

#[tokio::test]
async fn pumpapi_h2b2a_simulation_binds_lookup_slot_and_keeps_original_unsigned_bytes() {
    let (transport, tx) = fixture();
    let before = bincode::serialize(&tx).unwrap();
    let result = simulate_unsigned(&transport.client(), owner(), tx)
        .await
        .unwrap();
    assert!(result.success && result.signature.is_none());
    assert_eq!(
        transport.methods(),
        [
            RpcRequest::GetMultipleAccounts,
            RpcRequest::SimulateTransaction
        ]
    );
    let calls = transport.calls.lock().unwrap();
    assert_eq!(calls[1].1[1]["minContextSlot"], 42);
    assert_eq!(calls[1].1[1]["sigVerify"], false);
    assert_eq!(calls[1].1[1]["replaceRecentBlockhash"], false);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(calls[1].1[0].as_str().unwrap())
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn pumpapi_h2b2a_same_slot_extension_only_exposes_previous_prefix() {
    let mut transport = ReadRpc::new(vec![ui(data(
        LookupTableMeta {
            last_extended_slot: 42,
            last_extended_slot_start_index: 1,
            ..Default::default()
        },
        vec![key(4), key(5)],
    ))]);
    let old = transaction(Some(vec![lookup(9, vec![0], vec![])]));
    assert!(
        resolve_unsigned_accounts(&transport.client(), owner(), &old)
            .await
            .is_ok()
    );
    let new = transaction(Some(vec![lookup(9, vec![0], vec![1])]));
    assert_eq!(
        resolve_unsigned_accounts(&transport.client(), owner(), &new)
            .await
            .err()
            .unwrap()
            .to_string(),
        "PumpAPI lookup index is out of range or not active at RPC context"
    );
    transport.slot = 43;
    assert!(
        resolve_unsigned_accounts(&transport.client(), owner(), &new)
            .await
            .is_ok()
    );
    assert_eq!(
        transport.methods(),
        vec![RpcRequest::GetMultipleAccounts; 3]
    );
}

#[tokio::test]
async fn pumpapi_h2b2a_context_and_prefix_matrix_matches_the_existing_lookup_library() {
    let addresses = vec![key(4), key(5), key(6), key(7)];
    for slot in 0..5 {
        for extended in 0..=slot {
            for prefix in 0..=4 {
                for index in 0..=4 {
                    let meta = LookupTableMeta {
                        last_extended_slot: extended,
                        last_extended_slot_start_index: prefix,
                        ..Default::default()
                    };
                    let table = AddressLookupTable {
                        meta: meta.clone(),
                        addresses: Cow::Borrowed(&addresses),
                    };
                    let expected = table.lookup(slot, &[index], &Default::default());
                    let mut transport = ReadRpc::new(vec![ui(data(meta, addresses.clone()))]);
                    transport.slot = slot;
                    let tx = transaction(Some(vec![lookup(9, vec![index], vec![])]));
                    let actual = resolve_unsigned_accounts(&transport.client(), owner(), &tx).await;
                    match expected {
                        Ok(keys) => {
                            let view = actual.unwrap();
                            assert_eq!(view.accounts[3].address.to_bytes(), keys[0].to_bytes());
                            assert_eq!(view.lookup_slot(), Some(slot));
                            view.ensure_bound(owner(), &tx).unwrap();
                        }
                        Err(_) => assert!(actual.is_err()),
                    }
                    assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
                }
            }
        }
    }
}

#[tokio::test]
async fn pumpapi_h2b2a_inactive_future_or_inconsistent_tables_fail_before_simulation() {
    for (meta, expected) in [
        (
            LookupTableMeta {
                deactivation_slot: 41,
                ..Default::default()
            },
            "PumpAPI lookup table is deactivating or deactivated",
        ),
        (
            LookupTableMeta {
                deactivation_slot: 43,
                ..Default::default()
            },
            "PumpAPI lookup table is deactivating or deactivated",
        ),
        (
            LookupTableMeta {
                last_extended_slot: 43,
                ..Default::default()
            },
            "PumpAPI lookup table extension is newer than RPC context",
        ),
        (
            LookupTableMeta {
                last_extended_slot: 41,
                last_extended_slot_start_index: 3,
                ..Default::default()
            },
            "PumpAPI lookup table extension index is invalid",
        ),
    ] {
        let transport = ReadRpc::new(vec![ui(data(meta, vec![key(4), key(5)]))]);
        let tx = transaction(Some(vec![lookup(9, vec![0], vec![1])]));
        let error = simulate_unsigned(&transport.client(), owner(), tx)
            .await
            .err()
            .unwrap();
        assert_eq!(error.to_string(), expected);
        assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2a_malformed_wrong_owner_and_oversized_lookup_data_fail_closed() {
    for mutation in 0..12 {
        let (mut transport, tx) = fixture();
        match mutation {
            0 => transport.values[0]["owner"] = json!(key(8).to_string()),
            1 => transport.values[0]["executable"] = json!(true),
            2 => transport.values[0]["lamports"] = json!(0),
            3 => transport.values[0] = ui(vec![0; LOOKUP_TABLE_META_SIZE]),
            4 => transport.values[0] = ui(vec![0; LOOKUP_TABLE_META_SIZE - 1]),
            5 => transport.values[0] = ui(vec![0; LOOKUP_TABLE_META_SIZE + 1]),
            6 => transport.values[0]["data"] = json!(["%invalid%", "base64"]),
            7 => transport.values[0]["data"] = json!(["1", "base58"]),
            8 => {
                transport.values[0]["data"] =
                    json!(["A".repeat(MAX_LOOKUP_BASE64_BYTES + 1), "base64"])
            }
            9 => transport.values[0] = ui(vec![0; MAX_LOOKUP_BYTES + 1]),
            10 => transport.values[0]["space"] = json!(999),
            11 => transport.values[0]["data"] = json!(["not-zstd", "base64+zstd"]),
            _ => unreachable!(),
        }
        assert!(
            simulate_unsigned(&transport.client(), owner(), tx)
                .await
                .is_err(),
            "mutation={mutation}"
        );
        assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2a_missing_or_partial_lookup_response_is_not_a_valid_view() {
    for values in [
        vec![],
        vec![Value::Null],
        vec![table(vec![key(4), key(5)]), Value::Null],
    ] {
        let transport = ReadRpc::new(values);
        let tx = fixture().1;
        assert!(resolve_unsigned_accounts(&transport.client(), owner(), &tx)
            .await
            .is_err());
        assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2a_duplicate_static_tables_and_loaded_aliases_are_rejected() {
    for mutation in 0..5 {
        let (mut transport, mut tx) = fixture();
        let VersionedMessage::V0(message) = &mut tx.message else {
            unreachable!()
        };
        match mutation {
            0 => message.account_keys[1] = message.account_keys[0],
            1 => message
                .address_table_lookups
                .push(lookup(9, vec![1], vec![])),
            2 => message.address_table_lookups[0].readonly_indexes = vec![0],
            3 => transport.values[0] = table(vec![key(1), key(5)]),
            4 => transport.values[0] = table(vec![key(4), key(4)]),
            _ => unreachable!(),
        }
        assert!(resolve_unsigned_accounts(&transport.client(), owner(), &tx)
            .await
            .is_err());
        assert_eq!(
            transport.methods(),
            if mutation < 2 {
                vec![]
            } else {
                vec![RpcRequest::GetMultipleAccounts]
            }
        );
    }
}

#[tokio::test]
async fn pumpapi_h2b2a_wrong_payer_nonzero_or_extra_signatures_stop_before_account_reads() {
    for mutation in 0..4 {
        let (transport, mut tx) = fixture();
        let mut wallet = owner();
        match mutation {
            0 => wallet = Address::from_bytes([2; 32]),
            1 => tx.signatures[0] = Signature::from([9; 64]),
            2 => tx.signatures.clear(),
            3 => tx.signatures.push(Signature::default()),
            _ => unreachable!(),
        }
        assert!(resolve_unsigned_accounts(&transport.client(), wallet, &tx)
            .await
            .is_err());
        assert!(transport.methods().is_empty());
    }
}

#[tokio::test]
async fn pumpapi_h2b2a_table_and_instruction_limits_apply_before_account_reads() {
    let transport = ReadRpc::new((0..4).map(|i| table(vec![key(20 + i)])).collect());
    let tx = transaction(Some(
        (0..4).map(|i| lookup(9 + i, vec![0], vec![])).collect(),
    ));
    assert!(resolve_unsigned_accounts(&transport.client(), owner(), &tx)
        .await
        .is_ok());
    for mutation in 0..3 {
        let (transport, mut tx) = fixture();
        let VersionedMessage::V0(message) = &mut tx.message else {
            unreachable!()
        };
        match mutation {
            0 => {
                message.address_table_lookups =
                    (0..5).map(|i| lookup(9 + i, vec![0], vec![])).collect()
            }
            1 => message.instructions.clear(),
            2 => message.instructions = vec![message.instructions[0].clone(); 33],
            _ => unreachable!(),
        }
        assert!(resolve_unsigned_accounts(&transport.client(), owner(), &tx)
            .await
            .is_err());
        assert!(transport.methods().is_empty());
    }
}

#[tokio::test]
async fn pumpapi_h2b2a_full_table_and_256_resolved_accounts_are_bounded() {
    let addresses = (0u16..256)
        .map(|i| {
            let mut bytes = [17; 32];
            bytes[..2].copy_from_slice(&i.to_le_bytes());
            Pubkey::from(bytes)
        })
        .collect();
    let transport = ReadRpc::new(vec![table(addresses)]);
    let tx = transaction(Some(vec![lookup(9, (0..253).collect(), vec![])]));
    let view = resolve_unsigned_accounts(&transport.client(), owner(), &tx)
        .await
        .unwrap();
    assert_eq!(view.accounts.len(), 256);
    assert_eq!(view.lookup_slot(), Some(42));
    assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
}

#[tokio::test]
async fn pumpapi_h2b2a_account_view_cannot_be_reused_after_message_or_wallet_changes() {
    let (transport, tx) = fixture();
    let view = resolve_unsigned_accounts(&transport.client(), owner(), &tx)
        .await
        .unwrap();
    for mutation in 0..6 {
        let mut changed = tx.clone();
        let mut wallet = owner();
        let VersionedMessage::V0(message) = &mut changed.message else {
            unreachable!()
        };
        match mutation {
            0 => message.recent_blockhash = Hash::new_from_array([9; 32]),
            1 => message.instructions[0].data.push(8),
            2 => message.address_table_lookups[0].writable_indexes = vec![1],
            3 => message.account_keys[1] = key(7),
            4 => wallet = Address::from_bytes([2; 32]),
            5 => changed.signatures[0] = Signature::from([7; 64]),
            _ => unreachable!(),
        }
        assert!(view.ensure_bound(wallet, &changed).is_err());
    }
    view.ensure_bound(owner(), &tx).unwrap();
    assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
}

#[tokio::test]
async fn pumpapi_h2b2a_lookup_errors_are_redacted_and_never_retried_or_simulated() {
    let (mut transport, tx) = fixture();
    transport.error = Some("RPC failed https://fixture.invalid/private?token=secret".into());
    let error = resolve_unsigned_accounts(&transport.client(), owner(), &tx)
        .await
        .err()
        .unwrap();
    let error = format!("{error:#}");
    assert!(error.contains("<redacted-url>"));
    assert!(!error.contains("fixture.invalid") && !error.contains("secret"));
    assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
}

#[tokio::test]
async fn pumpapi_h2b2a_lookup_cancellation_does_not_fall_back_or_continue() {
    let (mut transport, tx) = fixture();
    let gate = Arc::new(Gate::default());
    transport.gate = Some(gate.clone());
    let rpc = transport.client();
    let handle = tokio::spawn(async move { resolve_unsigned_accounts(&rpc, owner(), &tx).await });
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    handle.abort();
    assert!(handle.await.err().unwrap().is_cancelled());
    gate.release.notify_one();
    assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
}

#[tokio::test]
async fn pumpapi_h2b2a_lookup_timeout_is_finite_without_a_second_rpc() {
    let (mut transport, tx) = fixture();
    transport.gate = Some(Arc::new(Gate::default()));
    let error = tokio::time::timeout(
        Duration::from_secs(6),
        resolve_unsigned_accounts(&transport.client(), owner(), &tx),
    )
    .await
    .unwrap()
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "PumpAPI lookup accounts RPC timed out");
    assert_eq!(transport.methods(), [RpcRequest::GetMultipleAccounts]);
}
