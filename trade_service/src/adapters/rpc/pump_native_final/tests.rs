use super::*;
use crate::adapters::rpc::{
    read_http::{HttpReply, ReadHttpTransport, ReadRequest},
    read_runtime::SharedReadRuntime,
};
use reqwest::{StatusCode, Url};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::{v0, Message},
    transaction::Transaction,
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

mod wire;

pub(crate) fn fixture(buy: bool, tip: bool) -> (VersionedTransaction, Vec<Pubkey>) {
    let wallet = Pubkey::new_from_array([1; 32]);
    let mut data = if buy {
        BUY_EXACT_SOL_IN_DISCRIMINATOR.to_vec()
    } else {
        SELL_DISCRIMINATOR.to_vec()
    };
    data.extend(1_000_000u64.to_le_bytes());
    data.extend(1_234u64.to_le_bytes());
    if buy {
        data.push(1);
    }
    let swap = Instruction {
        program_id: accounts::PUMPFUN,
        accounts: vec![AccountMeta::new(wallet, true)],
        data,
    };
    let mut keys: Vec<_> = (20..31)
        .map(|value| Pubkey::new_from_array([value; 32]))
        .collect();
    let mut instructions = vec![];
    if tip {
        let destination = Pubkey::new_from_array([31; 32]);
        instructions.push(solana_system_interface::instruction::transfer(
            &wallet,
            &destination,
            100_000,
        ));
        keys.push(destination);
    }
    sol_trade_sdk::trading::common::extend_compute_budget_instructions(
        &mut instructions,
        1_000,
        300_000,
    );
    instructions.push(swap);
    (
        VersionedTransaction::from(Transaction::new_unsigned(Message::new_with_blockhash(
            &instructions,
            Some(&wallet),
            &Hash::new_from_array([10; 32]),
        ))),
        keys,
    )
}

fn packet(tx: &VersionedTransaction, keys: &[Pubkey]) -> PumpNativeFinalPacket {
    PumpNativeFinalPacket::new(tx, keys, 176, Instant::now() + Duration::from_secs(3)).unwrap()
}

#[test]
fn pump_final_read_freezes_unsigned_complete_messages_and_eleven_or_twelve_roles() {
    for buy in [false, true] {
        for tip in [false, true] {
            let (mut tx, mut keys) = fixture(buy, tip);
            let frozen = bincode::serialize(&tx).unwrap();
            let plan = packet(&tx, &keys);
            let body = plan.body(17);
            assert_eq!(
                body,
                json!({"jsonrpc":"2.0","id":17,"method":"simulateTransaction","params":[
                base64::engine::general_purpose::STANDARD.encode(&frozen),
                {"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64",
                 "accounts":{"encoding":"base64","addresses":keys.iter().map(ToString::to_string).collect::<Vec<_>>()},
                 "minContextSlot":176,"innerInstructions":true}]})
            );
            assert_eq!(plan.account_count(), if tip { 12 } else { 11 });
            let VersionedMessage::Legacy(message) = &mut tx.message else {
                unreachable!()
            };
            message.recent_blockhash = Hash::new_from_array([11; 32]);
            keys.swap(0, 1);
            assert_eq!(plan.body(17), body);
            let reply = plan.binding().complete(json!({"opaque":true})).unwrap();
            assert!(reply.for_template(&tx, &keys, 176).is_err());
        }
    }
}

#[test]
fn pump_final_read_refuses_signed_or_malformed_envelopes_budgets_swaps_tips_and_context() {
    for damage in 0..30 {
        let (mut tx, mut keys) = fixture(true, true);
        let mut minimum = 176;
        let mut until = Instant::now() + Duration::from_secs(3);
        let VersionedMessage::Legacy(message) = &mut tx.message else {
            unreachable!()
        };
        let swap = message.instructions.len() - 1;
        match damage {
            0 => minimum = 0,
            1 => until = Instant::now(),
            2 => until = Instant::now() + Duration::from_secs(10),
            3 => tx.signatures[0] = Signature::from([1; 64]),
            4 => tx.signatures.push(Signature::default()),
            5 => message.header.num_required_signatures = 2,
            6 => message.instructions[swap].program_id_index = 255,
            7 => message.recent_blockhash = Hash::default(),
            8 => {
                let index = usize::from(message.instructions[swap].program_id_index);
                message.account_keys[index] = solana_system_interface::program::ID;
            }
            9 => message.instructions[1].data[1..].copy_from_slice(&999u64.to_le_bytes()),
            10 => message.instructions[2].data[1..].copy_from_slice(&299_999u32.to_le_bytes()),
            11 => message
                .instructions
                .push(message.instructions[swap].clone()),
            12 => {
                message.instructions.drain(..3);
            }
            13 => message.instructions[swap].data[8..16].copy_from_slice(&0u64.to_le_bytes()),
            14 => message.instructions[swap].data[16..24].copy_from_slice(&0u64.to_le_bytes()),
            15 => message.instructions[swap].data[24] = 2,
            16 => message.instructions[swap].data.truncate(16),
            17 => message.instructions[swap].data.resize(2_048, 0),
            18 => keys[1] = keys[0],
            19 => keys[0] = Pubkey::default(),
            20 => {
                keys.pop();
            }
            21 => *keys.last_mut().unwrap() = Pubkey::new_from_array([32; 32]),
            22 => message.instructions[0].data[0] = 1,
            23 => {
                message.instructions[0].accounts[1] = 0;
                *keys.last_mut().unwrap() = message.account_keys[0];
            }
            24 => message.instructions[0].accounts[0] = 1,
            25 => message.instructions[swap].data[0] ^= 1,
            26 => {
                message.instructions[1].program_id_index =
                    message.instructions[swap].program_id_index
            }
            27 => message.instructions[1].accounts.push(0),
            28 => message.header.num_readonly_signed_accounts = 1,
            _ => {
                tx.message = VersionedMessage::V0(v0::Message {
                    header: message.header,
                    account_keys: message.account_keys.clone(),
                    recent_blockhash: message.recent_blockhash,
                    instructions: message.instructions.clone(),
                    address_table_lookups: vec![],
                })
            }
        }
        assert!(
            PumpNativeFinalPacket::new(&tx, &keys, minimum, until).is_err(),
            "damage {damage}"
        );
    }
    let (tx, keys) = super::super::pump_native_probe::tests::fixture();
    assert!(PumpNativeFinalPacket::new(
        &VersionedTransaction::from(tx),
        &keys,
        176,
        Instant::now() + Duration::from_secs(3)
    )
    .is_err());
}

struct Malformed {
    damage: usize,
    calls: AtomicUsize,
}
impl ReadHttpTransport for Malformed {
    async fn fetch(&self, _: &Url, request: &ReadRequest) -> Result<HttpReply> {
        assert!(matches!(request, ReadRequest::PumpNativeFinal { .. }));
        self.calls.fetch_add(1, Ordering::SeqCst);
        let body = request.test_body();
        let count = body["params"][1]["accounts"]["addresses"]
            .as_array()
            .unwrap()
            .len();
        let mut result = json!({"jsonrpc":"2.0","id":body["id"],"result":{"context":{"slot":176},"value":{"err":null,"accounts":vec![Value::Null;count]}}});
        match self.damage {
            0 => result["id"] = json!(999),
            1 => result["jsonrpc"] = json!("1.0"),
            2 => {
                result["result"]["value"]
                    .as_object_mut()
                    .unwrap()
                    .remove("err");
            }
            3 => result["result"]["value"]["err"] = json!("failed"),
            4 => result["result"]["context"]["slot"] = json!(175),
            5 => result["result"]["value"]["accounts"] = json!([]),
            6 => {
                result.as_object_mut().unwrap().remove("result");
            }
            _ => result["error"] = json!({"code":-32005,"message":"fixture-secret"}),
        }
        Ok(HttpReply::for_test(
            StatusCode::OK,
            serde_json::to_vec(&result)?,
        ))
    }
}

#[tokio::test]
async fn pump_final_read_requires_matching_rpc_outcome_context_and_returned_role_count() {
    for damage in 0..8 {
        let runtime = SharedReadRuntime::with_test_transport(
            &["http://127.0.0.1:12345".into()],
            Malformed {
                damage,
                calls: AtomicUsize::new(0),
            },
        );
        let (tx, keys) = fixture(true, true);
        let error = runtime
            .pump_native_final("http://127.0.0.1:12345", packet(&tx, &keys))
            .await
            .err()
            .expect("bad envelope accepted");
        assert!(!format!("{error:#}").contains("fixture-secret"));
        assert_eq!(runtime.test_transport().calls.load(Ordering::SeqCst), 1);
        assert!(runtime
            .aggregator_simulation_slot("http://127.0.0.1:12345")
            .is_ok());
    }
}

#[tokio::test]
async fn pump_final_read_uses_original_simulation_admission_and_rejects_unknown_endpoints_before_io(
) {
    let url = "http://127.0.0.1:12345";
    let runtime = SharedReadRuntime::with_test_transport(
        &[url.into()],
        Malformed {
            damage: 0,
            calls: AtomicUsize::new(0),
        },
    );
    let mut held = vec![];
    while let Ok(permit) = runtime.aggregator_simulation_slot(url) {
        held.push(permit);
        assert!(held.len() < 100);
    }
    let (tx, keys) = fixture(true, true);
    assert!(runtime
        .pump_native_final(url, packet(&tx, &keys))
        .await
        .is_err());
    assert_eq!(runtime.test_transport().calls.load(Ordering::SeqCst), 0);
    drop(held);
    assert!(runtime
        .pump_native_final("http://127.0.0.1:12346", packet(&tx, &keys))
        .await
        .is_err());
    assert_eq!(runtime.test_transport().calls.load(Ordering::SeqCst), 0);
    assert!(runtime.aggregator_simulation_slot(url).is_ok());
}
