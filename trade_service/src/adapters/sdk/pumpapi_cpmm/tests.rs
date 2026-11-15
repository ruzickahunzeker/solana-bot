//! Actual read/parse/check pipeline with synthetic accounts. No signer or socket.
use super::*;
use crate::{
    adapters::sdk::{
        pumpapi_accounts::resolve_unsigned_accounts, pumpapi_simulation::simulate_unsigned,
    },
    application::trade_command::test_support::command,
    domain::trade::{ExecutionMode, TradeAmount, WSOL_MINT},
};
use base64::Engine;
use serde_json::{json, Value};
use sol_trade_sdk::instruction::utils::raydium_cpmm_types::AmmConfig;
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_program_pack::Pack;
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{Message, VersionedMessage},
    signature::Signature,
};
use spl_token_2022_interface::extension::{
    BaseStateWithExtensions, ExtensionType, StateWithExtensions,
};
use spl_token_interface::state::{Account as TokenAccount, AccountState, Mint};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

fn key(n: u8) -> Pubkey {
    Pubkey::from([n; 32])
}
fn owner() -> Address {
    Address::from_bytes([1; 32])
}
fn ui(owner: Pubkey, data: Vec<u8>) -> Value {
    json!({"lamports":10_000_000,"owner":owner.to_string(),"executable":false,"rentEpoch":0,"space":data.len(),"data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"]})
}
fn pool_ui(pool: &PoolState) -> Value {
    let mut data = POOL_STATE_DISCRIMINATOR.to_vec();
    data.extend(bincode::serialize(pool).unwrap());
    ui(accounts::RAYDIUM_CPMM, data)
}
fn mint_data(decimals: u8) -> Vec<u8> {
    let mut data = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            decimals,
            is_initialized: true,
            supply: 1_000_000_000,
            ..Default::default()
        },
        &mut data,
    )
    .unwrap();
    data
}
fn vault_data(mint: Pubkey, authority: Pubkey, state: AccountState, amount: u64) -> Vec<u8> {
    let mut data = vec![0; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner: authority,
            state,
            amount,
            ..Default::default()
        },
        &mut data,
    )
    .unwrap();
    data
}
fn clock_ui(slot: u64, epoch: u64, time: i64) -> Value {
    ui(
        solana_sdk::sysvar::id(),
        bincode::serialize(&Clock {
            slot,
            epoch,
            unix_timestamp: time,
            ..Default::default()
        })
        .unwrap(),
    )
}
#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}
#[derive(Clone)]
struct ReadRpc {
    pool: PoolState,
    initial: Value,
    values: HashMap<String, Value>,
    first_slot: u64,
    bank_slot: u64,
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    fail: Option<usize>,
    gate: Option<Arc<Gate>>,
}
impl ReadRpc {
    fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
    }
    fn methods(&self) -> Vec<RpcRequest> {
        self.calls.lock().unwrap().iter().map(|(r, _)| *r).collect()
    }
    fn update_pool(&mut self) {
        self.values.insert(key(10).to_string(), pool_ui(&self.pool));
    }
    fn input(&self, side: Side, target0: bool) -> TradeCommand {
        let mut c = command();
        c.route.pool = Pool::RaydiumCpmm;
        c.route.pool_id = Some(Address::from_bytes(key(10).to_bytes()));
        let (mint, quote) = if target0 {
            (self.pool.token0_mint, self.pool.token1_mint)
        } else {
            (self.pool.token1_mint, self.pool.token0_mint)
        };
        c.route.mint = Address::from_bytes(mint.to_bytes());
        c.route.quote_mint = Address::from_bytes(quote.to_bytes());
        c.execution.mode = ExecutionMode::Simulate;
        c.amount = match side {
            Side::Sell => TradeAmount::SellPercent { percent: 100 },
            Side::Buy if quote.to_string() == WSOL_MINT => TradeAmount::BuySol {
                lamports: 1_000_000,
            },
            Side::Buy => TradeAmount::BuyQuote {
                mint: c.route.quote_mint,
                units: 1_000_000,
            },
        };
        c
    }
    fn tx(&self, c: &TradeCommand) -> VersionedTransaction {
        let mint = Pubkey::new_from_array(c.route.mint.to_bytes());
        let quote = Pubkey::new_from_array(c.quote_mint().to_bytes());
        let (input, output) = if c.amount.side() == Side::Buy {
            (quote, mint)
        } else {
            (mint, quote)
        };
        let (iv, ov, ip, op) = if input == self.pool.token0_mint {
            (
                self.pool.token0_vault,
                self.pool.token1_vault,
                self.pool.token0_program,
                self.pool.token1_program,
            )
        } else {
            (
                self.pool.token1_vault,
                self.pool.token0_vault,
                self.pool.token1_program,
                self.pool.token0_program,
            )
        };
        let mut data = SWAP_BASE_IN_DISCRIMINATOR.to_vec();
        data.extend(1_000_000u64.to_le_bytes());
        data.extend(1u64.to_le_bytes());
        let accounts = vec![
            AccountMeta::new_readonly(key(1), true),
            AccountMeta::new_readonly(accounts::AUTHORITY, false),
            AccountMeta::new_readonly(self.pool.amm_config, false),
            AccountMeta::new(key(10), false),
            AccountMeta::new(key(70), false),
            AccountMeta::new(key(71), false),
            AccountMeta::new(iv, false),
            AccountMeta::new(ov, false),
            AccountMeta::new_readonly(ip, false),
            AccountMeta::new_readonly(op, false),
            AccountMeta::new_readonly(input, false),
            AccountMeta::new_readonly(output, false),
            AccountMeta::new(self.pool.observation_key, false),
        ];
        VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::Legacy(Message::new_with_blockhash(
                &[Instruction {
                    program_id: accounts::RAYDIUM_CPMM,
                    accounts,
                    data,
                }],
                Some(&key(1)),
                &Hash::new_from_array([8; 32]),
            )),
        }
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
            let call = {
                let mut calls = self.calls.lock().unwrap();
                calls.push((request, params.clone()));
                calls.len()
            };
            if self.fail == Some(call) {
                return Err(ClientErrorKind::Custom(
                    "RPC unavailable https://fixture.invalid/private?key=secret".into(),
                )
                .into());
            }
            match request {
                RpcRequest::GetMultipleAccounts => {
                    if let Some(gate) = &self.gate {
                        gate.entered.notify_one();
                        gate.release.notified().await;
                    }
                    let keys = params[0].as_array().unwrap();
                    let first = keys.len() == 1 && keys[0] == key(10).to_string();
                    let values = keys
                        .iter()
                        .map(|k| {
                            if first {
                                Ok(self.initial.clone())
                            } else {
                                self.values
                                    .get(k.as_str().unwrap())
                                    .cloned()
                                    .ok_or_else(|| {
                                        ClientError::from(ClientErrorKind::Custom(
                                            "unscripted read".into(),
                                        ))
                                    })
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(
                        json!({"context":{"slot":if first {self.first_slot}else{self.bank_slot}},"value":values}),
                    )
                }
                RpcRequest::SimulateTransaction => Ok(
                    json!({"context":{"slot":self.bank_slot},"value":{"err":null,"logs":[],"accounts":null,"unitsConsumed":10,"returnData":null,"innerInstructions":[],"replacementBlockhash":null}}),
                ),
                _ => Err(ClientErrorKind::Custom("forbidden fixture RPC".into()).into()),
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
        "memory://pumpapi-cpmm".into()
    }
}
fn fixture() -> ReadRpc {
    let pool = PoolState {
        amm_config: key(20),
        pool_creator: key(21),
        token0_mint: USDC_MINT.parse().unwrap(),
        token1_mint: WSOL_MINT.parse().unwrap(),
        token0_program: spl_token_interface::ID,
        token1_program: spl_token_interface::ID,
        token0_vault: key(30),
        token1_vault: key(31),
        lp_mint: key(40),
        observation_key: get_observation_state_pda(&key(10)).unwrap(),
        mint0_decimals: 6,
        mint1_decimals: 9,
        open_time: 900,
        ..Default::default()
    };
    let mut values = HashMap::new();
    values.insert(key(10).to_string(), pool_ui(&pool));
    values.insert(
        pool.token0_mint.to_string(),
        ui(pool.token0_program, mint_data(6)),
    );
    values.insert(
        pool.token1_mint.to_string(),
        ui(pool.token1_program, mint_data(9)),
    );
    values.insert(
        pool.token0_vault.to_string(),
        ui(
            pool.token0_program,
            vault_data(
                pool.token0_mint,
                accounts::AUTHORITY,
                AccountState::Initialized,
                1_000_000,
            ),
        ),
    );
    values.insert(
        pool.token1_vault.to_string(),
        ui(
            pool.token1_program,
            vault_data(
                pool.token1_mint,
                accounts::AUTHORITY,
                AccountState::Initialized,
                2_000_000,
            ),
        ),
    );
    let mut config = AMM_CONFIG_DISCRIMINATOR.to_vec();
    config.extend(
        bincode::serialize(&AmmConfig {
            trade_fee_rate: 2500,
            ..Default::default()
        })
        .unwrap(),
    );
    values.insert(
        pool.amm_config.to_string(),
        ui(accounts::RAYDIUM_CPMM, config),
    );
    values.insert(
        pool.observation_key.to_string(),
        ui(accounts::RAYDIUM_CPMM, vec![0; 64]),
    );
    values.insert(
        solana_sdk::sysvar::clock::id().to_string(),
        clock_ui(42, 17, 1000),
    );
    ReadRpc {
        initial: pool_ui(&pool),
        pool,
        values,
        first_slot: 41,
        bank_slot: 42,
        calls: Arc::new(Mutex::new(vec![])),
        fail: None,
        gate: None,
    }
}
async fn check(
    f: &ReadRpc,
    c: &TradeCommand,
    tx: &VersionedTransaction,
) -> anyhow::Result<Option<u64>> {
    let rpc = f.client();
    let view = resolve_unsigned_accounts(&rpc, owner(), tx).await?;
    check_cpmm_if_applicable(&rpc, owner(), c, tx, &view).await
}

#[tokio::test]
async fn pumpapi_h2b2b_cpmm_both_protocols_sides_and_orientations_use_two_bank_reads() {
    for kind in [Pool::RaydiumCpmm, Pool::StonkFunSwap] {
        for side in [Side::Buy, Side::Sell] {
            for target0 in [false, true] {
                let f = fixture();
                let mut c = f.input(side, target0);
                c.route.pool = kind;
                let tx = f.tx(&c);
                assert_eq!(check(&f, &c, &tx).await.unwrap(), Some(42));
                assert_eq!(f.methods(), vec![RpcRequest::GetMultipleAccounts; 2]);
                let calls = f.calls.lock().unwrap();
                assert_eq!(calls[0].1[0], json!([key(10).to_string()]));
                assert_eq!(calls[1].1[0].as_array().unwrap().len(), 8);
                assert_eq!(calls[1].1[1]["minContextSlot"], 41);
                assert_eq!(calls[1].1[1]["encoding"], "base64");
            }
        }
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_unsigned_pipeline_uses_verified_bank_slot_without_signing() {
    let f = fixture();
    let c = f.input(Side::Buy, true);
    let tx = f.tx(&c);
    let bytes = bincode::serialize(&tx).unwrap();
    let result = simulate_unsigned(&f.client(), owner(), tx, &c)
        .await
        .unwrap();
    assert!(result.success && result.signature.is_none());
    assert_eq!(
        f.methods(),
        [
            RpcRequest::GetMultipleAccounts,
            RpcRequest::GetMultipleAccounts,
            RpcRequest::SimulateTransaction
        ]
    );
    let calls = f.calls.lock().unwrap();
    assert_eq!(calls[2].1[1]["minContextSlot"], 42);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(calls[2].1[0].as_str().unwrap())
            .unwrap(),
        bytes
    );
}

#[tokio::test]
async fn pumpapi_h2b2b_other_protocols_are_explicitly_not_checked_without_rpc() {
    let f = fixture();
    let mut c = f.input(Side::Buy, true);
    c.route.pool = Pool::Pump;
    let tx = f.tx(&c);
    assert_eq!(check(&f, &c, &tx).await.unwrap(), None);
    assert!(f.methods().is_empty());
}

#[tokio::test]
async fn pumpapi_h2b2b_bad_message_roles_and_routed_commands_stop_before_bank_reads() {
    for mutation in 0..7 {
        let f = fixture();
        let mut c = f.input(Side::Buy, true);
        let mut tx = f.tx(&c);
        match mutation {
            0 => c.route.pool_id = None,
            1 => c.aggregator_sol = true,
            2 => {
                let VersionedMessage::Legacy(m) = &mut tx.message else {
                    unreachable!()
                };
                m.instructions[0].data[0] ^= 1;
            }
            3 => {
                let VersionedMessage::Legacy(m) = &mut tx.message else {
                    unreachable!()
                };
                m.instructions[0].accounts.pop();
            }
            4 => {
                let VersionedMessage::Legacy(m) = &mut tx.message else {
                    unreachable!()
                };
                m.instructions.push(m.instructions[0].clone());
            }
            5 => c.route.pool_id = Some(Address::from_bytes([99; 32])),
            6 => {
                let VersionedMessage::Legacy(m) = &mut tx.message else {
                    unreachable!()
                };
                m.instructions[0].data.pop();
            }
            _ => unreachable!(),
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert!(f.methods().is_empty());
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_invalid_pool_pair_owner_and_header_fail_on_first_read() {
    for mutation in 0..5 {
        let mut f = fixture();
        let c = f.input(Side::Buy, true);
        let tx = f.tx(&c);
        match mutation {
            0 => f.initial["owner"] = json!(key(99).to_string()),
            1 => f.initial = ui(accounts::RAYDIUM_CPMM, vec![0; 8 + POOL_STATE_SIZE]),
            2 => {
                f.pool.token0_mint = key(98);
                f.initial = pool_ui(&f.pool);
            }
            3 => {
                f.pool.token0_mint = f.pool.token1_mint;
                f.initial = pool_ui(&f.pool);
            }
            4 => f.initial = Value::Null,
            _ => unreachable!(),
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_pool_identity_drift_rejects_but_new_reserves_and_fees_are_allowed() {
    for change_identity in [false, true] {
        let mut f = fixture();
        let c = f.input(Side::Buy, true);
        let tx = f.tx(&c);
        if change_identity {
            f.pool.pool_creator = key(99);
        } else {
            f.pool.protocol_fees_token0 = 17;
            f.pool.lp_supply = 900;
        }
        f.update_pool();
        let result = check(&f, &c, &tx).await;
        assert_eq!(result.is_err(), change_identity);
        assert_eq!(f.methods(), vec![RpcRequest::GetMultipleAccounts; 2]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_closed_pool_old_bank_and_bad_clock_fail_without_simulation() {
    for mutation in 0..7 {
        let mut f = fixture();
        let c = f.input(Side::Buy, true);
        let tx = f.tx(&c);
        match mutation {
            0 => f.pool.status = SWAP_DISABLED,
            1 => f.pool.open_time = 1001,
            2 => f.pool.recent_epoch = 18,
            3 => f.bank_slot = 40,
            4 => {
                f.values.insert(
                    solana_sdk::sysvar::clock::id().to_string(),
                    clock_ui(41, 17, 1000),
                );
            }
            5 => {
                f.values.insert(
                    solana_sdk::sysvar::clock::id().to_string(),
                    clock_ui(42, 17, -1),
                );
            }
            6 => f.pool.creator_fee_on = 3,
            _ => unreachable!(),
        }
        f.update_pool();
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), vec![RpcRequest::GetMultipleAccounts; 2]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_wrong_mint_decimals_vault_authority_and_reserve_underflow_reject() {
    for mutation in 0..8 {
        let mut f = fixture();
        let c = f.input(Side::Buy, true);
        let tx = f.tx(&c);
        match mutation {
            0 => {
                f.values.get_mut(&f.pool.token0_mint.to_string()).unwrap()["owner"] =
                    json!(key(99).to_string());
            }
            1 => {
                f.values.insert(
                    f.pool.token0_mint.to_string(),
                    ui(f.pool.token0_program, mint_data(8)),
                );
            }
            2 => {
                f.values.insert(
                    f.pool.token0_vault.to_string(),
                    ui(
                        f.pool.token0_program,
                        vault_data(
                            key(98),
                            accounts::AUTHORITY,
                            AccountState::Initialized,
                            1_000_000,
                        ),
                    ),
                );
            }
            3 => {
                f.values.insert(
                    f.pool.token0_vault.to_string(),
                    ui(
                        f.pool.token0_program,
                        vault_data(
                            f.pool.token0_mint,
                            key(99),
                            AccountState::Initialized,
                            1_000_000,
                        ),
                    ),
                );
            }
            4 => {
                f.values.insert(
                    f.pool.token0_vault.to_string(),
                    ui(
                        f.pool.token0_program,
                        vault_data(
                            f.pool.token0_mint,
                            accounts::AUTHORITY,
                            AccountState::Frozen,
                            1_000_000,
                        ),
                    ),
                );
            }
            5 => f.pool.protocol_fees_token0 = 1_000_000,
            6 => {
                f.pool.protocol_fees_token0 = u64::MAX;
                f.pool.fund_fees_token0 = 1;
            }
            7 => {
                f.values
                    .insert(f.pool.token0_vault.to_string(), Value::Null);
            }
            _ => unreachable!(),
        }
        f.update_pool();
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), vec![RpcRequest::GetMultipleAccounts; 2]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_config_observation_and_unexpected_account_encoding_fail_closed() {
    for mutation in 0..6 {
        let mut f = fixture();
        let c = f.input(Side::Buy, true);
        let tx = f.tx(&c);
        match mutation {
            0 => {
                f.values.get_mut(&f.pool.amm_config.to_string()).unwrap()["owner"] =
                    json!(key(99).to_string());
            }
            1 => {
                let mut d = AMM_CONFIG_DISCRIMINATOR.to_vec();
                d.extend(
                    bincode::serialize(&AmmConfig {
                        trade_fee_rate: 1_000_001,
                        ..Default::default()
                    })
                    .unwrap(),
                );
                f.values
                    .insert(f.pool.amm_config.to_string(), ui(accounts::RAYDIUM_CPMM, d));
            }
            2 => {
                f.values
                    .get_mut(&f.pool.observation_key.to_string())
                    .unwrap()["owner"] = json!(key(99).to_string());
            }
            3 => {
                f.values.get_mut(&f.pool.token0_mint.to_string()).unwrap()["data"] =
                    json!(["not-zstd", "base64+zstd"]);
            }
            4 => {
                f.values.get_mut(&f.pool.token0_mint.to_string()).unwrap()["space"] = json!(999);
            }
            5 => {
                f.values.get_mut(&f.pool.token0_mint.to_string()).unwrap()["data"] =
                    json!(["A".repeat(MAX_ACCOUNT_BYTES.div_ceil(3) * 4 + 1), "base64"]);
            }
            _ => unreachable!(),
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), vec![RpcRequest::GetMultipleAccounts; 2]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_rpc_faults_redact_urls_and_do_not_retry_the_reads() {
    for fail in [1, 2] {
        let mut f = fixture();
        f.fail = Some(fail);
        let c = f.input(Side::Buy, true);
        let tx = f.tx(&c);
        let error = format!("{:#}", check(&f, &c, &tx).await.err().unwrap());
        assert!(error.contains("<redacted-url>"));
        assert!(!error.contains("secret"));
        assert_eq!(f.methods().len(), fail);
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_read_cancellation_does_not_continue_to_simulation() {
    let mut f = fixture();
    let gate = Arc::new(Gate::default());
    f.gate = Some(gate.clone());
    let c = f.input(Side::Buy, true);
    let tx = f.tx(&c);
    let rpc = f.client();
    let handle = tokio::spawn(async move { simulate_unsigned(&rpc, owner(), tx, &c).await });
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    handle.abort();
    assert!(handle.await.err().unwrap().is_cancelled());
    gate.release.notify_one();
    assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
}

fn token2022_mint(active: bool, unsupported: bool, bps: u16) -> Vec<u8> {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::{TransferFee, TransferFeeConfig},
            transfer_hook::TransferHook,
            BaseStateWithExtensionsMut, StateWithExtensionsMut,
        },
        state::Mint,
    };
    let mut extensions = vec![
        ExtensionType::TransferFeeConfig,
        ExtensionType::TransferHook,
    ];
    if unsupported {
        extensions.push(ExtensionType::NonTransferable);
    }
    let mut data = vec![0; ExtensionType::try_calculate_account_len::<Mint>(&extensions).unwrap()];
    let mut state = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut data).unwrap();
    let fee = state.init_extension::<TransferFeeConfig>(false).unwrap();
    fee.older_transfer_fee = TransferFee {
        epoch: 10u64.into(),
        maximum_fee: 5u64.into(),
        transfer_fee_basis_points: 100u16.into(),
    };
    fee.newer_transfer_fee = TransferFee {
        epoch: 20u64.into(),
        maximum_fee: 9u64.into(),
        transfer_fee_basis_points: bps.into(),
    };
    let hook = state.init_extension::<TransferHook>(false).unwrap();
    if active {
        hook.program_id = Some(key(99)).try_into().unwrap();
    }
    if unsupported {
        state.init_extension::<spl_token_2022_interface::extension::non_transferable::NonTransferable>(false).unwrap();
    }
    state.base = Mint {
        decimals: 6,
        is_initialized: true,
        supply: 1_000_000,
        ..Default::default()
    };
    state.pack_base();
    state.init_account_type().unwrap();
    data
}
fn token2022_vault(mint: Pubkey, transferring: bool) -> Vec<u8> {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::TransferFeeAmount, transfer_hook::TransferHookAccount,
            BaseStateWithExtensionsMut, StateWithExtensionsMut,
        },
        state::{Account, AccountState},
    };
    let mut data = vec![
        0;
        ExtensionType::try_calculate_account_len::<Account>(&[
            ExtensionType::TransferFeeAmount,
            ExtensionType::TransferHookAccount
        ])
        .unwrap()
    ];
    let mut state = StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut data).unwrap();
    state.init_extension::<TransferFeeAmount>(false).unwrap();
    state
        .init_extension::<TransferHookAccount>(false)
        .unwrap()
        .transferring = transferring.into();
    state.base = Account {
        mint,
        owner: accounts::AUTHORITY,
        amount: 1_000_000,
        state: AccountState::Initialized,
        ..Default::default()
    };
    state.pack_base();
    state.init_account_type().unwrap();
    data
}

#[test]
fn pumpapi_h2b2b_malformed_hook_payload_is_not_mistaken_for_disabled_hook() {
    let mut data = token2022_mint(false, false, 300);
    let mut offset = TokenAccount::LEN + 1;
    let mut damaged = false;
    while offset + 4 <= data.len() {
        let kind = u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap());
        let size = usize::from(u16::from_le_bytes(
            data[offset + 2..offset + 4].try_into().unwrap(),
        ));
        if kind == u16::from(ExtensionType::TransferHook) {
            data[offset + 2..offset + 4].copy_from_slice(&0u16.to_le_bytes());
            data.drain(offset + 4..offset + 4 + size);
            damaged = true;
            break;
        }
        offset += 4 + size;
    }
    assert!(damaged);
    let parsed =
        StateWithExtensions::<spl_token_2022_interface::state::Mint>::unpack(&data).unwrap();
    assert!(parsed
        .get_extension_types()
        .unwrap()
        .contains(&ExtensionType::TransferHook));
    assert!(spl_token_2022_interface::extension::transfer_hook::get_program_id(&parsed).is_none());
    let account: UiAccount =
        serde_json::from_value(ui(spl_token_2022_interface::ID, data)).unwrap();
    assert!(check_mint(&account, spl_token_2022_interface::ID, Some(6), 17, LABEL).is_err());
}

#[tokio::test]
async fn pumpapi_h2b2b_token2022_epoch_extensions_and_required_vault_state_are_checked() {
    for epoch in [17, 20] {
        for mutation in 0..6 {
            let mut f = fixture();
            f.pool.token0_mint = key(7);
            f.pool.token0_program = spl_token_2022_interface::ID;
            f.initial = pool_ui(&f.pool);
            f.update_pool();
            f.values.insert(
                key(7).to_string(),
                ui(
                    f.pool.token0_program,
                    token2022_mint(
                        mutation == 1,
                        mutation == 2,
                        if mutation == 5 { 10001 } else { 300 },
                    ),
                ),
            );
            f.values.insert(
                f.pool.token0_vault.to_string(),
                ui(
                    f.pool.token0_program,
                    if mutation == 3 {
                        vault_data(
                            key(7),
                            accounts::AUTHORITY,
                            AccountState::Initialized,
                            1_000_000,
                        )
                    } else {
                        token2022_vault(key(7), mutation == 4)
                    },
                ),
            );
            f.values.insert(
                solana_sdk::sysvar::clock::id().to_string(),
                clock_ui(42, epoch, 1000),
            );
            let c = f.input(Side::Buy, true);
            let tx = f.tx(&c);
            let result = check(&f, &c, &tx).await;
            assert_eq!(
                result.is_ok(),
                mutation == 0 || (mutation == 5 && epoch == 17),
                "epoch={epoch} mutation={mutation}"
            );
            assert_eq!(f.methods(), vec![RpcRequest::GetMultipleAccounts; 2]);
        }
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_custom_quote_precision_is_not_silently_reinterpreted() {
    for decimals in [6, 9] {
        let mut f = fixture();
        f.pool.token0_mint = key(7);
        f.pool.mint0_decimals = decimals;
        f.initial = pool_ui(&f.pool);
        f.update_pool();
        f.values.insert(
            key(7).to_string(),
            ui(f.pool.token0_program, mint_data(decimals)),
        );
        f.values.insert(
            f.pool.token0_vault.to_string(),
            ui(
                f.pool.token0_program,
                vault_data(
                    key(7),
                    accounts::AUTHORITY,
                    AccountState::Initialized,
                    1_000_000,
                ),
            ),
        );
        let c = f.input(Side::Buy, false);
        let tx = f.tx(&c);
        let result = check(&f, &c, &tx).await;
        assert_eq!(result.is_ok(), decimals == 9);
        if decimals == 6 {
            assert_eq!(
                result.err().unwrap().to_string(),
                "PumpAPI CPMM quote decimals do not match legacy request precision"
            );
        }
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_alt_then_pool_snapshot_preserves_message_and_minimum_context() {
    use solana_address_lookup_table_interface::{
        program,
        state::{AddressLookupTable, LookupTableMeta},
    };
    use solana_sdk::message::v0::{Message as V0Message, MessageAddressTableLookup};
    use std::borrow::Cow;
    for first_slot in [41, 42] {
        let mut f = fixture();
        f.first_slot = first_slot;
        let c = f.input(Side::Buy, true);
        let tx = f.tx(&c);
        let VersionedMessage::Legacy(mut m) = tx.message else {
            unreachable!()
        };
        let removed = m
            .account_keys
            .iter()
            .position(|key| *key == f.pool.token0_mint)
            .unwrap();
        m.account_keys.remove(removed);
        m.header.num_readonly_unsigned_accounts -= 1;
        let loaded = m.account_keys.len() as u8;
        for ix in &mut m.instructions {
            if usize::from(ix.program_id_index) > removed {
                ix.program_id_index -= 1;
            }
            for index in &mut ix.accounts {
                if usize::from(*index) == removed {
                    *index = loaded;
                } else if usize::from(*index) > removed {
                    *index -= 1;
                }
            }
        }
        let tx = VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::V0(V0Message {
                header: m.header,
                account_keys: m.account_keys,
                recent_blockhash: m.recent_blockhash,
                instructions: m.instructions,
                address_table_lookups: vec![MessageAddressTableLookup {
                    account_key: key(90),
                    writable_indexes: vec![],
                    readonly_indexes: vec![0],
                }],
            }),
        };
        let table = AddressLookupTable {
            meta: LookupTableMeta {
                last_extended_slot: 41,
                ..Default::default()
            },
            addresses: Cow::Owned(vec![f.pool.token0_mint]),
        }
        .serialize_for_tests()
        .unwrap();
        f.values
            .insert(key(90).to_string(), ui(program::id(), table));
        let bytes = bincode::serialize(&tx).unwrap();
        let result = simulate_unsigned(&f.client(), owner(), tx, &c).await;
        assert_eq!(result.is_ok(), first_slot == 42);
        let calls = f.calls.lock().unwrap();
        assert_eq!(calls[1].1[1]["minContextSlot"], 42);
        if first_slot == 42 {
            assert_eq!(calls.len(), 4);
            assert_eq!(calls[2].1[1]["minContextSlot"], 42);
            assert_eq!(calls[3].1[1]["minContextSlot"], 42);
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(calls[3].1[0].as_str().unwrap())
                    .unwrap(),
                bytes
            );
        } else {
            assert_eq!(calls.len(), 2);
        }
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_direction_and_account_privilege_changes_reject_even_with_valid_bank() {
    for mutation in 0..3 {
        let f = fixture();
        let mut c = f.input(Side::Buy, true);
        let mut tx = f.tx(&c);
        if mutation == 0 {
            c.amount = TradeAmount::SellPercent { percent: 100 };
        } else {
            let VersionedMessage::Legacy(m) = &mut tx.message else {
                unreachable!()
            };
            if mutation == 1 {
                m.instructions[0].accounts.swap(6, 7);
            } else {
                m.instructions[0].accounts[4] = m.instructions[0].accounts[6];
            }
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), vec![RpcRequest::GetMultipleAccounts; 2]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2b_entire_two_read_stage_has_a_finite_deadline() {
    let mut f = fixture();
    f.gate = Some(Arc::new(Gate::default()));
    let c = f.input(Side::Buy, true);
    let tx = f.tx(&c);
    let error = tokio::time::timeout(Duration::from_secs(6), check(&f, &c, &tx))
        .await
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "PumpAPI CPMM account check timed out");
    assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
}
