use super::*;
use crate::adapters::sdk::damm_probe::evidence::tests::{
    change_token, clock, extended_mint, extended_token, key, mint, token, ui, Fixture,
};
use serde_json::{json, Value};
use sol_trade_sdk::{
    constants::{SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    instruction::utils::meteora_damm_v2::accounts,
};
use solana_client::rpc_request::RpcRequest;
use solana_sdk::{hash::Hash, instruction::Instruction, signature::Signature};
use std::sync::atomic::Ordering;

const RESERVE: u64 = 2_039_280;
const FEE: u64 = 5_000;
const SAME_BANK_WALLET: u64 = 100_000_000;

pub(crate) fn buy_runtime_fixture(
    wallet: Pubkey,
    native: bool,
) -> (Vec<Value>, Value, solana_sdk::message::Message) {
    let case = Case::with_wallet_profile(
        false,
        native,
        false,
        true,
        (true, key(4)),
        Encoding::Parsed,
        (wallet, true),
    );
    (
        case.source.rpc.values,
        case.result,
        case.plan.transaction.message,
    )
}

#[derive(Clone, Copy)]
enum Encoding {
    Parsed,
    Compiled,
    Partial,
}
struct Case {
    source: Fixture,
    plan: PreparedDammBuyProbe,
    result: Value,
    net_output: u64,
}

fn state(program: Pubkey, mint: Pubkey, owner: Pubkey, amount: u64, extended: bool) -> Value {
    let mut value = if extended {
        ui(program, extended_token(mint, owner, false, false))
    } else {
        token(program, mint, owner, amount)
    };
    change_token(&mut value, |a| {
        a.amount = amount;
        if mint == WSOL_TOKEN_ACCOUNT {
            a.is_native = Some(RESERVE).into();
        }
    });
    value["lamports"] = json!(
        RESERVE
            + if mint == WSOL_TOKEN_ACCOUNT {
                amount
            } else {
                0
            }
    );
    value
}
fn encode(plan: &PreparedDammBuyProbe, ix: Instruction, parsed: Value, kind: Encoding) -> Value {
    let i = |key| u8::try_from(index(plan, key).unwrap()).unwrap();
    match kind {
        Encoding::Parsed => {
            json!({"programId":ix.program_id.to_string(),"program":"spl-token","parsed":parsed,"stackHeight":2})
        }
        Encoding::Compiled => {
            json!({"programIdIndex":i(ix.program_id),"accounts":ix.accounts.iter().map(|a|i(a.pubkey)).collect::<Vec<_>>(),"data":bs58::encode(ix.data).into_string(),"stackHeight":2})
        }
        Encoding::Partial => {
            json!({"programId":ix.program_id.to_string(),"accounts":ix.accounts.iter().map(|a|a.pubkey.to_string()).collect::<Vec<_>>(),"data":bs58::encode(ix.data).into_string(),"stackHeight":2})
        }
    }
}
fn init(plan: &PreparedDammBuyProbe, input: bool, kind: Encoding) -> Vec<Value> {
    let a = &plan.accounts;
    let mint = if input { a.input_mint } else { a.output_mint };
    let ata = if input { a.input_ata } else { a.output_ata };
    let program = if mint == a.pool.token_a_mint {
        a.pool.token_a_program
    } else {
        a.pool.token_b_program
    };
    let ix = spl_token_2022_interface::instruction::initialize_account3(
        &program, &ata, &mint, &a.wallet,
    )
    .unwrap();
    vec![encode(
        plan,
        ix,
        json!({"type":"initializeAccount3","info":{"account":ata.to_string(),"mint":mint.to_string(),"owner":a.wallet.to_string()}}),
        kind,
    )]
}
fn transfer(plan: &PreparedDammBuyProbe, input: bool, kind: Encoding) -> Value {
    let a = &plan.accounts;
    let a_in = a.input_mint == a.pool.token_a_mint;
    let (mint, from, to, authority, amount) = if input {
        (
            a.input_mint,
            a.input_ata,
            if a_in {
                a.pool.token_a_vault
            } else {
                a.pool.token_b_vault
            },
            a.wallet,
            a.amount,
        )
    } else {
        (
            a.output_mint,
            if a_in {
                a.pool.token_b_vault
            } else {
                a.pool.token_a_vault
            },
            a.output_ata,
            accounts::AUTHORITY,
            120,
        )
    };
    let program = if mint == a.pool.token_a_mint {
        a.pool.token_a_program
    } else {
        a.pool.token_b_program
    };
    let decimals = if mint == WSOL_TOKEN_ACCOUNT { 9 } else { 6 };
    let ix = spl_token_2022_interface::instruction::transfer_checked(
        &program,
        &from,
        &mint,
        &to,
        &authority,
        &[],
        amount,
        decimals,
    )
    .unwrap();
    encode(
        plan,
        ix,
        json!({"type":"transferChecked","info":{"source":from.to_string(),"mint":mint.to_string(),"destination":to.to_string(),"authority":authority.to_string(),"tokenAmount":{"amount":amount.to_string(),"decimals":decimals}}}),
        kind,
    )
}

impl Case {
    fn new(
        reversed: bool,
        native: bool,
        extended: bool,
        input_exists: bool,
        output_exists: bool,
        kind: Encoding,
    ) -> Self {
        Self::with_output(
            reversed,
            native,
            extended,
            input_exists,
            (output_exists, key(4)),
            kind,
        )
    }
    fn with_output(
        reversed: bool,
        native: bool,
        extended: bool,
        input_exists: bool,
        output: (bool, Pubkey),
        kind: Encoding,
    ) -> Self {
        Self::with_wallet(
            reversed,
            native,
            extended,
            input_exists,
            output,
            kind,
            key(5),
        )
    }
    fn with_wallet(
        reversed: bool,
        native: bool,
        extended: bool,
        input_exists: bool,
        output: (bool, Pubkey),
        kind: Encoding,
        wallet: Pubkey,
    ) -> Self {
        Self::with_wallet_profile(
            reversed,
            native,
            extended,
            input_exists,
            output,
            kind,
            (wallet, false),
        )
    }
    fn with_wallet_profile(
        reversed: bool,
        native: bool,
        extended: bool,
        input_exists: bool,
        output: (bool, Pubkey),
        kind: Encoding,
        profile: (Pubkey, bool),
    ) -> Self {
        let (wallet, rate_limiter_sysvar) = profile;
        let (output_exists, target) = output;
        assert!(native || input_exists);
        let quote_mint = if native {
            WSOL_TOKEN_ACCOUNT
        } else {
            USDC_TOKEN_ACCOUNT
        };
        let mut source = Fixture::new(reversed, extended, quote_mint, output_exists);
        source.params.include_rate_limiter_sysvar = rate_limiter_sysvar;
        let target_program = if extended {
            spl_token_2022_interface::ID
        } else {
            spl_token_interface::ID
        };
        let (mi, vi) = if reversed { (2, 4) } else { (1, 3) };
        if target != key(4) {
            assert!(!native && !extended && target == WSOL_TOKEN_ACCOUNT);
            if reversed {
                source.pool.token_b_mint = target;
                source.params.token_b_mint = target;
            } else {
                source.pool.token_a_mint = target;
                source.params.token_a_mint = target;
            }
            source.update_pool();
            source.rpc.values[mi] = mint(target_program, 9);
        }
        if extended {
            source.rpc.values[mi] = ui(target_program, extended_mint(false, false, 300));
        }
        source.rpc.values[vi] = state(
            target_program,
            target,
            accounts::AUTHORITY,
            1_000_000,
            extended,
        );
        let qi = if reversed { 3 } else { 4 };
        source.rpc.values[qi] = state(
            spl_token_interface::ID,
            quote_mint,
            accounts::AUTHORITY,
            2_000_000,
            false,
        );
        source.rpc.values[6] = if input_exists {
            state(spl_token_interface::ID, quote_mint, wallet, 1000, false)
        } else {
            Value::Null
        };
        source.rpc.values[7] = if output_exists {
            state(target_program, target, wallet, 2000, extended)
        } else {
            Value::Null
        };
        let mut wallet_account = ui(solana_system_interface::program::id(), vec![]);
        wallet_account["lamports"] = json!(10_000_000);
        source.rpc.values.push(wallet_account);
        let plan = super::super::prepare(
            wallet,
            &source.params,
            if native {
                SOL_TOKEN_ACCOUNT
            } else {
                USDC_TOKEN_ACCOUNT
            },
            target,
            100,
            Hash::new_from_array([6; 32]),
            false,
        )
        .unwrap();
        let mut post = source.rpc.values.clone();
        let input_before = if input_exists { 1500 } else { 0 };
        let output_before = if output_exists { 2500 } else { 0 };
        let net_output = if extended { 118 } else { 120 };
        post[6] = state(
            spl_token_interface::ID,
            quote_mint,
            wallet,
            input_before + if native { 100 } else { 0 } - 100,
            false,
        );
        post[7] = state(
            target_program,
            target,
            wallet,
            output_before + net_output,
            extended,
        );
        post[qi] = state(
            spl_token_interface::ID,
            quote_mint,
            accounts::AUTHORITY,
            2_000_100,
            false,
        );
        post[vi] = state(
            target_program,
            target,
            accounts::AUTHORITY,
            999_880,
            extended,
        );
        post[5] = clock(43, 17);
        let rents = u64::from(!input_exists) * RESERVE + u64::from(!output_exists) * RESERVE;
        post[8]["lamports"] = json!(SAME_BANK_WALLET - u64::from(native) * 100 - rents - FEE);
        let mut groups = vec![];
        if !input_exists {
            groups.push(json!({"index":0,"instructions":init(&plan,true,kind)}));
        }
        if !output_exists {
            groups.push(
                json!({"index":layout(&plan).output_setup,"instructions":init(&plan,false,kind)}),
            );
        }
        groups.push(json!({"index":layout(&plan).swap,"instructions":[transfer(&plan,true,kind),transfer(&plan,false,kind)]}));
        let roles = [
            (
                plan.accounts.input_ata,
                quote_mint,
                spl_token_interface::ID,
                wallet,
                input_before,
                6,
                input_exists,
            ),
            (
                plan.accounts.output_ata,
                target,
                target_program,
                wallet,
                output_before,
                7,
                output_exists,
            ),
        ];
        let mut pre_tokens = vec![];
        let mut post_tokens = vec![];
        let mut before = vec![1_000_000u64; plan.transaction.message.account_keys.len()];
        let mut after = before.clone();
        for (position, key) in keys(&plan).into_iter().enumerate() {
            if let Ok(i) = index(&plan, key) {
                before[i] = source.rpc.values[position]["lamports"]
                    .as_u64()
                    .unwrap_or(0);
                after[i] = post[position]["lamports"].as_u64().unwrap_or(0);
            }
        }
        before[index(&plan, wallet).unwrap()] = SAME_BANK_WALLET;
        for (ata, mint, program, owner, pre_amount, position, exists) in roles.into_iter().chain([
            (
                if reversed {
                    source.params.token_a_vault
                } else {
                    source.params.token_b_vault
                },
                quote_mint,
                spl_token_interface::ID,
                accounts::AUTHORITY,
                2_000_000,
                qi,
                true,
            ),
            (
                if reversed {
                    source.params.token_b_vault
                } else {
                    source.params.token_a_vault
                },
                target,
                target_program,
                accounts::AUTHORITY,
                1_000_000,
                vi,
                true,
            ),
        ]) {
            let i = index(&plan, ata).unwrap();
            if exists {
                let value = state(program, mint, owner, pre_amount, extended && mint == target);
                before[i] = value["lamports"].as_u64().unwrap();
                pre_tokens.push(Self::row(&plan, ata, mint, program, owner, pre_amount));
            } else {
                before[i] = 0;
            }
            let data = crate::adapters::sdk::damm_probe::evidence::tests::bytes(&post[position]);
            let token = spl_token_interface::state::Account::unpack(&data[..165]).unwrap();
            post_tokens.push(Self::row(&plan, ata, mint, program, owner, token.amount));
        }
        source.rpc.simulation_slot = 43;
        let result = json!({"err":null,"accounts":post,"innerInstructions":groups,"fee":FEE,
            "preBalances":before,"postBalances":after,"preTokenBalances":pre_tokens,"postTokenBalances":post_tokens});
        Self {
            source,
            plan,
            result,
            net_output,
        }
    }
    fn row(
        plan: &PreparedDammBuyProbe,
        ata: Pubkey,
        mint: Pubkey,
        program: Pubkey,
        owner: Pubkey,
        amount: u64,
    ) -> Value {
        json!({"accountIndex":index(plan,ata).unwrap(),"mint":mint.to_string(),"owner":owner.to_string(),"programId":program.to_string(),
            "uiTokenAmount":{"amount":amount.to_string(),"decimals":if mint==WSOL_TOKEN_ACCOUNT{9}else{6},"uiAmount":null,"uiAmountString":"ignored"}})
    }
    fn default() -> Self {
        Self::new(false, true, false, false, false, Encoding::Parsed)
    }
    async fn run(&self) -> Result<BuyOutputEvidence> {
        let mut read = self.source.rpc.clone();
        read.simulation_value = Some(self.result.clone());
        quote(&read.client(), &self.plan, 42).await
    }
    fn swap(&mut self) -> &mut Vec<Value> {
        let swap = layout(&self.plan).swap;
        self.result["innerInstructions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|g| g["index"] == swap)
            .unwrap()["instructions"]
            .as_array_mut()
            .unwrap()
    }
}

#[tokio::test]
async fn damm_buy_quote_covers_native_and_usdc_existing_or_new_atas_and_all_trace_encodings() {
    let mut count = 0;
    for reversed in [false, true] {
        for native in [false, true] {
            for extended in [false, true] {
                for input_exists in [false, true] {
                    if !native && !input_exists {
                        continue;
                    }
                    for output_exists in [false, true] {
                        for kind in [Encoding::Parsed, Encoding::Compiled, Encoding::Partial] {
                            let case = Case::new(
                                reversed,
                                native,
                                extended,
                                input_exists,
                                output_exists,
                                kind,
                            );
                            let value = case.run().await.unwrap();
                            assert_eq!(value.output_units, case.net_output);
                            assert_eq!(
                                value.minimum(&case.plan, 1500).unwrap(),
                                case.net_output * 8500 / 10000
                            );
                            assert_eq!(
                                (
                                    value.account_slot,
                                    value.simulation_slot,
                                    value.probe_fee_lamports
                                ),
                                (42, 43, FEE)
                            );
                            assert_eq!(case.plan.transaction.signatures, [Signature::default()]);
                            assert_eq!(
                                case.source.rpc.methods(),
                                [
                                    RpcRequest::GetMultipleAccounts,
                                    RpcRequest::SimulateTransaction
                                ]
                            );
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 72);
}

#[tokio::test]
async fn damm_buy_quote_uses_same_bank_funding_not_earlier_rpc_balance_deltas() {
    let mut case = Case::default();
    case.source.rpc.values[8]["lamports"] = json!(100);
    let value = case.run().await.unwrap();
    assert_eq!(value.output_units, 120);
    assert_ne!(
        case.source.rpc.values[8]["lamports"],
        json!(SAME_BANK_WALLET)
    );
}

#[tokio::test]
async fn damm_buy_quote_requires_fee_and_complete_same_bank_token_and_native_balances() {
    for field in [
        "fee",
        "preBalances",
        "postBalances",
        "preTokenBalances",
        "postTokenBalances",
        "accounts",
        "innerInstructions",
    ] {
        let mut case = Case::default();
        case.result[field] = Value::Null;
        assert!(case.run().await.is_err(), "field={field}");
    }
    for field in ["preBalances", "postBalances", "accounts"] {
        let mut case = Case::default();
        case.result[field].as_array_mut().unwrap().pop();
        assert!(case.run().await.is_err(), "field={field}");
    }
}

#[tokio::test]
async fn damm_buy_quote_refuses_rpc_errors_replacement_alt_and_slot_regression() {
    for mutation in 0..7 {
        let mut case = Case::default();
        match mutation {
            0 => case.result["err"] = json!({"InstructionError":[4,"InvalidArgument"]}),
            1 => {
                case.result["replacementBlockhash"] = json!({"blockhash":Hash::new_from_array([9;32]).to_string(),"lastValidBlockHeight":999})
            }
            2 => {
                case.result["loadedAddresses"] =
                    json!({"writable":[key(99).to_string()],"readonly":[]})
            }
            3 => case.source.rpc.simulation_slot = 41,
            4 => case.source.rpc.bank = 41,
            5 => case.source.rpc.fail = true,
            _ => case.source.rpc.fail_on = Some(RpcRequest::SimulateTransaction),
        }
        let error = case.run().await.err().unwrap();
        assert!(!format!("{error:#}").contains("secret"));
    }
}

#[tokio::test]
async fn damm_buy_quote_native_input_and_vaults_require_consistent_native_reserves() {
    for post in [false, true] {
        for mutation in 0..5 {
            let mut case = Case::new(false, true, false, true, true, Encoding::Parsed);
            let values = if post {
                case.result["accounts"].as_array_mut().unwrap()
            } else {
                &mut case.source.rpc.values
            };
            let i = if mutation == 4 { 4 } else { 6 };
            match mutation {
                0 => change_token(&mut values[i], |a| a.is_native = None.into()),
                1 => change_token(&mut values[i], |a| a.is_native = Some(0).into()),
                2 => values[i]["lamports"] = json!(1),
                3 => {
                    let old = values[i]["lamports"].as_u64().unwrap();
                    // A deficit is invalid; a positive unsynchronized deposit is legitimate.
                    values[i]["lamports"] = json!(old - 1);
                }
                _ => change_token(&mut values[i], |a| a.is_native = None.into()),
            }
            assert!(case.run().await.is_err(), "post={post} mutation={mutation}");
        }
    }
}

#[tokio::test]
async fn damm_buy_quote_wallet_owner_data_native_balance_and_usdc_balance_fail_before_simulation() {
    for mutation in 0..6 {
        let mut case = Case::new(false, mutation < 4, false, true, true, Encoding::Parsed);
        match mutation {
            0 => case.source.rpc.values[8]["owner"] = json!(key(99).to_string()),
            1 => case.source.rpc.values[8] = ui(solana_system_interface::program::id(), vec![0]),
            2 => case.source.rpc.values[8]["lamports"] = json!(99),
            3 => case.source.rpc.values[8]["executable"] = json!(true),
            4 => change_token(&mut case.source.rpc.values[6], |a| a.amount = 99),
            _ => case.source.rpc.values[6] = Value::Null,
        }
        assert!(case.run().await.is_err());
        assert_eq!(case.source.rpc.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn damm_buy_quote_native_funding_cannot_hide_missing_rent_fee_or_extra_wallet_debit() {
    for mutation in 0..6 {
        let mut case = Case::default();
        let wallet = index(&case.plan, key(5)).unwrap();
        let input = index(&case.plan, case.plan.accounts.input_ata).unwrap();
        match mutation {
            0 => case.result["fee"] = json!(FEE + 1),
            1 => case.result["preBalances"][wallet] = json!(SAME_BANK_WALLET + 1),
            2 => case.result["preBalances"][input] = json!(1),
            3 => case.result["postBalances"][input] = json!(RESERVE + 1),
            4 => {
                let v = case.result["accounts"][8]["lamports"].as_u64().unwrap() + 1;
                case.result["accounts"][8]["lamports"] = json!(v);
                case.result["postBalances"][wallet] = json!(v);
            }
            _ => case.result["preBalances"][wallet] = json!(1),
        }
        assert!(case.run().await.is_err(), "mutation={mutation}");
    }
}

#[tokio::test]
async fn damm_buy_quote_net_output_and_input_are_not_replaced_by_gross_or_cross_bank_changes() {
    for mutation in 0..6 {
        let mut case = Case::new(false, false, true, true, true, Encoding::Parsed);
        match mutation {
            0 => case.swap()[0]["parsed"]["info"]["tokenAmount"]["amount"] = json!("99"),
            1 => case.swap()[1]["parsed"]["info"]["tokenAmount"]["amount"] = json!("0"),
            2 => case.result["postTokenBalances"][1]["uiTokenAmount"]["amount"] = json!("2620"),
            3 => case.result["preTokenBalances"][1]["uiTokenAmount"]["amount"] = json!("2000"),
            4 => case.swap()[1]["parsed"]["info"]["tokenAmount"]["decimals"] = json!(9),
            _ => case.swap()[1]["parsed"]["info"]["mint"] = json!(USDC_TOKEN_ACCOUNT.to_string()),
        }
        assert!(case.run().await.is_err(), "mutation={mutation}");
    }
}

#[tokio::test]
async fn damm_buy_quote_rejects_wrong_setup_groups_duplicate_init_and_non_swap_transfers() {
    for mutation in 0..6 {
        let mut case = Case::default();
        match mutation {
            0 => case.result["innerInstructions"][0]["index"] = json!(2),
            1 => {
                let v = case.result["innerInstructions"][0]["instructions"][0].clone();
                case.result["innerInstructions"][0]["instructions"]
                    .as_array_mut()
                    .unwrap()
                    .push(v);
            }
            2 => {
                case.result["innerInstructions"][0]["instructions"][0]["parsed"]["info"]
                    ["account"] = json!(case.plan.accounts.output_ata.to_string())
            }
            3 => {
                case.result["innerInstructions"][1]["instructions"][0]["parsed"]["info"]["mint"] =
                    json!(WSOL_TOKEN_ACCOUNT.to_string())
            }
            4 => {
                let transfer = case.swap()[0].clone();
                case.result["innerInstructions"][0]["instructions"]
                    .as_array_mut()
                    .unwrap()
                    .push(transfer);
            }
            _ => {
                case.swap()[0]["programId"] =
                    json!(solana_system_interface::program::id().to_string())
            }
        }
        assert!(case.run().await.is_err(), "mutation={mutation}");
    }
}

#[tokio::test]
async fn damm_buy_quote_token2022_uses_the_simulation_epoch_fee_not_the_earlier_snapshot_epoch() {
    let mut case = Case::new(false, false, true, true, true, Encoding::Parsed);
    case.result["accounts"][5] = clock(43, 20);
    assert!(case.run().await.is_err());
    change_token(&mut case.result["accounts"][7], |a| a.amount = 2616);
    case.result["accounts"][7]["lamports"] = json!(RESERVE);
    case.result["postTokenBalances"][1]["uiTokenAmount"]["amount"] = json!("2616");
    let value = case.run().await.unwrap();
    assert_eq!(value.output_units, 116);
    assert_eq!(value.minimum(&case.plan, 1500).unwrap(), 98);
}

#[tokio::test]
async fn damm_buy_quote_expiry_message_and_slippage_are_rechecked_without_renewal_or_rpc() {
    let case = Case::default();
    let mut value = case.run().await.unwrap();
    assert_eq!(value.minimum(&case.plan, 1500).unwrap(), 102);
    assert!(value.minimum(&case.plan, 10000).is_err());
    let next = super::super::prepare(
        key(5),
        &case.source.params,
        SOL_TOKEN_ACCOUNT,
        key(4),
        101,
        Hash::new_from_array([6; 32]),
        false,
    )
    .unwrap();
    assert!(value.minimum(&next, 1500).is_err());
    value.started = Instant::now() - Duration::from_secs(4);
    assert!(value.minimum(&case.plan, 1500).is_err());
    assert_eq!(case.source.rpc.methods().len(), 2);
}

#[tokio::test]
async fn damm_buy_quote_cancellation_drops_only_the_original_readonly_request() {
    for method in [
        RpcRequest::GetMultipleAccounts,
        RpcRequest::SimulateTransaction,
    ] {
        let mut case = Case::default();
        case.source.rpc.block = Some(method);
        assert!(tokio::time::timeout(Duration::from_millis(20), case.run())
            .await
            .is_err());
        assert!(case.source.rpc.dropped.load(Ordering::SeqCst));
        assert_eq!(
            case.source.rpc.methods().len(),
            if method == RpcRequest::GetMultipleAccounts {
                1
            } else {
                2
            }
        );
    }
}

#[tokio::test]
async fn damm_buy_quote_has_a_total_deadline_for_account_read_and_simulation() {
    for method in [
        RpcRequest::GetMultipleAccounts,
        RpcRequest::SimulateTransaction,
    ] {
        let mut case = Case::default();
        case.source.rpc.block = Some(method);
        assert!(case
            .run()
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("timed out"));
        assert!(case.source.rpc.dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn damm_buy_quote_zero_context_and_aliased_roles_are_rejected_before_rpc() {
    let mut case = Case::default();
    assert!(quote(&case.source.rpc.client(), &case.plan, 0)
        .await
        .is_err());
    case.plan.accounts.pool.pool = case.plan.accounts.wallet;
    assert!(case.run().await.is_err());
    assert!(case.source.rpc.methods().is_empty());
}

#[test]
fn damm_buy_quote_does_not_relax_the_original_prefunded_sell_account_or_cpi_profile() {
    let case = Case::default();
    let old = crate::adapters::sdk::damm_probe::prepare_damm_probe(
        key(5),
        &case.source.params,
        WSOL_TOKEN_ACCOUNT,
        key(4),
        100,
        Hash::new_from_array([6; 32]),
        false,
    )
    .unwrap();
    assert!(crate::adapters::sdk::damm_probe::evidence::check_snapshot(
        &old,
        42,
        &case.source.rpc.values[..8]
            .iter()
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap())
            .collect::<Vec<_>>(),
        false
    )
    .is_err());
    let result: solana_client::rpc_response::RpcSimulateTransactionResult =
        serde_json::from_value(case.result).unwrap();
    assert!(trace::transfers_for(
        old.view(),
        result.inner_instructions.as_deref().unwrap(),
        trace::Layout {
            input_setup: None,
            output_setup: 0,
            swap: 1,
        },
    )
    .is_err());
}

#[tokio::test]
async fn damm_buy_quote_native_rent_overflow_cannot_produce_output_evidence() {
    let mut case = Case::default();
    let output = index(&case.plan, case.plan.accounts.output_ata).unwrap();
    case.result["accounts"][7]["lamports"] = json!(u64::MAX);
    case.result["postBalances"][output] = json!(u64::MAX);
    assert!(case
        .run()
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("rent overflow"));
}

mod funding_compatibility;

pub(in crate::adapters::sdk::damm_probe::buy_plan) async fn binding_fixture(
    native: bool,
    extended: bool,
    output_exists: bool,
) -> (PreparedDammBuyProbe, BuyOutputEvidence) {
    let case = Case::new(
        false,
        native,
        extended,
        true,
        output_exists,
        Encoding::Parsed,
    );
    let evidence = case.run().await.unwrap();
    (case.plan, evidence)
}

pub(in crate::adapters::sdk::damm_probe::buy_plan) async fn message_fixture(
    wallet: Pubkey,
    native: bool,
    extended: bool,
    exists: bool,
    reversed: bool,
    remaining: Option<u64>,
) -> (PreparedDammBuyProbe, BuyOutputEvidence) {
    let mut case = Case::with_wallet(
        reversed,
        native,
        extended,
        true,
        (exists, key(4)),
        Encoding::Parsed,
        wallet,
    );
    if let Some(remaining) = remaining {
        let i = index(&case.plan, wallet).unwrap();
        let debit = case.result["preBalances"][i].as_u64().unwrap()
            - case.result["postBalances"][i].as_u64().unwrap();
        case.result["preBalances"][i] = json!(remaining.checked_add(debit).unwrap());
        case.result["postBalances"][i] = json!(remaining);
        case.result["accounts"][8]["lamports"] = json!(remaining);
    }
    let evidence = case.run().await.unwrap();
    (case.plan, evidence)
}

pub(in crate::adapters::sdk::damm_probe::buy_plan) async fn native_output_record_fixture(
    exists: bool,
) -> (PreparedDammBuyProbe, BuyOutputEvidence) {
    let case = Case::with_output(
        false,
        false,
        false,
        true,
        (exists, WSOL_TOKEN_ACCOUNT),
        Encoding::Parsed,
    );
    let evidence = case.run().await.unwrap();
    (case.plan, evidence)
}

pub(in crate::adapters::sdk::damm_probe::buy_plan) fn preparation_fixture(
    native: bool,
    extended: bool,
    output_exists: bool,
) -> (
    PreparedDammBuyProbe,
    crate::adapters::sdk::damm_probe::evidence::tests::ReadRpc,
) {
    let case = Case::new(
        false,
        native,
        extended,
        true,
        output_exists,
        Encoding::Parsed,
    );
    let mut read = case.source.rpc.clone();
    read.simulation_value = Some(case.result);
    (case.plan, read)
}

#[tokio::test]
async fn damm_buy_quote_current_binds_its_own_bank_without_a_separate_slot_rpc() {
    let case = Case::default();
    let mut read = case.source.rpc.clone();
    read.simulation_value = Some(case.result.clone());
    let evidence = quote_current(&read.client(), &case.plan).await.unwrap();
    assert_eq!(evidence.context(), (42, 43, FEE));
    let calls = read.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].1[1]["minContextSlot"].is_null());
    assert_eq!(calls[1].1[1]["minContextSlot"], 42);
}

#[tokio::test]
async fn damm_buy_quote_current_rejects_a_zero_bank_without_simulation() {
    let mut case = Case::default();
    case.source.rpc.bank = 0;
    assert!(quote_current(&case.source.rpc.client(), &case.plan)
        .await
        .is_err());
    assert_eq!(case.source.rpc.methods(), [RpcRequest::GetMultipleAccounts]);
}
