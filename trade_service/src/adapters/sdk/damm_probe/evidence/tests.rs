use super::*;
use base64::Engine;
use serde_json::{json, Value};
use sol_trade_sdk::{
    instruction::utils::meteora_damm_v2_types::{Pool, POOL_DISCRIMINATOR, POOL_SIZE},
    trading::core::params::MeteoraDammV2Params,
};
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_program_pack::Pack;
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Signature, transaction::Transaction};
use spl_token_interface::state::{Account as TokenAccount, AccountState, Mint};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

pub(crate) fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
pub(crate) fn ui(owner: Pubkey, data: Vec<u8>) -> Value {
    json!({"owner":owner.to_string(),"data":[base64::engine::general_purpose::STANDARD.encode(&data),"base64"],
        "lamports":10_000_000,"executable":false,"rentEpoch":0,"space":data.len()})
}
pub(crate) fn bytes(value: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value["data"][0].as_str().unwrap())
        .unwrap()
}
fn pool_ui(pool: &Pool) -> Value {
    let fees = &pool.pool_fees;
    let mut data = POOL_DISCRIMINATOR.to_vec();
    data.extend(
        bincode::serialize(&(
            &fees.base_fee,
            fees.protocol_fee_percent,
            fees.partner_fee_percent,
            fees.referral_fee_percent,
            fees.padding_0,
            &fees.dynamic_fee,
            fees.padding_1,
        ))
        .unwrap(),
    );
    macro_rules! append { ($($field:expr),* $(,)?) => { $(data.extend(bincode::serialize(&$field).unwrap());)* }; }
    append!(
        pool.token_a_mint,
        pool.token_b_mint,
        pool.token_a_vault,
        pool.token_b_vault,
        pool.whitelisted_vault,
        pool.partner,
        pool.liquidity,
        pool.padding,
        pool.protocol_a_fee,
        pool.protocol_b_fee,
        pool.partner_a_fee,
        pool.partner_b_fee,
        pool.sqrt_min_price,
        pool.sqrt_max_price,
        pool.sqrt_price,
        pool.activation_point,
        pool.activation_type,
        pool.pool_status,
        pool.token_a_flag,
        pool.token_b_flag,
        pool.collect_fee_mode,
        pool.pool_type,
        pool.padding_0,
        pool.fee_a_per_liquidity,
        pool.fee_b_per_liquidity,
        pool.permanent_lock_liquidity,
        pool.metrics,
        pool.padding_1,
        pool.reward_infos
    );
    assert_eq!(data.len(), 8 + POOL_SIZE);
    assert_eq!(pool_account_decode(&data).unwrap(), *pool);
    ui(accounts::METEORA_DAMM_V2, data)
}
pub(crate) fn mint(program: Pubkey, decimals: u8) -> Value {
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
    ui(program, data)
}
pub(crate) fn token(program: Pubkey, mint: Pubkey, owner: Pubkey, amount: u64) -> Value {
    let mut data = vec![0; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner,
            amount,
            state: AccountState::Initialized,
            ..Default::default()
        },
        &mut data,
    )
    .unwrap();
    ui(program, data)
}
pub(crate) fn change_token(value: &mut Value, change: impl FnOnce(&mut TokenAccount)) {
    let mut data = bytes(value);
    let mut state = TokenAccount::unpack(&data[..TokenAccount::LEN]).unwrap();
    change(&mut state);
    TokenAccount::pack(state, &mut data[..TokenAccount::LEN]).unwrap();
    let program = value["owner"].as_str().unwrap().parse().unwrap();
    *value = ui(program, data);
}
pub(crate) fn clock(slot: u64, epoch: u64) -> Value {
    ui(
        solana_sdk::sysvar::id(),
        bincode::serialize(&Clock {
            slot,
            epoch,
            unix_timestamp: 1000,
            ..Default::default()
        })
        .unwrap(),
    )
}

#[derive(Clone)]
pub(crate) struct ReadRpc {
    pub values: Vec<Value>,
    pub bank: u64,
    pub simulation_slot: u64,
    pub calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    pub fail: bool,
    pub fail_on: Option<RpcRequest>,
    pub simulation_value: Option<Value>,
    pub block: Option<RpcRequest>,
    pub dropped: Arc<AtomicBool>,
}
struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl ReadRpc {
    pub(crate) fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
    }
    pub(crate) fn methods(&self) -> Vec<RpcRequest> {
        self.calls.lock().unwrap().iter().map(|(r, _)| *r).collect()
    }
}
impl RpcSender for ReadRpc {
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
            self.calls.lock().unwrap().push((request, params));
            assert!(
                matches!(
                    request,
                    RpcRequest::GetMultipleAccounts | RpcRequest::SimulateTransaction
                ),
                "forbidden quote RPC: {request}"
            );
            if self.block == Some(request) {
                let _drop = DropFlag(self.dropped.clone());
                std::future::pending::<()>().await;
            }
            if self.fail || self.fail_on == Some(request) {
                return Err(ClientErrorKind::Custom(
                    "RPC https://fixture.invalid/?key=secret unavailable".into(),
                )
                .into());
            }
            Ok(match request {
                RpcRequest::GetMultipleAccounts => {
                    json!({"context":{"slot":self.bank},"value":self.values})
                }
                RpcRequest::SimulateTransaction => {
                    let value = self.simulation_value.clone().unwrap_or_else(
                        || json!({"err":null,"logs":[],"accounts":null,"unitsConsumed":123}),
                    );
                    json!({"context":{"slot":self.simulation_slot},"value":value})
                }
                _ => unreachable!(),
            })
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "memory://ordinary-damm-account-evidence".into()
    }
}

pub(crate) struct Fixture {
    pub rpc: ReadRpc,
    pub params: MeteoraDammV2Params,
    pub pool: Pool,
    pub quote: Pubkey,
}
impl Fixture {
    pub(crate) fn new(reversed: bool, token2022: bool, quote: Pubkey, output_exists: bool) -> Self {
        let target_program = if token2022 {
            spl_token_2022_interface::ID
        } else {
            spl_token_interface::ID
        };
        let (a, b, ap, bp) = if reversed {
            (quote, key(4), spl_token_interface::ID, target_program)
        } else {
            (key(4), quote, target_program, spl_token_interface::ID)
        };
        let params =
            MeteoraDammV2Params::new(key(1), key(2), key(3), a, b, ap, bp).with_swap_mode(0);
        let pool = Pool {
            token_a_mint: a,
            token_b_mint: b,
            token_a_vault: key(2),
            token_b_vault: key(3),
            liquidity: 1u128 << 96,
            sqrt_price: 1u128 << 64,
            ..Default::default()
        };
        let rpc = ReadRpc {
            values: vec![
                pool_ui(&pool),
                mint(ap, if a == WSOL_TOKEN_ACCOUNT { 9 } else { 6 }),
                mint(bp, if b == WSOL_TOKEN_ACCOUNT { 9 } else { 6 }),
                token(ap, a, accounts::AUTHORITY, 1_000_000),
                token(bp, b, accounts::AUTHORITY, 2_000_000),
                clock(42, 17),
                token(target_program, key(4), key(5), 1000),
                if output_exists {
                    token(spl_token_interface::ID, quote, key(5), 2000)
                } else {
                    Value::Null
                },
            ],
            bank: 42,
            simulation_slot: 42,
            calls: Arc::default(),
            fail: false,
            fail_on: None,
            simulation_value: None,
            block: None,
            dropped: Arc::default(),
        };
        Self {
            rpc,
            params,
            pool,
            quote,
        }
    }
    pub(crate) fn default() -> Self {
        Self::new(false, false, USDC_TOKEN_ACCOUNT, true)
    }
    pub(crate) fn prepare(&self) -> PreparedDammProbe {
        super::super::prepare_damm_probe(
            key(5),
            &self.params,
            key(4),
            self.quote,
            100,
            Hash::new_from_array([6; 32]),
            false,
        )
        .unwrap()
    }
    pub(crate) fn update_pool(&mut self) {
        self.rpc.values[0] = pool_ui(&self.pool);
    }
}

async fn failure(f: &Fixture) -> String {
    let prepared = f.prepare();
    let rpc = f.rpc.client();
    let error = inspect_prepared_damm_probe(&rpc, &prepared)
        .await
        .err()
        .expect("invalid evidence must fail")
        .to_string();
    assert_eq!(f.rpc.methods(), [RpcRequest::GetMultipleAccounts]);
    error
}

#[tokio::test]
async fn snapshot_covers_both_orientations_programs_quotes_and_optional_output_in_one_bank() {
    for reversed in [false, true] {
        for token2022 in [false, true] {
            for quote in [WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
                for exists in [false, true] {
                    let f = Fixture::new(reversed, token2022, quote, exists);
                    let prepared = f.prepare();
                    let rpc = f.rpc.client();
                    let evidence = inspect_prepared_damm_probe(&rpc, &prepared).await.unwrap();
                    assert_eq!(evidence.output_before(), if exists { 2000 } else { 0 });
                    let result = simulate_evidenced_damm_probe(&evidence).await.unwrap();
                    assert_eq!(result.context.slot, 42);
                    let calls = f.rpc.calls.lock().unwrap();
                    assert_eq!(calls.len(), 2);
                    assert_eq!(calls[0].0, RpcRequest::GetMultipleAccounts);
                    let intent = prepared.accounts();
                    assert_eq!(
                        calls[0].1[0],
                        json!([
                            f.params.pool.to_string(),
                            f.pool.token_a_mint.to_string(),
                            f.pool.token_b_mint.to_string(),
                            f.pool.token_a_vault.to_string(),
                            f.pool.token_b_vault.to_string(),
                            solana_sdk::sysvar::clock::id().to_string(),
                            intent.input_ata.to_string(),
                            intent.output_ata.to_string()
                        ])
                    );
                    assert_eq!(
                        calls[0].1[1],
                        json!({"encoding":"base64","commitment":"processed","dataSlice":null,"minContextSlot":null})
                    );
                    assert_eq!(calls[1].0, RpcRequest::SimulateTransaction);
                    assert_eq!(calls[1].1[1]["minContextSlot"], 42);
                    assert_eq!(calls[1].1[1]["sigVerify"], false);
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(calls[1].1[0].as_str().unwrap())
                        .unwrap();
                    let actual: Transaction = bincode::deserialize(&bytes).unwrap();
                    assert_eq!(&actual, prepared.transaction());
                    assert_eq!(actual.signatures, [Signature::default()]);
                }
            }
        }
    }
}

#[tokio::test]
async fn every_required_account_missing_fails_before_simulation() {
    for index in 0..7 {
        let mut f = Fixture::default();
        f.rpc.values[index] = Value::Null;
        assert!(failure(&f).await.contains("required account is missing"));
    }
}

#[tokio::test]
async fn response_count_slot_and_clock_inconsistency_fail_before_simulation() {
    for damage in 0..6 {
        let mut f = Fixture::default();
        match damage {
            0 => {
                f.rpc.values.pop();
            }
            1 => f.rpc.values.push(Value::Null),
            2 => f.rpc.bank = 0,
            3 => f.rpc.values[5] = clock(41, 17),
            4 => f.rpc.values[5]["owner"] = json!(key(99).to_string()),
            _ => f.rpc.values[5] = ui(solana_sdk::sysvar::id(), vec![0; 39]),
        }
        assert!(!failure(&f).await.is_empty());
    }
}

#[tokio::test]
async fn stale_pool_identity_and_malformed_pool_data_fail_closed() {
    for damage in 0..12 {
        let mut f = Fixture::default();
        match damage {
            0 => f.pool.token_a_mint = key(99),
            1 => f.pool.token_b_mint = key(99),
            2 => f.pool.token_a_vault = key(99),
            3 => f.pool.token_b_vault = key(99),
            _ => {}
        }
        f.update_pool();
        match damage {
            4 => f.rpc.values[0]["owner"] = json!(key(99).to_string()),
            5 => {
                let mut data = bytes(&f.rpc.values[0]);
                data[0] ^= 1;
                f.rpc.values[0] = ui(accounts::METEORA_DAMM_V2, data);
            }
            6 => f.rpc.values[0] = ui(accounts::METEORA_DAMM_V2, vec![0; 8 + POOL_SIZE - 1]),
            7 => f.rpc.values[0]["lamports"] = json!(0),
            8 => f.rpc.values[0]["executable"] = json!(true),
            9 => f.rpc.values[0]["space"] = json!(1),
            10 => f.rpc.values[0]["data"] = json!(["!", "base64"]),
            11 => f.rpc.values[0] = ui(accounts::METEORA_DAMM_V2, vec![0; MAX_ACCOUNT_BYTES + 1]),
            _ => {}
        }
        assert!(!failure(&f).await.is_empty());
    }
}

#[tokio::test]
async fn mint_owner_initialization_decimals_and_canonical_quote_program_are_checked() {
    for damage in 0..6 {
        let mut f = Fixture::default();
        match damage {
            0 => f.rpc.values[1]["owner"] = json!(key(99).to_string()),
            1 => f.rpc.values[2] = mint(spl_token_interface::ID, 9),
            2 => {
                let mut data = bytes(&f.rpc.values[1]);
                let mut m = Mint::unpack(&data).unwrap();
                m.is_initialized = false;
                Mint::pack(m, &mut data).unwrap();
                f.rpc.values[1] = ui(spl_token_interface::ID, data);
            }
            3 => f.rpc.values[1] = ui(spl_token_interface::ID, vec![0; 2]),
            4 => f.params.token_a_program = key(99),
            _ => {
                f.params.token_b_program = spl_token_2022_interface::ID;
                f.rpc.values[2]["owner"] = json!(spl_token_2022_interface::ID.to_string());
            }
        }
        assert!(!failure(&f).await.is_empty());
    }
}

#[tokio::test]
async fn vault_and_user_ata_program_mint_authority_frozen_delegate_close_state_are_checked() {
    for index in [3, 4, 6, 7] {
        for damage in 0..7 {
            let mut f = Fixture::default();
            if damage == 0 {
                f.rpc.values[index]["owner"] = json!(key(99).to_string());
            } else {
                change_token(&mut f.rpc.values[index], |account| match damage {
                    1 => account.mint = key(99),
                    2 => account.owner = key(99),
                    3 => account.state = AccountState::Frozen,
                    4 => account.delegate = Some(key(99)).into(),
                    5 => account.close_authority = Some(key(99)).into(),
                    _ => account.state = AccountState::Uninitialized,
                });
            }
            assert!(!failure(&f).await.is_empty());
        }
    }
}

#[tokio::test]
async fn insufficient_input_ata_balance_is_not_replaced_by_other_token_account_balance() {
    let mut f = Fixture::default();
    change_token(&mut f.rpc.values[6], |account| account.amount = 99);
    assert!(failure(&f)
        .await
        .contains("input ATA balance is insufficient"));
}

#[tokio::test]
async fn unaudited_referral_and_aliased_roles_are_rejected_without_any_rpc() {
    for referral in [false, true] {
        let mut f = Fixture::default();
        if referral {
            f.params.referral_token_account = Some(key(99));
        } else {
            f.params.token_a_vault = f.params.token_b_vault;
        }
        let prepared = f.prepare();
        let rpc = f.rpc.client();
        assert!(inspect_prepared_damm_probe(&rpc, &prepared).await.is_err());
        assert!(f.rpc.methods().is_empty());
    }
}

#[tokio::test]
async fn transport_failure_is_redacted_without_retry_or_simulation() {
    let mut f = Fixture::default();
    f.rpc.fail = true;
    let error = failure(&f).await;
    assert!(!error.contains("secret"));
}

#[tokio::test]
async fn simulation_cannot_regress_behind_the_bound_snapshot_slot() {
    let mut f = Fixture::default();
    f.rpc.simulation_slot = 41;
    let prepared = f.prepare();
    let rpc = f.rpc.client();
    let evidence = inspect_prepared_damm_probe(&rpc, &prepared).await.unwrap();
    assert!(simulate_evidenced_damm_probe(&evidence)
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("precedes account evidence"));
    assert_eq!(
        f.rpc.methods(),
        [
            RpcRequest::GetMultipleAccounts,
            RpcRequest::SimulateTransaction
        ]
    );
}

#[tokio::test]
async fn cancellation_at_snapshot_or_simulation_drops_only_the_local_readonly_future() {
    for request in [
        RpcRequest::GetMultipleAccounts,
        RpcRequest::SimulateTransaction,
    ] {
        let mut f = Fixture::default();
        f.rpc.block = Some(request);
        let prepared = f.prepare();
        let rpc = f.rpc.client();
        let result = tokio::time::timeout(Duration::from_millis(20), async {
            let evidence = inspect_prepared_damm_probe(&rpc, &prepared).await?;
            simulate_evidenced_damm_probe(&evidence).await
        })
        .await;
        assert!(result.is_err());
        assert!(f.rpc.dropped.load(Ordering::SeqCst));
        let expected = if request == RpcRequest::GetMultipleAccounts {
            vec![request]
        } else {
            vec![RpcRequest::GetMultipleAccounts, request]
        };
        assert_eq!(f.rpc.methods(), expected);
    }
}

#[tokio::test]
async fn account_snapshot_has_a_total_deadline_and_drops_a_stalled_rpc() {
    let mut f = Fixture::default();
    f.rpc.block = Some(RpcRequest::GetMultipleAccounts);
    let prepared = f.prepare();
    let rpc = f.rpc.client();
    let error = inspect_prepared_damm_probe(&rpc, &prepared)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("account evidence timed out"));
    assert!(f.rpc.dropped.load(Ordering::SeqCst));
    assert_eq!(f.rpc.methods(), [RpcRequest::GetMultipleAccounts]);
}

pub(crate) fn extended_mint(active_hook: bool, unsupported: bool, newer_bps: u16) -> Vec<u8> {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::{TransferFee, TransferFeeConfig},
            transfer_hook::TransferHook,
            BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut,
        },
        state::Mint,
    };
    let mut types = vec![
        ExtensionType::TransferFeeConfig,
        ExtensionType::TransferHook,
    ];
    if unsupported {
        types.push(ExtensionType::NonTransferable);
    }
    let mut data = vec![0; ExtensionType::try_calculate_account_len::<Mint>(&types).unwrap()];
    let mut mint = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut data).unwrap();
    let fee = mint.init_extension::<TransferFeeConfig>(false).unwrap();
    fee.older_transfer_fee = TransferFee {
        epoch: 10u64.into(),
        maximum_fee: 5u64.into(),
        transfer_fee_basis_points: 100u16.into(),
    };
    fee.newer_transfer_fee = TransferFee {
        epoch: 20u64.into(),
        maximum_fee: 9u64.into(),
        transfer_fee_basis_points: newer_bps.into(),
    };
    if active_hook {
        mint.init_extension::<TransferHook>(false)
            .unwrap()
            .program_id = Some(key(99)).try_into().unwrap();
    } else {
        mint.init_extension::<TransferHook>(false).unwrap();
    }
    if unsupported {
        mint.init_extension::<spl_token_2022_interface::extension::non_transferable::NonTransferable>(false).unwrap();
    }
    mint.base = Mint {
        decimals: 6,
        is_initialized: true,
        supply: 1_000_000,
        ..Default::default()
    };
    mint.pack_base();
    mint.init_account_type().unwrap();
    data
}
pub(crate) fn extended_token(
    mint: Pubkey,
    owner: Pubkey,
    transferring: bool,
    missing_required: bool,
) -> Vec<u8> {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::TransferFeeAmount, transfer_hook::TransferHookAccount,
            BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut,
        },
        state::{Account, AccountState},
    };
    let types = if missing_required {
        vec![ExtensionType::TransferHookAccount]
    } else {
        vec![
            ExtensionType::TransferFeeAmount,
            ExtensionType::TransferHookAccount,
        ]
    };
    let mut data = vec![0; ExtensionType::try_calculate_account_len::<Account>(&types).unwrap()];
    let mut token = StateWithExtensionsMut::<Account>::unpack_uninitialized(&mut data).unwrap();
    if !missing_required {
        token.init_extension::<TransferFeeAmount>(false).unwrap();
    }
    token
        .init_extension::<TransferHookAccount>(false)
        .unwrap()
        .transferring = transferring.into();
    token.base = Account {
        mint,
        owner,
        amount: 1000,
        state: AccountState::Initialized,
        ..Default::default()
    };
    token.pack_base();
    token.init_account_type().unwrap();
    data
}

#[tokio::test]
async fn token2022_epoch_hooks_and_required_vault_and_user_extensions_are_checked() {
    for reversed in [false, true] {
        for epoch in [17, 20] {
            for damage in 0..8 {
                let mut f = Fixture::new(reversed, true, USDC_TOKEN_ACCOUNT, true);
                let (mi, vi) = if reversed { (2, 4) } else { (1, 3) };
                f.rpc.values[mi] = ui(
                    spl_token_2022_interface::ID,
                    extended_mint(
                        damage == 1,
                        damage == 2,
                        if damage == 7 { 10001 } else { 300 },
                    ),
                );
                f.rpc.values[vi] = ui(
                    spl_token_2022_interface::ID,
                    extended_token(key(4), accounts::AUTHORITY, damage == 5, damage == 3),
                );
                f.rpc.values[6] = ui(
                    spl_token_2022_interface::ID,
                    extended_token(key(4), key(5), damage == 6, damage == 4),
                );
                f.rpc.values[5] = clock(42, epoch);
                let prepared = f.prepare();
                let rpc = f.rpc.client();
                let result = inspect_prepared_damm_probe(&rpc, &prepared).await;
                assert_eq!(
                    result.is_ok(),
                    damage == 0 || (damage == 7 && epoch == 17),
                    "reversed={reversed} epoch={epoch} damage={damage}"
                );
                assert_eq!(f.rpc.methods(), [RpcRequest::GetMultipleAccounts]);
            }
        }
    }
}

#[tokio::test]
async fn output_token2022_existing_or_new_ata_obeys_the_same_mint_extension_evidence() {
    for damage in 0..4 {
        let mut f = Fixture::new(false, true, USDC_TOKEN_ACCOUNT, true);
        f.rpc.values[1] = ui(
            spl_token_2022_interface::ID,
            extended_mint(false, false, 300),
        );
        f.rpc.values[3] = ui(
            spl_token_2022_interface::ID,
            extended_token(key(4), accounts::AUTHORITY, false, false),
        );
        f.rpc.values[6] = token(spl_token_interface::ID, USDC_TOKEN_ACCOUNT, key(5), 1000);
        f.rpc.values[7] = if damage == 1 {
            Value::Null
        } else {
            ui(
                spl_token_2022_interface::ID,
                extended_token(key(4), key(5), damage == 3, damage == 2),
            )
        };
        let prepared = super::super::prepare_damm_probe(
            key(5),
            &f.params,
            USDC_TOKEN_ACCOUNT,
            key(4),
            100,
            Hash::new_from_array([6; 32]),
            false,
        )
        .unwrap();
        let rpc = f.rpc.client();
        let result = inspect_prepared_damm_probe(&rpc, &prepared).await;
        assert_eq!(result.is_ok(), damage < 2);
        if let Ok(evidence) = result {
            assert_eq!(evidence.output_before(), if damage == 1 { 0 } else { 1000 });
        }
        assert_eq!(f.rpc.methods(), [RpcRequest::GetMultipleAccounts]);
    }
}
