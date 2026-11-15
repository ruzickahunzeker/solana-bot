//! Actual PumpSwap checks with public synthetic state, no SDK client, signer or socket.
use super::*;
use crate::{
    adapters::sdk::{
        pumpapi_accounts::resolve_unsigned_accounts, pumpapi_pool_accounts::check_pool_accounts,
        pumpapi_simulation::simulate_unsigned,
    },
    application::trade_command::test_support::command,
    domain::trade::{ExecutionMode, Pool, TradeAmount, WSOL_MINT},
};
use base64::Engine;
use serde_json::{json, Value};
use sol_trade_sdk::instruction::utils::pumpswap_types::{Pool as SwapPool, POOL_SIZE};
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
use spl_token_interface::state::{Account as TokenAccount, AccountState, Mint};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

fn k(n: u8) -> Pubkey {
    Pubkey::from([n; 32])
}
fn wallet() -> Address {
    Address::from_bytes([1; 32])
}
fn ui(owner: Pubkey, data: Vec<u8>) -> Value {
    json!({"owner":owner.to_string(),"lamports":10_000_000,"executable":false,"rentEpoch":0,"space":data.len(),"data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"]})
}
fn mint(decimals: u8) -> Vec<u8> {
    let mut d = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            decimals,
            is_initialized: true,
            supply: 1_000_000_000,
            ..Default::default()
        },
        &mut d,
    )
    .unwrap();
    d
}
fn vault(m: Pubkey, authority: Pubkey, amount: u64, state: AccountState) -> Vec<u8> {
    let mut d = vec![0; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint: m,
            owner: authority,
            amount,
            state,
            ..Default::default()
        },
        &mut d,
    )
    .unwrap();
    d
}
fn pool_data(p: &SwapPool) -> Vec<u8> {
    let body = bincode::serialize(&(
        p.pool_bump,
        p.index,
        p.creator,
        p.base_mint,
        p.quote_mint,
        p.lp_mint,
        p.pool_base_token_account,
        p.pool_quote_token_account,
        p.lp_supply,
        p.coin_creator,
        p.is_mayhem_mode,
        p.is_cashback_coin,
        p.virtual_quote_reserves,
        p.creator_fee_bps,
        p.can_edit_creator_fee,
        p.is_holder_reward,
    ))
    .unwrap();
    assert_eq!(body.len(), POOL_SIZE);
    let mut d = POOL_DISCRIMINATOR.to_vec();
    d.extend(body);
    d
}
fn global(protocol: Pubkey, reserved: Pubkey, extra: Pubkey) -> Vec<u8> {
    let mut d = vec![149, 8, 156, 202, 160, 252, 176, 217];
    d.extend(
        bincode::serialize(&(
            k(88),
            9u64,
            11u64,
            0u8,
            [protocol; 8],
            13u64,
            k(88),
            Pubkey::default(),
            reserved,
            true,
            [reserved; 7],
            true,
            [extra; 8],
            0u64,
            k(88),
            false,
        ))
        .unwrap(),
    );
    d.push(1);
    d
}
fn fees() -> Vec<u8> {
    let mut d = vec![143, 52, 146, 187, 219, 123, 76, 155];
    d.extend(
        bincode::serialize(&(
            0u8,
            k(88),
            2u64,
            3u64,
            5u64,
            1u32,
            0u128,
            31u64,
            37u64,
            41u64,
            0u32,
            0u64,
            0u64,
            0u64,
        ))
        .unwrap(),
    );
    d
}
#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}
#[derive(Clone)]
struct Fixture {
    pool: SwapPool,
    protocol: Pubkey,
    reserved: Pubkey,
    extra: Pubkey,
    values: HashMap<String, Value>,
    slot: u64,
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    fail: bool,
    gate: Option<Arc<Gate>>,
}
impl Fixture {
    fn new(mayhem: bool, cashback: bool, creator: bool) -> Self {
        let base: Pubkey = USDC_MINT.parse().unwrap();
        let quote: Pubkey = WSOL_MINT.parse().unwrap();
        let pool = SwapPool {
            creator: pumpswap::get_pump_pool_authority_pda(&base),
            base_mint: base,
            quote_mint: quote,
            lp_mint: k(40),
            pool_base_token_account: k(30),
            pool_quote_token_account: k(31),
            lp_supply: 1000,
            coin_creator: if creator { k(21) } else { Pubkey::default() },
            is_mayhem_mode: mayhem,
            is_cashback_coin: cashback,
            virtual_quote_reserves: 1000,
            creator_fee_bps: 37,
            ..Default::default()
        };
        let mut f = Self {
            pool,
            protocol: k(50),
            reserved: k(51),
            extra: k(52),
            values: HashMap::new(),
            slot: 42,
            calls: Arc::new(Mutex::new(vec![])),
            fail: false,
            gate: None,
        };
        f.update_pool();
        f.values
            .insert(base.to_string(), ui(spl_token_interface::ID, mint(6)));
        f.values
            .insert(quote.to_string(), ui(spl_token_interface::ID, mint(9)));
        f.values.insert(
            k(30).to_string(),
            ui(
                spl_token_interface::ID,
                vault(base, k(10), 1_000_000, AccountState::Initialized),
            ),
        );
        f.values.insert(
            k(31).to_string(),
            ui(
                spl_token_interface::ID,
                vault(quote, k(10), 2_000_000, AccountState::Initialized),
            ),
        );
        f.update_global();
        f.values.insert(
            accounts::FEE_CONFIG.to_string(),
            ui(accounts::FEE_PROGRAM, fees()),
        );
        f.values.insert(
            solana_sdk::sysvar::clock::id().to_string(),
            ui(
                solana_sdk::sysvar::id(),
                bincode::serialize(&Clock {
                    slot: 42,
                    epoch: 17,
                    unix_timestamp: 1000,
                    ..Default::default()
                })
                .unwrap(),
            ),
        );
        f
    }
    fn update_pool(&mut self) {
        self.values.insert(
            k(10).to_string(),
            ui(accounts::AMM_PROGRAM, pool_data(&self.pool)),
        );
    }
    fn update_global(&mut self) {
        self.values.insert(
            accounts::GLOBAL_ACCOUNT.to_string(),
            ui(
                accounts::AMM_PROGRAM,
                global(self.protocol, self.reserved, self.extra),
            ),
        );
    }
    fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
    }
    fn methods(&self) -> Vec<RpcRequest> {
        self.calls.lock().unwrap().iter().map(|(r, _)| *r).collect()
    }
    fn command(&self, side: Side, target_base: bool, hint: bool) -> TradeCommand {
        let mut c = command();
        c.route.pool = Pool::PumpAmm;
        c.route.pool_id = hint.then(|| Address::from_bytes(k(10).to_bytes()));
        c.execution.mode = ExecutionMode::Simulate;
        let (target, funding) = if target_base {
            (self.pool.base_mint, self.pool.quote_mint)
        } else {
            (self.pool.quote_mint, self.pool.base_mint)
        };
        c.route.mint = Address::from_bytes(target.to_bytes());
        c.route.quote_mint = Address::from_bytes(funding.to_bytes());
        c.amount = if side == Side::Sell {
            TradeAmount::SellPercent { percent: 100 }
        } else if funding.to_string() == WSOL_MINT {
            TradeAmount::BuySol {
                lamports: 1_000_000,
            }
        } else {
            TradeAmount::BuyQuote {
                mint: c.route.quote_mint,
                units: 1_000_000,
            }
        };
        c
    }
    fn tx(&self, c: &TradeCommand) -> VersionedTransaction {
        let input = if c.amount.side() == Side::Buy {
            pubkey(c.quote_mint())
        } else {
            pubkey(c.route.mint)
        };
        let buy = input == self.pool.quote_mint;
        let base_program: Pubkey = self.values[&self.pool.base_mint.to_string()]["owner"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let quote_program: Pubkey = self.values[&self.pool.quote_mint.to_string()]["owner"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let creator = Pubkey::find_program_address(
            &[b"creator_vault", self.pool.coin_creator.as_ref()],
            &accounts::AMM_PROGRAM,
        )
        .0;
        let recipient = if self.pool.is_mayhem_mode {
            self.reserved
        } else {
            self.protocol
        };
        let mut a = vec![
            AccountMeta::new(k(10), false),
            AccountMeta::new(k(1), true),
            AccountMeta::new_readonly(accounts::GLOBAL_ACCOUNT, false),
            AccountMeta::new_readonly(self.pool.base_mint, false),
            AccountMeta::new_readonly(self.pool.quote_mint, false),
            AccountMeta::new(k(70), false),
            AccountMeta::new(k(71), false),
            AccountMeta::new(self.pool.pool_base_token_account, false),
            AccountMeta::new(self.pool.pool_quote_token_account, false),
            AccountMeta::new_readonly(recipient, false),
            AccountMeta::new(ata(recipient, self.pool.quote_mint, quote_program), false),
            AccountMeta::new_readonly(base_program, false),
            AccountMeta::new_readonly(quote_program, false),
            AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            AccountMeta::new_readonly(accounts::ASSOCIATED_TOKEN_PROGRAM, false),
            AccountMeta::new_readonly(accounts::EVENT_AUTHORITY, false),
            AccountMeta::new_readonly(accounts::AMM_PROGRAM, false),
            AccountMeta::new(ata(creator, self.pool.quote_mint, quote_program), false),
            AccountMeta::new_readonly(creator, false),
        ];
        let volume = pumpswap::get_user_volume_accumulator_pda(&k(1)).unwrap();
        if buy {
            a.push(AccountMeta::new_readonly(
                accounts::GLOBAL_VOLUME_ACCUMULATOR,
                false,
            ));
            a.push(AccountMeta::new(volume, false));
        }
        a.push(AccountMeta::new_readonly(accounts::FEE_CONFIG, false));
        a.push(AccountMeta::new_readonly(accounts::FEE_PROGRAM, false));
        if self.pool.is_cashback_coin {
            a.push(AccountMeta::new(
                ata(volume, self.pool.quote_mint, quote_program),
                false,
            ));
            if !buy {
                a.push(AccountMeta::new(volume, false));
            }
        }
        if self.pool.coin_creator != Pubkey::default() {
            a.push(AccountMeta::new_readonly(
                pumpswap::get_pool_v2_pda(&self.pool.base_mint).unwrap(),
                false,
            ));
        }
        a.push(AccountMeta::new_readonly(self.extra, false));
        a.push(AccountMeta::new(
            ata(self.extra, self.pool.quote_mint, quote_program),
            false,
        ));
        let mut d = if buy {
            pumpswap::BUY_EXACT_QUOTE_IN_DISCRIMINATOR.to_vec()
        } else {
            pumpswap::SELL_DISCRIMINATOR.to_vec()
        };
        d.extend(1_000_000u64.to_le_bytes());
        d.extend(1u64.to_le_bytes());
        if buy {
            d.push(1);
        }
        VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::Legacy(Message::new_with_blockhash(
                &[Instruction {
                    program_id: accounts::AMM_PROGRAM,
                    accounts: a,
                    data: d,
                }],
                Some(&k(1)),
                &Hash::new_from_array([8; 32]),
            )),
        }
    }
}
impl RpcSender for Fixture {
    fn send<'a, 'b>(
        &'a self,
        r: RpcRequest,
        p: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ClientError>> + Send + 'b>>
    where
        'a: 'b,
        Self: 'b,
    {
        Box::pin(async move {
            self.calls.lock().unwrap().push((r, p.clone()));
            if self.fail {
                return Err(ClientErrorKind::Custom(
                    "RPC unavailable https://fixture.invalid/private?key=secret".into(),
                )
                .into());
            }
            match r {
                RpcRequest::GetMultipleAccounts => {
                    if let Some(gate) = &self.gate {
                        gate.entered.notify_one();
                        gate.release.notified().await;
                    }
                    let keys = p[0].as_array().unwrap();
                    let slot = if keys.len() == 1 && keys[0] == k(90).to_string() {
                        42
                    } else {
                        self.slot
                    };
                    let value = keys
                        .iter()
                        .map(|key| {
                            self.values
                                .get(key.as_str().unwrap())
                                .cloned()
                                .ok_or_else(|| {
                                    ClientError::from(ClientErrorKind::Custom(
                                        "unscripted account".into(),
                                    ))
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(json!({"context":{"slot":slot},"value":value}))
                }
                RpcRequest::SimulateTransaction => Ok(
                    json!({"context":{"slot":self.slot},"value":{"err":null,"logs":[],"accounts":null,"unitsConsumed":10,"returnData":null,"innerInstructions":[],"replacementBlockhash":null}}),
                ),
                _ => Err(ClientErrorKind::Custom("forbidden PumpSwap RPC".into()).into()),
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
        "memory://pumpapi-pumpswap".into()
    }
}
async fn check(
    f: &Fixture,
    c: &TradeCommand,
    tx: &VersionedTransaction,
) -> anyhow::Result<Option<u64>> {
    let rpc = f.client();
    let view = resolve_unsigned_accounts(&rpc, wallet(), tx).await?;
    check_pool_accounts(&rpc, wallet(), c, tx, &view).await
}

#[tokio::test]
async fn pumpapi_h2b2c_pumpswap_modes_orientations_and_optional_pool_hint_use_one_bank() {
    for mayhem in [false, true] {
        for cashback in [false, true] {
            for creator in [false, true] {
                for hint in [false, true] {
                    for target in [false, true] {
                        for side in [Side::Buy, Side::Sell] {
                            let f = Fixture::new(mayhem, cashback, creator);
                            let c = f.command(side, target, hint);
                            let tx = f.tx(&c);
                            assert_eq!(check(&f, &c, &tx).await.unwrap(), Some(42));
                            assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
                            assert_eq!(
                                f.calls.lock().unwrap()[0].1[0].as_array().unwrap().len(),
                                8
                            );
                        }
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_pumpswap_unsigned_simulation_preserves_bytes_and_bank_slot() {
    let f = Fixture::new(false, true, true);
    let c = f.command(Side::Buy, true, false);
    let tx = f.tx(&c);
    let bytes = bincode::serialize(&tx).unwrap();
    let result = simulate_unsigned(&f.client(), wallet(), tx, &c)
        .await
        .unwrap();
    assert!(result.success && result.signature.is_none());
    assert_eq!(
        f.methods(),
        [
            RpcRequest::GetMultipleAccounts,
            RpcRequest::SimulateTransaction
        ]
    );
    let calls = f.calls.lock().unwrap();
    assert_eq!(calls[1].1[1]["minContextSlot"], 42);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(calls[1].1[0].as_str().unwrap())
            .unwrap(),
        bytes
    );
}

#[tokio::test]
async fn pumpapi_h2b2c_bad_hint_opcode_and_multiple_swaps_stop_before_bank_read() {
    for mutation in 0..5 {
        let f = Fixture::new(false, false, false);
        let mut c = f.command(Side::Buy, true, true);
        let mut tx = f.tx(&c);
        match mutation {
            0 => c.route.pool_id = Some(Address::from_bytes([99; 32])),
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
                m.instructions.push(m.instructions[0].clone());
            }
            4 => {
                let VersionedMessage::Legacy(m) = &mut tx.message else {
                    unreachable!()
                };
                m.instructions[0].data[24] = 2;
            }
            _ => unreachable!(),
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert!(f.methods().is_empty());
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_bad_pool_layout_candidates_pair_and_owner_fail_closed() {
    for mutation in 0..5 {
        let mut f = Fixture::new(false, false, true);
        let mut c = f.command(Side::Buy, true, false);
        let tx = f.tx(&c);
        match mutation {
            0 => f.values.get_mut(&k(10).to_string()).unwrap()["owner"] = json!(k(99).to_string()),
            1 => {
                f.values
                    .insert(k(10).to_string(), ui(accounts::AMM_PROGRAM, vec![0; 300]));
            }
            2 => {
                f.pool.pool_base_token_account = k(99);
                f.update_pool();
            }
            3 => c.route.mint = Address::from_bytes([99; 32]),
            4 => {
                f.values.insert(k(30).to_string(), Value::Null);
            }
            _ => unreachable!(),
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_mint_vault_authority_frozen_and_program_mismatch_reject() {
    for mutation in 0..4 {
        let mut f = Fixture::new(false, false, true);
        let c = f.command(Side::Buy, true, true);
        let tx = f.tx(&c);
        match mutation {
            0 => {
                f.values.get_mut(&f.pool.base_mint.to_string()).unwrap()["owner"] =
                    json!(k(99).to_string())
            }
            1 => {
                f.values.insert(
                    k(30).to_string(),
                    ui(
                        spl_token_interface::ID,
                        vault(
                            f.pool.base_mint,
                            k(99),
                            1_000_000,
                            AccountState::Initialized,
                        ),
                    ),
                );
            }
            2 => {
                f.values.insert(
                    k(30).to_string(),
                    ui(
                        spl_token_interface::ID,
                        vault(f.pool.base_mint, k(10), 1_000_000, AccountState::Frozen),
                    ),
                );
            }
            3 => {
                f.values.get_mut(&k(30).to_string()).unwrap()["owner"] =
                    json!(spl_token_2022_interface::ID.to_string())
            }
            _ => unreachable!(),
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_global_rotation_is_read_from_the_same_bank_not_static_cache() {
    let mut f = Fixture::new(false, true, true);
    let c = f.command(Side::Buy, true, true);
    let old = f.tx(&c);
    f.protocol = k(54);
    f.extra = k(55);
    f.update_global();
    assert!(check(&f, &c, &old).await.is_err());
    let fresh = f.tx(&c);
    assert_eq!(check(&f, &c, &fresh).await.unwrap(), Some(42));
    assert_eq!(f.methods(), vec![RpcRequest::GetMultipleAccounts; 2]);
}

#[tokio::test]
async fn pumpapi_h2b2c_bad_config_disable_flags_and_reserves_are_not_fallbacks() {
    for mutation in 0..7 {
        let mut f = Fixture::new(false, false, true);
        let c = f.command(Side::Buy, true, true);
        let tx = f.tx(&c);
        match mutation {
            0 => {
                f.values
                    .get_mut(&accounts::GLOBAL_ACCOUNT.to_string())
                    .unwrap()["owner"] = json!(k(99).to_string())
            }
            1 => {
                let mut d = global(f.protocol, f.reserved, f.extra);
                d[56] = 1;
                f.values.insert(
                    accounts::GLOBAL_ACCOUNT.to_string(),
                    ui(accounts::AMM_PROGRAM, d),
                );
            }
            2 => {
                f.values
                    .insert(accounts::FEE_CONFIG.to_string(), Value::Null);
            }
            3 => {
                let mut d = global(f.protocol, f.reserved, f.extra);
                d[40..48].copy_from_slice(&10000u64.to_le_bytes());
                f.values.insert(
                    accounts::GLOBAL_ACCOUNT.to_string(),
                    ui(accounts::AMM_PROGRAM, d),
                );
            }
            4 => {
                f.values.insert(
                    k(30).to_string(),
                    ui(
                        spl_token_interface::ID,
                        vault(f.pool.base_mint, k(10), 0, AccountState::Initialized),
                    ),
                );
            }
            5 => {
                f.pool.virtual_quote_reserves = -2_000_001;
                f.update_pool();
            }
            6 => {
                f.pool.virtual_quote_reserves = i128::MAX;
                f.update_pool();
            }
            _ => unreachable!(),
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_direction_creator_cashback_and_remaining_roles_cannot_be_shifted() {
    for mutation in 0..5 {
        let f = Fixture::new(false, true, true);
        let mut c = f.command(Side::Buy, true, true);
        let mut tx = f.tx(&c);
        if mutation == 0 {
            c.amount = TradeAmount::SellPercent { percent: 100 };
        } else {
            let VersionedMessage::Legacy(m) = &mut tx.message else {
                unreachable!()
            };
            match mutation {
                1 => m.instructions[0].accounts.swap(7, 8),
                2 => m.instructions[0].accounts.swap(17, 18),
                3 => {
                    m.instructions[0].accounts.remove(23);
                }
                4 => {
                    m.instructions[0].accounts.pop();
                }
                _ => unreachable!(),
            }
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_alt_minimum_context_is_propagated_before_simulation() {
    use solana_address_lookup_table_interface::{
        program,
        state::{AddressLookupTable, LookupTableMeta},
    };
    use solana_sdk::message::v0::{Message as V0Message, MessageAddressTableLookup};
    use std::borrow::Cow;
    for bank in [41, 42] {
        let mut f = Fixture::new(false, false, true);
        f.slot = bank;
        let c = f.command(Side::Buy, true, true);
        let tx = f.tx(&c);
        let VersionedMessage::Legacy(mut m) = tx.message else {
            unreachable!()
        };
        let removed = m
            .account_keys
            .iter()
            .position(|key| *key == f.pool.base_mint)
            .unwrap();
        m.account_keys.remove(removed);
        m.header.num_readonly_unsigned_accounts -= 1;
        let loaded = m.account_keys.len() as u8;
        for ix in &mut m.instructions {
            if usize::from(ix.program_id_index) > removed {
                ix.program_id_index -= 1;
            }
            for i in &mut ix.accounts {
                if usize::from(*i) == removed {
                    *i = loaded;
                } else if usize::from(*i) > removed {
                    *i -= 1;
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
                    account_key: k(90),
                    writable_indexes: vec![],
                    readonly_indexes: vec![0],
                }],
            }),
        };
        let d = AddressLookupTable {
            meta: LookupTableMeta {
                last_extended_slot: 41,
                ..Default::default()
            },
            addresses: Cow::Owned(vec![f.pool.base_mint]),
        }
        .serialize_for_tests()
        .unwrap();
        f.values.insert(k(90).to_string(), ui(program::id(), d));
        let result = simulate_unsigned(&f.client(), wallet(), tx, &c).await;
        assert_eq!(result.is_ok(), bank == 42);
        let calls = f.calls.lock().unwrap();
        assert_eq!(calls[1].1[1]["minContextSlot"], 42);
        assert_eq!(calls.len(), if bank == 42 { 3 } else { 2 });
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_rpc_error_is_redacted_without_retry_or_signature() {
    let mut f = Fixture::new(false, false, true);
    f.fail = true;
    let c = f.command(Side::Buy, true, true);
    let tx = f.tx(&c);
    let error = format!("{:#}", check(&f, &c, &tx).await.err().unwrap());
    assert!(error.contains("<redacted-url>"));
    assert!(!error.contains("secret"));
    assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
}

#[tokio::test]
async fn pumpapi_h2b2c_cancellation_and_timeout_do_not_continue_to_simulation() {
    for cancel in [false, true] {
        let mut f = Fixture::new(false, false, true);
        let gate = Arc::new(Gate::default());
        f.gate = Some(gate.clone());
        let c = f.command(Side::Buy, true, true);
        let tx = f.tx(&c);
        let rpc = f.client();
        let handle = tokio::spawn(async move { simulate_unsigned(&rpc, wallet(), tx, &c).await });
        tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
            .await
            .unwrap();
        if cancel {
            handle.abort();
            assert!(handle.await.err().unwrap().is_cancelled());
        } else {
            let error = tokio::time::timeout(Duration::from_secs(6), handle)
                .await
                .unwrap()
                .unwrap()
                .err()
                .unwrap();
            assert_eq!(
                error.to_string(),
                "PumpAPI PumpSwap account check timed out"
            );
        }
        gate.release.notify_one();
        assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_token2022_hook_and_required_vault_extensions_use_shared_checks() {
    use spl_token_2022_interface::{
        extension::{
            transfer_hook::{TransferHook, TransferHookAccount},
            BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut,
        },
        state::{Account as Account22, AccountState as State22, Mint as Mint22},
    };
    for damage in 0..4 {
        let mut f = Fixture::new(false, true, true);
        f.pool.base_mint = k(7);
        f.pool.creator = pumpswap::get_pump_pool_authority_pda(&k(7));
        f.update_pool();
        let mut d =
            vec![
                0;
                ExtensionType::try_calculate_account_len::<Mint22>(&[ExtensionType::TransferHook])
                    .unwrap()
            ];
        let mut mint = StateWithExtensionsMut::<Mint22>::unpack_uninitialized(&mut d).unwrap();
        let hook = mint.init_extension::<TransferHook>(false).unwrap();
        if damage == 1 {
            hook.program_id = Some(k(99)).try_into().unwrap();
        }
        mint.base = Mint22 {
            decimals: 6,
            supply: 1_000_000_000,
            is_initialized: true,
            ..Default::default()
        };
        mint.pack_base();
        mint.init_account_type().unwrap();
        f.values
            .insert(k(7).to_string(), ui(spl_token_2022_interface::ID, d));
        let mut d = vec![
            0;
            ExtensionType::try_calculate_account_len::<Account22>(&[
                ExtensionType::TransferHookAccount
            ])
            .unwrap()
        ];
        let mut v = StateWithExtensionsMut::<Account22>::unpack_uninitialized(&mut d).unwrap();
        v.init_extension::<TransferHookAccount>(false).unwrap();
        v.base = Account22 {
            mint: k(7),
            owner: k(10),
            amount: 1_000_000,
            state: State22::Initialized,
            ..Default::default()
        };
        v.pack_base();
        v.init_account_type().unwrap();
        if damage == 2 {
            d = vault(k(7), k(10), 1_000_000, AccountState::Initialized);
        }
        if damage == 3 {
            d.extend(u16::from(ExtensionType::TransferHookAccount).to_le_bytes());
            d.extend(1u16.to_le_bytes());
            d.push(0);
        }
        f.values
            .insert(k(30).to_string(), ui(spl_token_2022_interface::ID, d));
        let c = f.command(Side::Buy, true, true);
        let tx = f.tx(&c);
        let result = check(&f, &c, &tx).await;
        assert_eq!(result.is_ok(), damage == 0);
        if damage == 3 {
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("duplicate extensions"));
        }
        assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_custom_quote_precision_is_checked_against_funding_mint() {
    for decimals in [6, 9] {
        let mut f = Fixture::new(false, false, true);
        f.pool.base_mint = k(7);
        f.update_pool();
        f.values.insert(
            k(7).to_string(),
            ui(spl_token_interface::ID, mint(decimals)),
        );
        f.values.insert(
            k(30).to_string(),
            ui(
                spl_token_interface::ID,
                vault(k(7), k(10), 1_000_000, AccountState::Initialized),
            ),
        );
        let c = f.command(Side::Buy, false, true);
        let tx = f.tx(&c);
        let result = check(&f, &c, &tx).await;
        assert_eq!(result.is_ok(), decimals == 9);
        if decimals == 6 {
            assert_eq!(
                result.err().unwrap().to_string(),
                "PumpAPI PumpSwap quote decimals do not match legacy request precision"
            );
        }
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_bad_tiers_and_clock_context_are_rejected_without_defaults() {
    for damage in 0..4 {
        let mut f = Fixture::new(false, false, true);
        let c = f.command(Side::Buy, true, true);
        let tx = f.tx(&c);
        if damage < 3 {
            let rows = match damage {
                0 => vec![],
                1 => vec![(0u128, 1u64, 2u64, 3u64), (0, 2, 3, 4)],
                _ => vec![(0, u64::MAX, 2, 3)],
            };
            let mut d = vec![143, 52, 146, 187, 219, 123, 76, 155];
            d.extend(
                bincode::serialize(&(0u8, k(88), 2u64, 3u64, 5u64, rows.len() as u32)).unwrap(),
            );
            for row in rows {
                d.extend(bincode::serialize(&row).unwrap());
            }
            d.extend(bincode::serialize(&(0u32, 0u64, 0u64, 0u64)).unwrap());
            f.values.insert(
                accounts::FEE_CONFIG.to_string(),
                ui(accounts::FEE_PROGRAM, d),
            );
        } else {
            f.values.insert(
                solana_sdk::sysvar::clock::id().to_string(),
                ui(
                    solana_sdk::sysvar::id(),
                    bincode::serialize(&Clock {
                        slot: 41,
                        epoch: 17,
                        unix_timestamp: 1000,
                        ..Default::default()
                    })
                    .unwrap(),
                ),
            );
        }
        assert!(check(&f, &c, &tx).await.is_err());
        assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}

#[tokio::test]
async fn pumpapi_h2b2c_legacy_pool_and_both_buy_opcodes_retain_sdk_semantics() {
    use sol_trade_sdk::instruction::utils::pumpswap_types::LEGACY_POOL_SIZE;
    for legacy in [false, true] {
        for exact_output in [false, true] {
            let mut f = Fixture::new(false, false, true);
            if legacy {
                let p = &f.pool;
                let mut body = bincode::serialize(&(
                    p.pool_bump,
                    p.index,
                    p.creator,
                    p.base_mint,
                    p.quote_mint,
                    p.lp_mint,
                    p.pool_base_token_account,
                    p.pool_quote_token_account,
                    p.lp_supply,
                    p.coin_creator,
                    p.is_mayhem_mode,
                    p.is_cashback_coin,
                ))
                .unwrap();
                body.resize(LEGACY_POOL_SIZE, 0);
                let mut d = POOL_DISCRIMINATOR.to_vec();
                d.extend(body);
                f.values
                    .insert(k(10).to_string(), ui(accounts::AMM_PROGRAM, d));
            }
            let c = f.command(Side::Buy, true, true);
            let mut tx = f.tx(&c);
            if exact_output {
                let VersionedMessage::Legacy(m) = &mut tx.message else {
                    unreachable!()
                };
                m.instructions[0].data[..8].copy_from_slice(&pumpswap::BUY_DISCRIMINATOR);
            }
            assert_eq!(check(&f, &c, &tx).await.unwrap(), Some(42));
            assert_eq!(f.methods(), [RpcRequest::GetMultipleAccounts]);
        }
    }
}
