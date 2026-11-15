use super::*;
use crate::adapters::sdk::damm_probe::{
    evidence::{
        inspect_prepared_damm_probe,
        tests::{
            bytes, change_token, clock, extended_mint, extended_token, key, token, ui, Fixture,
        },
    },
    plan::PreparedDammProbe,
    prepare_damm_probe,
};
use base64::Engine;
use serde_json::{json, Value};
use sol_trade_sdk::constants::{USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT};
use solana_client::rpc_request::RpcRequest;
use solana_program_pack::Pack;
use solana_sdk::{
    hash::Hash, instruction::Instruction, signature::Signature, transaction::Transaction,
};
use spl_token_interface::state::Account as TokenAccount;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy)]
enum Encoding {
    Compiled,
    Partial,
    Parsed,
}
#[derive(Clone, Copy)]
enum Kind {
    Checked,
    Legacy,
    Fee,
}

struct Case {
    source: Fixture,
    prepared: PreparedDammProbe,
    reply: Value,
    before: [u64; 4],
    output: u64,
}
fn account_amount(value: &Value) -> u64 {
    TokenAccount::unpack(&bytes(value)[..TokenAccount::LEN])
        .unwrap()
        .amount
}
fn account_index(prepared: &PreparedDammProbe, key: Pubkey) -> u8 {
    u8::try_from(
        prepared
            .transaction()
            .message
            .account_keys
            .iter()
            .position(|a| *a == key)
            .unwrap(),
    )
    .unwrap()
}
fn encode(
    ix: &Instruction,
    prepared: &PreparedDammProbe,
    encoding: Encoding,
    parsed: Value,
) -> Value {
    match encoding {
        Encoding::Compiled => json!({"programIdIndex":account_index(prepared,ix.program_id),
            "accounts":ix.accounts.iter().map(|a|account_index(prepared,a.pubkey)).collect::<Vec<_>>(),"data":bs58::encode(&ix.data).into_string(),"stackHeight":2}),
        Encoding::Partial => {
            json!({"programId":ix.program_id.to_string(),"accounts":ix.accounts.iter().map(|a|a.pubkey.to_string()).collect::<Vec<_>>(),"data":bs58::encode(&ix.data).into_string(),"stackHeight":2})
        }
        Encoding::Parsed => {
            json!({"programId":ix.program_id.to_string(),"program":"spl-token","parsed":parsed,"stackHeight":2})
        }
    }
}
fn movement(
    prepared: &PreparedDammProbe,
    input: bool,
    amount: u64,
    fee: u64,
    encoding: Encoding,
    kind: Kind,
) -> Value {
    let p = prepared.accounts();
    let a = p.input_mint == p.pool.token_a_mint;
    let (program, mint, from, to, authority) = if input {
        (
            if a {
                p.pool.token_a_program
            } else {
                p.pool.token_b_program
            },
            p.input_mint,
            p.input_ata,
            if a {
                p.pool.token_a_vault
            } else {
                p.pool.token_b_vault
            },
            p.wallet,
        )
    } else {
        (
            if a {
                p.pool.token_b_program
            } else {
                p.pool.token_a_program
            },
            p.output_mint,
            if a {
                p.pool.token_b_vault
            } else {
                p.pool.token_a_vault
            },
            p.output_ata,
            accounts::AUTHORITY,
        )
    };
    let decimals = if mint == WSOL_TOKEN_ACCOUNT { 9 } else { 6 };
    let ix=match kind {
        Kind::Checked=>spl_token_2022_interface::instruction::transfer_checked(&program,&from,&mint,&to,&authority,&[],amount,decimals).unwrap(),
        Kind::Legacy=>{
            let mut ix=spl_token_interface::instruction::transfer(&spl_token_interface::ID,&from,&to,&authority,&[],amount).unwrap();
            ix.program_id=program;ix
        },
        Kind::Fee=>spl_token_2022_interface::extension::transfer_fee::instruction::transfer_checked_with_fee(&program,&from,&mint,&to,&authority,&[],amount,decimals,fee).unwrap(),
    };
    let mut info = json!({"source":from.to_string(),"destination":to.to_string(),"authority":authority.to_string()});
    let name = match kind {
        Kind::Legacy => {
            info["amount"] = json!(amount.to_string());
            "transfer"
        }
        Kind::Checked | Kind::Fee => {
            info["mint"] = json!(mint.to_string());
            info["tokenAmount"] = json!({"amount":amount.to_string(),"decimals":decimals,"uiAmount":null,"uiAmountString":"not used"});
            if matches!(kind, Kind::Fee) {
                info["feeAmount"] = json!({"amount":fee.to_string(),"decimals":decimals,"uiAmount":null,"uiAmountString":"not used"});
            }
            if matches!(kind, Kind::Fee) {
                "transferCheckedWithFee"
            } else {
                "transferChecked"
            }
        }
    };
    encode(&ix, prepared, encoding, json!({"type":name,"info":info}))
}
fn initialize(prepared: &PreparedDammProbe, encoding: Encoding) -> Vec<Value> {
    let p = prepared.accounts();
    let program = if p.output_mint == p.pool.token_a_mint {
        p.pool.token_a_program
    } else {
        p.pool.token_b_program
    };
    let size =
        spl_token_2022_interface::instruction::get_account_data_size(&program, &p.output_mint, &[])
            .unwrap();
    let immutable =
        spl_token_2022_interface::instruction::initialize_immutable_owner(&program, &p.output_ata)
            .unwrap();
    let init = spl_token_2022_interface::instruction::initialize_account3(
        &program,
        &p.output_ata,
        &p.output_mint,
        &p.wallet,
    )
    .unwrap();
    vec![
        encode(
            &size,
            prepared,
            encoding,
            json!({"type":"getAccountDataSize","info":{"mint":p.output_mint.to_string(),"extensionTypes":[]}}),
        ),
        encode(
            &immutable,
            prepared,
            encoding,
            json!({"type":"initializeImmutableOwner","info":{"account":p.output_ata.to_string()}}),
        ),
        encode(
            &init,
            prepared,
            encoding,
            json!({"type":"initializeAccount3","info":{"account":p.output_ata.to_string(),"mint":p.output_mint.to_string(),"owner":p.wallet.to_string()}}),
        ),
    ]
}
impl Case {
    fn new(
        reversed: bool,
        token2022: bool,
        sell_target: bool,
        exists: bool,
        quote: Pubkey,
        encoding: Encoding,
    ) -> Self {
        let mut source = Fixture::new(reversed, token2022, quote, exists);
        let target_program = if token2022 {
            spl_token_2022_interface::ID
        } else {
            spl_token_interface::ID
        };
        if token2022 {
            let (mi, vi) = if reversed { (2, 4) } else { (1, 3) };
            source.rpc.values[mi] = ui(target_program, extended_mint(false, false, 300));
            source.rpc.values[vi] = ui(
                target_program,
                extended_token(key(4), accounts::AUTHORITY, false, false),
            );
        }
        let (input, output, input_program, output_program) = if sell_target {
            (key(4), quote, target_program, spl_token_interface::ID)
        } else {
            (quote, key(4), spl_token_interface::ID, target_program)
        };
        source.rpc.values[6] = if token2022 && sell_target {
            ui(target_program, extended_token(input, key(5), false, false))
        } else {
            token(input_program, input, key(5), 1000)
        };
        source.rpc.values[7] = if !exists {
            Value::Null
        } else if token2022 && !sell_target {
            ui(target_program, extended_token(output, key(5), false, false))
        } else {
            token(output_program, output, key(5), 2000)
        };
        let prepared = prepare_damm_probe(
            key(5),
            &source.params,
            input,
            output,
            100,
            Hash::new_from_array([6; 32]),
            false,
        )
        .unwrap();
        let a = input == source.params.token_a_mint;
        let (iv, ov) = if a { (3, 4) } else { (4, 3) };
        let before = [
            1000,
            if exists {
                account_amount(&source.rpc.values[7])
            } else {
                0
            },
            account_amount(&source.rpc.values[iv]),
            account_amount(&source.rpc.values[ov]),
        ];
        let input_fee = if token2022 && sell_target { 1 } else { 0 };
        let output_fee = if token2022 && !sell_target { 2 } else { 0 };
        let net = 120 - output_fee;
        let mut post = source.rpc.values.clone();
        if !exists {
            post[7] = if token2022 && !sell_target {
                ui(target_program, extended_token(output, key(5), false, false))
            } else {
                token(output_program, output, key(5), 0)
            };
        }
        change_token(&mut post[6], |a| a.amount = before[0] - 100);
        change_token(&mut post[7], |a| a.amount = before[1] + net);
        change_token(&mut post[iv], |a| a.amount = before[2] + 100 - input_fee);
        change_token(&mut post[ov], |a| a.amount = before[3] - 120);
        post[5] = clock(43, 17);
        source.rpc.simulation_slot = 43;
        let mut groups = vec![];
        if !exists {
            groups.push(json!({"index":0,"instructions":initialize(&prepared,encoding)}));
        }
        groups.push(json!({"index":1,"instructions":[movement(&prepared,true,100,input_fee,encoding,Kind::Checked),movement(&prepared,false,120,output_fee,encoding,Kind::Checked)]}));
        let reply = json!({"err":null,"logs":[],"accounts":post,"unitsConsumed":123,"innerInstructions":groups});
        Self {
            source,
            prepared,
            reply,
            before,
            output: net,
        }
    }
    fn default() -> Self {
        Self::new(
            false,
            false,
            true,
            true,
            USDC_TOKEN_ACCOUNT,
            Encoding::Compiled,
        )
    }
    fn swap(&mut self) -> &mut Vec<Value> {
        self.reply["innerInstructions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|g| g["index"] == 1)
            .unwrap()["instructions"]
            .as_array_mut()
            .unwrap()
    }
    async fn run(&self) -> Result<u64> {
        let mut memory = self.source.rpc.clone();
        memory.simulation_value = Some(self.reply.clone());
        let rpc = memory.client();
        let evidence = inspect_prepared_damm_probe(&rpc, &self.prepared).await?;
        quote_evidenced_damm_probe(&evidence).await
    }
    fn rows(&self, before: bool) -> Value {
        let p = self.prepared.accounts();
        let a = p.input_mint == p.pool.token_a_mint;
        let (ip, op, iv, ov, ivi, ovi) = if a {
            (
                p.pool.token_a_program,
                p.pool.token_b_program,
                p.pool.token_a_vault,
                p.pool.token_b_vault,
                3,
                4,
            )
        } else {
            (
                p.pool.token_b_program,
                p.pool.token_a_program,
                p.pool.token_b_vault,
                p.pool.token_a_vault,
                4,
                3,
            )
        };
        let roles = [
            (p.input_ata, p.input_mint, ip, p.wallet, 6),
            (p.output_ata, p.output_mint, op, p.wallet, 7),
            (iv, p.input_mint, ip, accounts::AUTHORITY, ivi),
            (ov, p.output_mint, op, accounts::AUTHORITY, ovi),
        ];
        let mut rows = vec![];
        for (i, (key, mint, program, owner, post_index)) in roles.into_iter().enumerate() {
            if before && i == 1 && self.source.rpc.values[7].is_null() {
                continue;
            }
            let amount = if before {
                self.before[i]
            } else {
                account_amount(&self.reply["accounts"][post_index])
            };
            rows.push(json!({"accountIndex":account_index(&self.prepared,key),"mint":mint.to_string(),"owner":owner.to_string(),"programId":program.to_string(),
                "uiTokenAmount":{"amount":amount.to_string(),"decimals":if mint==WSOL_TOKEN_ACCOUNT {9}else{6},"uiAmount":null,"uiAmountString":"ignored"}}));
        }
        json!(rows)
    }
    fn add_balances(&mut self) {
        self.reply["preTokenBalances"] = self.rows(true);
        self.reply["postTokenBalances"] = self.rows(false);
    }
}

#[tokio::test]
async fn real_rpc_wire_proves_both_directions_programs_quotes_encodings_and_new_atas() {
    for reversed in [false, true] {
        for t2022 in [false, true] {
            for sell in [false, true] {
                for exists in [false, true] {
                    for quote in [USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT] {
                        for encoding in [Encoding::Compiled, Encoding::Partial, Encoding::Parsed] {
                            let mut c = Case::new(reversed, t2022, sell, exists, quote, encoding);
                            c.add_balances();
                            assert_eq!(c.run().await.unwrap(), c.output);
                            let calls = c.source.rpc.calls.lock().unwrap();
                            assert_eq!(calls.len(), 2);
                            assert_eq!(calls[0].0, RpcRequest::GetMultipleAccounts);
                            assert_eq!(calls[1].0, RpcRequest::SimulateTransaction);
                            let options = &calls[1].1[1];
                            assert_eq!(options["innerInstructions"], true);
                            assert_eq!(options["minContextSlot"], 42);
                            assert_eq!(options["sigVerify"], false);
                            assert_eq!(options["replaceRecentBlockhash"], false);
                            assert_eq!(options["commitment"], "processed");
                            assert_eq!(options["accounts"]["addresses"], calls[0].1[0]);
                            assert_eq!(options["accounts"]["encoding"], "base64");
                            let data = base64::engine::general_purpose::STANDARD
                                .decode(calls[1].1[0].as_str().unwrap())
                                .unwrap();
                            let actual: Transaction = bincode::deserialize(&data).unwrap();
                            assert_eq!(&actual, c.prepared.transaction());
                            assert_eq!(actual.signatures, [Signature::default()]);
                        }
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn cross_slot_external_deposits_and_withdrawals_do_not_change_this_swap_output() {
    for baseline in [0, 10, 2000, 1_000_000] {
        let mut c = Case::default();
        change_token(&mut c.source.rpc.values[7], |a| a.amount = baseline);
        c.before[1] = 1000;
        change_token(&mut c.reply["accounts"][7], |a| a.amount = 1120);
        c.add_balances();
        assert_eq!(c.run().await.unwrap(), 120);
    }
}

#[tokio::test]
async fn complete_trace_is_required_and_old_balance_subtraction_is_never_a_fallback() {
    for damage in 0..6 {
        let mut c = Case::default();
        match damage {
            0 => {
                c.reply.as_object_mut().unwrap().remove("innerInstructions");
            }
            1 => c.reply["innerInstructions"] = json!([]),
            2 => {
                c.swap().remove(0);
            }
            3 => {
                c.swap().remove(1);
            }
            4 => {
                let ix = c.swap()[0].clone();
                c.swap().push(ix);
            }
            _ => {
                let ix = c.swap()[1].clone();
                c.swap().push(ix);
            }
        }
        assert!(c.run().await.is_err(), "damage={damage}");
        assert_eq!(
            c.source.rpc.methods(),
            [
                RpcRequest::GetMultipleAccounts,
                RpcRequest::SimulateTransaction
            ]
        );
    }
}

#[tokio::test]
async fn simulation_failure_context_replacement_and_account_shape_fail_closed() {
    for damage in 0..10 {
        let mut c = Case::default();
        match damage {
            0 => c.reply["err"] = json!({"InstructionError":[1,{"Custom":42}]}),
            1 => c.source.rpc.simulation_slot = 41,
            2 => c.reply["accounts"][5] = clock(44, 17),
            3 => {
                c.reply["accounts"].as_array_mut().unwrap().pop();
            }
            4 => c.reply["accounts"][7] = Value::Null,
            5 => c.reply["accounts"] = Value::Null,
            6 => {
                c.reply["replacementBlockhash"] =
                    json!({"blockhash":Hash::default().to_string(),"lastValidBlockHeight":1})
            }
            7 => {
                c.reply["loadedAddresses"] = json!({"writable":[key(99).to_string()],"readonly":[]})
            }
            8 => c.reply["accounts"][0]["owner"] = json!(key(99).to_string()),
            _ => change_token(&mut c.reply["accounts"][7], |a| a.owner = key(99)),
        }
        assert!(c.run().await.is_err(), "damage={damage}");
    }
}

#[tokio::test]
async fn amount_authority_program_mint_decimals_and_extra_signers_cannot_drift() {
    for damage in 0..9 {
        let mut c = Case::new(
            false,
            false,
            true,
            true,
            USDC_TOKEN_ACCOUNT,
            Encoding::Parsed,
        );
        match damage {
            0 => c.swap()[0]["parsed"]["info"]["tokenAmount"]["amount"] = json!("99"),
            1 => c.swap()[1]["parsed"]["info"]["authority"] = json!(key(5).to_string()),
            2 => c.swap()[1]["parsed"]["info"]["mint"] = json!(key(4).to_string()),
            3 => c.swap()[1]["parsed"]["info"]["tokenAmount"]["decimals"] = json!(9),
            4 => {
                c.swap()[1]["parsed"]["info"]["destination"] =
                    json!(c.prepared.accounts().input_ata.to_string())
            }
            5 => c.swap()[1]["parsed"]["info"]["signers"] = json!([key(5).to_string()]),
            6 => c.swap()[1]["parsed"]["info"]["tokenAmount"]["amount"] = json!("0"),
            7 => c.swap()[1]["parsed"]["info"]["tokenAmount"]["amount"] = json!("1e6"),
            _ => {
                c.swap()[1]["parsed"]["info"]["tokenAmount"]["amount"] =
                    json!("18446744073709551616")
            }
        }
        assert!(c.run().await.is_err(), "damage={damage}");
    }
}

#[tokio::test]
async fn inner_indexes_groups_and_encoded_limits_are_bounded() {
    for damage in 0..10 {
        let mut c = Case::default();
        match damage {
            0 => c.swap()[0]["programIdIndex"] = json!(255),
            1 => c.swap()[0]["accounts"][0] = json!(255),
            2 => c.reply["innerInstructions"][0]["index"] = json!(2),
            3 => {
                let g = c.reply["innerInstructions"][0].clone();
                c.reply["innerInstructions"].as_array_mut().unwrap().push(g);
            }
            4 => c.swap()[0]["data"] = json!("!"),
            5 => c.swap()[0]["data"] = json!("1".repeat(513)),
            6 => {
                let mut data = bs58::decode(c.swap()[0]["data"].as_str().unwrap())
                    .into_vec()
                    .unwrap();
                data.push(0);
                c.swap()[0]["data"] = json!(bs58::encode(data).into_string());
            }
            7 => {
                let ix = c.swap()[0].clone();
                c.swap().extend(vec![ix; 65]);
            }
            8 => c.swap()[0]["accounts"]
                .as_array_mut()
                .unwrap()
                .push(json!(0)),
            _ => {
                c.swap()[0]["programIdIndex"] = json!(account_index(
                    &c.prepared,
                    solana_system_interface::program::id()
                ))
            }
        }
        assert!(c.run().await.is_err(), "damage={damage}");
    }
}

#[tokio::test]
async fn only_classic_spl_unchecked_transfers_are_accepted() {
    for encoding in [Encoding::Compiled, Encoding::Partial, Encoding::Parsed] {
        let mut c = Case::default();
        c.swap()[0] = movement(&c.prepared, true, 100, 0, encoding, Kind::Legacy);
        c.swap()[1] = movement(&c.prepared, false, 120, 0, encoding, Kind::Legacy);
        assert_eq!(c.run().await.unwrap(), 120);
        let mut c = Case::new(false, true, true, true, USDC_TOKEN_ACCOUNT, encoding);
        c.swap()[0] = movement(&c.prepared, true, 100, 1, encoding, Kind::Legacy);
        assert!(c.run().await.is_err());
    }
}

#[tokio::test]
async fn declared_token2022_transfer_fee_is_checked_in_all_trace_encodings() {
    for encoding in [Encoding::Compiled, Encoding::Partial, Encoding::Parsed] {
        for sell in [false, true] {
            for wrong in [false, true] {
                let mut c = Case::new(false, true, sell, true, USDC_TOKEN_ACCOUNT, encoding);
                let position = if sell { 0 } else { 1 };
                let amount = if sell { 100 } else { 120 };
                let fee = if sell { 1 } else { 2 };
                c.swap()[position] = movement(
                    &c.prepared,
                    sell,
                    amount,
                    fee + u64::from(wrong),
                    encoding,
                    Kind::Fee,
                );
                c.add_balances();
                let result = c.run().await;
                assert_eq!(result.is_ok(), !wrong);
                if let Ok(value) = result {
                    assert_eq!(value, c.output);
                }
            }
        }
    }
}

#[tokio::test]
async fn optional_same_simulation_balances_are_cross_checked_not_trusted_blindly() {
    for damage in 0..12 {
        let mut c = Case::default();
        c.add_balances();
        match damage {
            0 => {
                c.reply.as_object_mut().unwrap().remove("preTokenBalances");
            }
            1 => {
                c.reply.as_object_mut().unwrap().remove("postTokenBalances");
            }
            2 => {
                let r = c.reply["preTokenBalances"][0].clone();
                c.reply["preTokenBalances"].as_array_mut().unwrap().push(r);
            }
            3 => c.reply["preTokenBalances"][0]["accountIndex"] = json!(255),
            4 => c.reply["preTokenBalances"][1]["uiTokenAmount"]["amount"] = json!("2001"),
            5 => c.reply["postTokenBalances"][1]["uiTokenAmount"]["amount"] = json!("2121"),
            6 => c.reply["preTokenBalances"][0]["uiTokenAmount"]["amount"] = json!("+1000"),
            7 => {
                c.reply["preTokenBalances"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("owner");
            }
            8 => c.reply["preTokenBalances"][0]["programId"] = json!(key(99).to_string()),
            9 => c.reply["preTokenBalances"][0]["mint"] = json!(USDC_TOKEN_ACCOUNT.to_string()),
            10 => {
                c.reply["preTokenBalances"]
                    .as_array_mut()
                    .unwrap()
                    .remove(1);
            }
            _ => c.reply["preTokenBalances"][0]["uiTokenAmount"]["decimals"] = json!(9),
        }
        assert!(c.run().await.is_err(), "damage={damage}");
    }
}

#[tokio::test]
async fn missing_pre_output_balance_requires_this_simulation_initialization_evidence() {
    for encoding in [Encoding::Compiled, Encoding::Partial, Encoding::Parsed] {
        let mut c = Case::new(false, false, true, false, USDC_TOKEN_ACCOUNT, encoding);
        c.add_balances();
        assert_eq!(c.run().await.unwrap(), 120);
        let mut c = Case::new(false, false, true, false, USDC_TOKEN_ACCOUNT, encoding);
        c.add_balances();
        c.reply["innerInstructions"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        assert!(c.run().await.is_err());
    }
}

#[tokio::test]
async fn trace_post_state_conservation_prevents_impossible_amounts() {
    for damage in 0..4 {
        let mut c = Case::default();
        let index = match damage {
            0 => 7,
            1 => 3,
            2 => 6,
            _ => 4,
        };
        change_token(&mut c.reply["accounts"][index], |a| {
            a.amount = if damage < 2 { 0 } else { u64::MAX }
        });
        assert!(c.run().await.is_err(), "damage={damage}");
    }
}

#[tokio::test]
async fn transport_errors_are_redacted_without_retry_and_cancellation_never_sends() {
    let mut c = Case::default();
    c.source.rpc.fail_on = Some(RpcRequest::SimulateTransaction);
    let error = c.run().await.err().unwrap().to_string();
    assert!(!error.contains("secret"));
    assert_eq!(
        c.source.rpc.methods(),
        [
            RpcRequest::GetMultipleAccounts,
            RpcRequest::SimulateTransaction
        ]
    );
    for request in [
        RpcRequest::GetMultipleAccounts,
        RpcRequest::SimulateTransaction,
    ] {
        let mut c = Case::default();
        c.source.rpc.block = Some(request);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), c.run())
                .await
                .is_err()
        );
        assert!(c.source.rpc.dropped.load(Ordering::SeqCst));
        assert!(c.source.rpc.methods().iter().all(|r| matches!(
            r,
            RpcRequest::GetMultipleAccounts | RpcRequest::SimulateTransaction
        )));
    }
}

#[tokio::test]
async fn full_input_consumption_and_u64_values_never_round_through_floats() {
    for amount in [1, 9_007_199_254_740_993, u64::MAX] {
        for encoding in [Encoding::Compiled, Encoding::Partial, Encoding::Parsed] {
            let mut c = Case::default();
            let p = c.prepared.accounts();
            c.prepared = prepare_damm_probe(
                p.wallet,
                &p.pool,
                p.input_mint,
                p.output_mint,
                amount,
                Hash::new_from_array([6; 32]),
                false,
            )
            .unwrap();
            change_token(&mut c.source.rpc.values[6], |a| a.amount = amount);
            change_token(&mut c.source.rpc.values[7], |a| a.amount = 0);
            change_token(&mut c.source.rpc.values[3], |a| a.amount = 0);
            change_token(&mut c.source.rpc.values[4], |a| a.amount = amount);
            let mut post = c.source.rpc.values.clone();
            post[5] = clock(43, 17);
            change_token(&mut post[6], |a| a.amount = 0);
            change_token(&mut post[7], |a| a.amount = amount);
            change_token(&mut post[3], |a| a.amount = amount);
            change_token(&mut post[4], |a| a.amount = 0);
            c.reply["accounts"] = json!(post);
            c.before = [amount, 0, 0, amount];
            c.swap()[0] = movement(&c.prepared, true, amount, 0, encoding, Kind::Checked);
            c.swap()[1] = movement(&c.prepared, false, amount, 0, encoding, Kind::Checked);
            c.add_balances();
            c.reply["postTokenBalances"][1]["uiTokenAmount"]["uiAmount"] = json!(0.125);
            assert_eq!(c.run().await.unwrap(), amount);
        }
    }
}

fn fee_config(bps: u16, cap: u64) -> Vec<u8> {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::{TransferFee, TransferFeeConfig},
            BaseStateWithExtensionsMut, StateWithExtensionsMut,
        },
        state::Mint,
    };
    let mut data = extended_mint(false, false, bps);
    let mut state = StateWithExtensionsMut::<Mint>::unpack(&mut data).unwrap();
    state
        .get_extension_mut::<TransferFeeConfig>()
        .unwrap()
        .newer_transfer_fee = TransferFee {
        epoch: 20u64.into(),
        maximum_fee: cap.into(),
        transfer_fee_basis_points: bps.into(),
    };
    data
}

#[tokio::test]
async fn simulated_epoch_rounding_caps_zero_and_full_transfer_fees_determine_net_output() {
    for (gross, bps, cap, fee) in [
        (1, 100, u64::MAX, 1),
        (120, 300, 9, 4),
        (120, 10_000, 9, 9),
        (u64::MAX, 10_000, 7, 7),
        (120, 0, u64::MAX, 0),
        (120, 10_000, u64::MAX, 120),
    ] {
        let mut c = Case::new(
            false,
            true,
            false,
            true,
            USDC_TOKEN_ACCOUNT,
            Encoding::Compiled,
        );
        // The snapshot is epoch 17. Only the post-simulation epoch 20 fee is authoritative.
        c.reply["accounts"][1] = ui(spl_token_2022_interface::ID, fee_config(bps, cap));
        c.reply["accounts"][5] = clock(43, 20);
        c.before[1] = 0;
        c.before[3] = gross;
        let net = gross - fee;
        change_token(&mut c.reply["accounts"][7], |a| a.amount = net);
        change_token(&mut c.reply["accounts"][3], |a| a.amount = 0);
        c.swap()[1] = movement(
            &c.prepared,
            false,
            gross,
            fee,
            Encoding::Compiled,
            Kind::Fee,
        );
        c.add_balances();
        let result = c.run().await;
        assert_eq!(result.is_ok(), net > 0, "gross={gross} bps={bps} cap={cap}");
        if let Ok(output) = result {
            assert_eq!(output, net);
        }
    }
}

#[tokio::test]
async fn zero_credited_input_and_post_simulation_mint_policy_changes_fail_closed() {
    for damage in 0..5 {
        let mut c = Case::new(
            false,
            true,
            true,
            true,
            USDC_TOKEN_ACCOUNT,
            Encoding::Compiled,
        );
        match damage {
            0 => {
                c.reply["accounts"][1] =
                    ui(spl_token_2022_interface::ID, fee_config(10_000, u64::MAX));
                c.reply["accounts"][5] = clock(43, 20);
            }
            1 => {
                c.reply["accounts"][1] = ui(
                    spl_token_2022_interface::ID,
                    extended_mint(true, false, 300),
                )
            }
            2 => {
                c.reply["accounts"][1] = ui(
                    spl_token_2022_interface::ID,
                    extended_mint(false, true, 300),
                )
            }
            3 => c.reply["accounts"][1]["owner"] = json!(key(99).to_string()),
            _ => {
                c.reply["accounts"][3] = token(
                    spl_token_2022_interface::ID,
                    key(4),
                    accounts::AUTHORITY,
                    1100,
                )
            }
        }
        assert!(c.run().await.is_err(), "damage={damage}");
    }
}

#[tokio::test]
async fn a_new_ata_between_rpc_banks_is_not_assumed_to_have_zero_simulation_balance() {
    for balances in [false, true] {
        let mut c = Case::new(
            false,
            false,
            true,
            false,
            USDC_TOKEN_ACCOUNT,
            Encoding::Compiled,
        );
        c.before[1] = 1000;
        change_token(&mut c.reply["accounts"][7], |a| a.amount = 1120);
        c.reply["innerInstructions"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        if balances {
            c.add_balances();
            let mut row = c.reply["postTokenBalances"][1].clone();
            row["uiTokenAmount"]["amount"] = json!("1000");
            c.reply["preTokenBalances"]
                .as_array_mut()
                .unwrap()
                .push(row);
        }
        assert_eq!(c.run().await.unwrap(), 120);
    }
    let mut c = Case::new(
        false,
        false,
        true,
        false,
        USDC_TOKEN_ACCOUNT,
        Encoding::Compiled,
    );
    c.add_balances();
    let mut row = c.reply["postTokenBalances"][1].clone();
    row["uiTokenAmount"]["amount"] = json!("0");
    c.reply["preTokenBalances"]
        .as_array_mut()
        .unwrap()
        .push(row);
    assert!(c
        .run()
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("initialization contradicts"));
}

#[tokio::test]
async fn token_mutations_unknown_partial_accounts_and_init_role_drift_are_rejected() {
    for encoding in [Encoding::Compiled, Encoding::Partial, Encoding::Parsed] {
        for damage in 0..4 {
            let mut c = Case::new(false, false, true, false, USDC_TOKEN_ACCOUNT, encoding);
            let p = c.prepared.accounts();
            let close = spl_token_interface::instruction::close_account(
                &spl_token_interface::ID,
                &p.output_ata,
                &p.wallet,
                &p.wallet,
                &[],
            )
            .unwrap();
            match damage {
                0 => {
                    let encoded = encode(
                        &close,
                        &c.prepared,
                        encoding,
                        json!({"type":"closeAccount","info":{"account":p.output_ata.to_string(),"destination":p.wallet.to_string(),"owner":p.wallet.to_string()}}),
                    );
                    c.swap().push(encoded);
                }
                1 => {
                    let groups = c.reply["innerInstructions"].as_array_mut().unwrap();
                    groups.swap(0, 1);
                }
                2 => {
                    let init = c.reply["innerInstructions"][0]["instructions"][2].clone();
                    c.reply["innerInstructions"][0]["instructions"]
                        .as_array_mut()
                        .unwrap()
                        .push(init);
                }
                _ => {
                    c.reply["innerInstructions"][0]["instructions"][2] = if matches!(
                        encoding,
                        Encoding::Parsed
                    ) {
                        json!({"programId":spl_token_interface::ID.to_string(),"program":"spl-token","parsed":{"type":"initializeAccount3","info":{"account":p.output_ata.to_string(),"mint":p.input_mint.to_string(),"owner":p.wallet.to_string()}}})
                    } else {
                        let ix = spl_token_interface::instruction::initialize_account3(
                            &spl_token_interface::ID,
                            &p.output_ata,
                            &p.input_mint,
                            &p.wallet,
                        )
                        .unwrap();
                        encode(&ix, &c.prepared, encoding, Value::Null)
                    };
                }
            }
            assert!(c.run().await.is_err());
        }
    }
    let mut c = Case::new(
        false,
        false,
        true,
        true,
        USDC_TOKEN_ACCOUNT,
        Encoding::Partial,
    );
    c.swap()[0]["accounts"][0] = json!(key(99).to_string());
    assert!(c.run().await.is_err());
}

#[tokio::test]
async fn count_limits_apply_even_to_otherwise_allowed_readonly_events() {
    let mut c = Case::default();
    let event = json!({"programIdIndex":account_index(&c.prepared,accounts::METEORA_DAMM_V2),"accounts":[],"data":"","stackHeight":2});
    let mut events = vec![event; 63];
    events.extend(c.swap().clone());
    *c.swap() = events;
    assert!(c
        .run()
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("too many inner instructions"));
}

#[tokio::test]
async fn partial_fill_and_exact_out_profiles_never_request_a_quote_simulation() {
    for mode in [1, 2] {
        let mut c = Case::default();
        c.source.params.swap_mode = mode;
        c.prepared = prepare_damm_probe(
            key(5),
            &c.source.params,
            key(4),
            c.source.quote,
            100,
            Hash::new_from_array([6; 32]),
            false,
        )
        .unwrap();
        assert!(c
            .run()
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("requires exact-in"));
        assert_eq!(c.source.rpc.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}
