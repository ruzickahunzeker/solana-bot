use super::*;
use crate::{
    adapters::sdk::ordinary_params::ordinary_sell_params,
    domain::preparation_balance::{PreparationBalanceSnapshot, PreparationInput},
};
use sol_trade_sdk::{trading::core::params::MeteoraDammV2Params, TradeTokenType};
use solana_sdk::hash::Hash;
use std::time::Instant;

pub(super) fn key(value: u8) -> Pubkey {
    Pubkey::new_from_array([value; 32])
}

pub(super) struct Fixture {
    pub context: DammSellContext,
    pub intent: ExecuteTrade,
    pub request: TradeSellParams,
    pub balance: u64,
    pub percent: u64,
    pub holding: Arc<PreparationBalanceSnapshot>,
    pub quote: Arc<crate::adapters::sdk::damm_probe::DammOutputEvidence>,
}

impl Fixture {
    pub fn profile(&self) -> Result<Arc<DammSellMessageProfile>> {
        DammSellMessageProfile::new(
            self.context,
            &self.intent,
            &self.request,
            self.balance,
            self.percent,
            self.holding.clone(),
            self.quote.clone(),
        )
    }
}

pub(super) fn case(
    provider: SwqosType,
    simulate: bool,
    percent: u64,
    quote: Pubkey,
    reversed: bool,
    program: Pubkey,
) -> Fixture {
    let context = DammSellContext {
        wallet: key(1),
        provider,
        version: TradeTransactionVersion::V0,
        seed_optimized: false,
    };
    let intent = ExecuteTrade {
        pool: Pool::MeteoraDammV2,
        mint: key(2),
        quote_mint: quote,
        pool_id: Some(key(3)),
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: None,
        sell_percent: Some(percent),
        slippage_bps: 1_500,
        tip_sol: 0.001,
        recent_blockhash: Hash::new_from_array([9; 32]),
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 250,
        simulate,
        use_mev: Some(true),
        mev_provider: None,
    };
    let (a, b, ap, bp) = if reversed {
        (quote, intent.mint, spl_token_interface::ID, program)
    } else {
        (intent.mint, quote, program, spl_token_interface::ID)
    };
    let params = MeteoraDammV2Params::new(key(3), key(4), key(5), a, b, ap, bp).with_swap_mode(0);
    let request = ordinary_sell_params(
        &intent,
        DexParamEnum::MeteoraDammV2(params),
        Some(777),
        percent,
        provider,
    )
    .unwrap();
    let balance = 10_000;
    let account = sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
        &context.wallet,
        &intent.mint,
        &program,
    );
    let holding = Arc::new(
        PreparationBalanceSnapshot::from_account(
            Address::from_bytes(context.wallet.to_bytes()),
            Address::from_bytes(intent.mint.to_bytes()),
            balance,
            PreparationInput {
                account: Address::from_bytes(account.to_bytes()),
                program: Address::from_bytes(program.to_bytes()),
                decimals: 6,
            },
            123,
            Instant::now(),
        )
        .unwrap(),
    );
    let quote = crate::adapters::sdk::damm_probe::DammOutputEvidence::fixture_for_profile(
        context.wallet,
        &intent,
        &request,
        balance,
        percent,
        &holding,
    );
    Fixture {
        context,
        intent,
        request,
        balance,
        percent,
        holding,
        quote,
    }
}

pub(super) fn fixture(provider: SwqosType, simulate: bool, percent: u64) -> Fixture {
    case(
        provider,
        simulate,
        percent,
        WSOL_TOKEN_ACCOUNT,
        false,
        spl_token_interface::ID,
    )
}

#[test]
fn damm_sell_message_rejects_a_rebuilt_nominal_profile_with_weakened_evidenced_minimum() {
    let mut f = fixture(SwqosType::Default, false, 50);
    f.request.fixed_output_token_amount = Some(1);
    let error = f.profile().err().unwrap();
    assert!(
        error.to_string().contains("minimum output differs"),
        "{error:#}"
    );
    assert!(!error
        .downcast_ref::<crate::domain::preparation_balance::PreparationBalanceFailure>()
        .unwrap()
        .allows_lane_fallback());
}

#[test]
fn damm_sell_message_cannot_rebuild_a_consistent_but_different_route_or_blockhash_over_old_quote() {
    for mutation in 0..4 {
        let mut f = fixture(SwqosType::Default, false, 50);
        match mutation {
            0 => {
                let DexParamEnum::MeteoraDammV2(p) = &mut f.request.extension_params else {
                    unreachable!()
                };
                p.token_a_vault = key(30);
            }
            1 => {
                f.intent.recent_blockhash = Hash::new_from_array([30; 32]);
                f.request.recent_blockhash = Some(f.intent.recent_blockhash);
            }
            2 => {
                let DexParamEnum::MeteoraDammV2(p) = &mut f.request.extension_params else {
                    unreachable!()
                };
                p.pool = key(30);
                f.intent.pool_id = Some(p.pool);
            }
            3 => {
                let DexParamEnum::MeteoraDammV2(p) = &mut f.request.extension_params else {
                    unreachable!()
                };
                p.include_rate_limiter_sysvar = !p.include_rate_limiter_sysvar;
            }
            _ => unreachable!(),
        }
        let error = f.profile().err().unwrap();
        assert!(
            error.to_string().contains("actual output quote context"),
            "{mutation}: {error:#}"
        );
    }
}

#[test]
fn damm_sell_message_binds_raw_percentage_input_to_the_original_quote_not_only_new_balance() {
    let mut f = fixture(SwqosType::Default, false, 50);
    let wallet = Address::from_bytes(f.context.wallet.to_bytes());
    let mint = Address::from_bytes(f.intent.mint.to_bytes());
    let input = f.holding.checked_input(wallet, mint).unwrap();
    f.balance = 9000;
    f.holding = Arc::new(
        PreparationBalanceSnapshot::from_account(
            wallet,
            mint,
            f.balance,
            input,
            123,
            Instant::now(),
        )
        .unwrap(),
    );
    assert!(f.profile().is_err());
}

#[test]
fn damm_sell_message_profiles_preserve_direction_program_percentage_and_simulation_matrix() {
    let mut count = 0;
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        for simulate in [false, true] {
            for percent in [1, 50, 100] {
                for quote in [WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
                    for reversed in [false, true] {
                        for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                            let fixture =
                                case(provider, simulate, percent, quote, reversed, program);
                            let profile = fixture.profile().unwrap();
                            profile
                                .validate_request(
                                    fixture.context,
                                    &fixture.request,
                                    fixture.balance,
                                    percent,
                                )
                                .unwrap();
                            let swap = &profile.protocol[1];
                            assert_eq!(&swap.data[8..16], &(10_000 * percent / 100).to_le_bytes());
                            assert_eq!(&swap.data[16..24], &777u64.to_le_bytes());
                            assert_eq!(swap.data[24], 0);
                            assert_eq!(profile.protocol.len(), if percent == 100 { 3 } else { 2 });
                            assert_eq!(swap.accounts[8].pubkey, fixture.context.wallet);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 96);
}

#[test]
fn damm_sell_message_request_mutations_are_rejected_without_refreshing_profile() {
    for change in 0..23 {
        let mut f = fixture(SwqosType::Temporal, false, 100);
        let profile = f.profile().unwrap();
        match change {
            0 => f.request.mint = key(30),
            1 => f.request.output_token_type = TradeTokenType::USDC,
            2 => f.request.input_token_amount = 1,
            3 => f.request.slippage_basis_points = Some(1),
            4 => f.request.recent_blockhash = Some(Hash::new_from_array([8; 32])),
            5 => f.request.wait_tx_confirmed = true,
            6 => f.request.wait_for_all_submits = true,
            7 => f.request.with_tip = false,
            8 => f.request.create_output_token_ata = false,
            9 => f.request.close_output_token_ata = true,
            10 => f.request.close_mint_token_ata = false,
            11 => f.request.fixed_output_token_amount = Some(1),
            12 => f.request.simulate = true,
            13 => f.request.grpc_recv_us = Some(123),
            14 => f.request.dex_type = DexType::PumpFun,
            15 => f.request.address_lookup_table_accounts.push(
                solana_sdk::message::AddressLookupTableAccount {
                    key: key(31),
                    addresses: vec![key(4)],
                },
            ),
            16 => f.request.gas_fee_strategy.update_sell_tip(0.002),
            17 => f.request.gas_fee_strategy.update_sell_cu_price(2_000),
            18..=21 => {
                let DexParamEnum::MeteoraDammV2(pool) = &mut f.request.extension_params else {
                    unreachable!()
                };
                match change {
                    18 => pool.pool = key(31),
                    19 => pool.token_a_vault = key(31),
                    20 => pool.referral_token_account = Some(key(31)),
                    21 => pool.include_rate_limiter_sysvar = true,
                    _ => unreachable!(),
                }
            }
            22 => {
                f.request.durable_nonce =
                    Some(sol_trade_sdk::common::nonce_cache::DurableNonceInfo {
                        nonce_account: Some(key(30)),
                        current_nonce: Some(Hash::new_from_array([30; 32])),
                    })
            }
            _ => unreachable!(),
        }
        assert!(
            profile
                .validate_request(f.context, &f.request, f.balance, f.percent)
                .is_err(),
            "change {change}"
        );
    }
}

#[test]
fn damm_sell_message_profile_binds_complete_execution_intent() {
    let f = fixture(SwqosType::Temporal, false, 50);
    let profile = f.profile().unwrap();
    profile.validate_intent(&f.intent).unwrap();
    for change in 0..16 {
        let mut intent = f.intent.clone();
        match change {
            0 => intent.pool = Pool::MeteoraDbc,
            1 => intent.mint = key(30),
            2 => intent.quote_mint = USDC_TOKEN_ACCOUNT,
            3 => intent.pool_id = None,
            4 => intent.sol_hop_pool = Some(Pool::PumpAmm),
            5 => intent.sol_hop_pool_id = Some(key(30)),
            6 => intent.amount_lamports = Some(1),
            7 => intent.sell_percent = Some(100),
            8 => intent.slippage_bps = 1,
            9 => intent.tip_sol = 0.002,
            10 => intent.recent_blockhash = Hash::new_from_array([30; 32]),
            11 => intent.wait_tx_confirmed = true,
            12 => intent.mev_submit_verify_ms = 0,
            13 => intent.simulate = true,
            14 => intent.use_mev = Some(false),
            15 => intent.mev_provider = Some(crate::domain::trade::MevProvider::Jito),
            _ => unreachable!(),
        }
        assert!(profile.validate_intent(&intent).is_err(), "change {change}");
    }
}

#[test]
fn damm_sell_message_context_balance_and_percentage_are_bound() {
    let f = fixture(SwqosType::Temporal, false, 50);
    let profile = f.profile().unwrap();
    for change in 0..4 {
        let mut context = f.context;
        match change {
            0 => context.wallet = key(30),
            1 => context.provider = SwqosType::Jito,
            2 => context.version = TradeTransactionVersion::V1,
            3 => context.seed_optimized = true,
            _ => unreachable!(),
        }
        assert!(profile
            .validate_request(context, &f.request, f.balance, f.percent)
            .is_err());
    }
    assert!(profile
        .validate_request(f.context, &f.request, f.balance + 1, f.percent)
        .is_err());
    assert!(profile
        .validate_request(f.context, &f.request, f.balance, 100)
        .is_err());
}

#[test]
fn damm_sell_message_zero_minimum_default_hash_and_unsupported_modes_are_not_profiles() {
    for change in 0..8 {
        let mut f = fixture(SwqosType::Temporal, false, 50);
        match change {
            0 => f.request.fixed_output_token_amount = Some(0),
            1 => f.request.fixed_output_token_amount = None,
            2 => {
                f.intent.recent_blockhash = Hash::default();
                f.request.recent_blockhash = Some(Hash::default());
            }
            3 => f.context.seed_optimized = true,
            4 => f.context.version = TradeTransactionVersion::V1,
            5 => f.intent.pool = Pool::MeteoraDbc,
            6 => f.intent.sol_hop_pool_id = Some(key(30)),
            7 => {
                let DexParamEnum::MeteoraDammV2(pool) = &mut f.request.extension_params else {
                    unreachable!()
                };
                pool.swap_mode = 1;
            }
            _ => unreachable!(),
        }
        assert!(f.profile().is_err(), "change {change}");
    }
}

#[test]
fn damm_sell_message_fee_clones_cannot_change_frozen_budgets_or_add_routes() {
    for change in 0..4 {
        let f = fixture(SwqosType::Temporal, false, 50);
        let profile = f.profile().unwrap();
        let fees = f.request.gas_fee_strategy.clone();
        match change {
            0 => fees.clear(),
            1 => fees.set(
                SwqosType::Temporal,
                TradeType::Sell,
                GasFeeStrategyType::Normal,
                300_001,
                1_000,
                0.001,
            ),
            2 => fees.set(
                SwqosType::Jito,
                TradeType::Sell,
                GasFeeStrategyType::Normal,
                300_000,
                1_000,
                0.001,
            ),
            3 => fees.set(
                SwqosType::Temporal,
                TradeType::Sell,
                GasFeeStrategyType::HighTipLowCuPrice,
                300_000,
                1_000,
                0.001,
            ),
            _ => unreachable!(),
        }
        assert!(profile
            .validate_request(f.context, &f.request, f.balance, f.percent)
            .is_err());
    }
}

#[test]
fn damm_sell_message_pool_output_program_and_all_account_fields_are_frozen() {
    for change in 0..8 {
        let mut f = fixture(SwqosType::Temporal, false, 50);
        let profile = f.profile().unwrap();
        let DexParamEnum::MeteoraDammV2(pool) = &mut f.request.extension_params else {
            unreachable!()
        };
        match change {
            0 => pool.token_a_mint = key(30),
            1 => pool.token_b_mint = USDC_TOKEN_ACCOUNT,
            2 => pool.token_a_program = spl_token_2022_interface::ID,
            3 => pool.token_b_program = spl_token_2022_interface::ID,
            4 => pool.token_b_vault = key(30),
            5 => pool.swap_mode = 2,
            6 => pool.token_a_vault = pool.token_b_vault,
            7 => pool.token_b_program = key(30),
            _ => unreachable!(),
        }
        assert!(profile
            .validate_request(f.context, &f.request, f.balance, f.percent)
            .is_err());
    }
}

#[test]
fn damm_sell_message_public_profile_never_unwraps_output_or_closes_partial_input() {
    for quote in [WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
        let partial = case(
            SwqosType::Default,
            false,
            50,
            quote,
            false,
            spl_token_interface::ID,
        );
        let full = case(
            SwqosType::Default,
            false,
            100,
            quote,
            false,
            spl_token_interface::ID,
        );
        let partial = partial.profile().unwrap();
        let full = full.profile().unwrap();
        assert_eq!(partial.protocol.len(), 2);
        assert_eq!(full.protocol.len(), 3);
        assert_eq!(full.protocol[2].program_id, spl_token_interface::ID);
        assert_eq!(full.protocol[2].data, [9]);
        assert_eq!(
            full.protocol[2].accounts[0].pubkey,
            full.protocol[1].accounts[2].pubkey
        );
        assert_eq!(full.protocol[2].accounts[1].pubkey, full.context.wallet);
    }
}

#[test]
fn damm_sell_message_retains_checked_u64_overflow_and_zero_percentage_rounding() {
    for (balance, percent, expected) in [(u64::MAX, 100, "overflow"), (1, 1, "zero")] {
        let mut f = fixture(SwqosType::Default, true, percent);
        let wallet = Address::from_bytes(f.context.wallet.to_bytes());
        let mint = Address::from_bytes(f.intent.mint.to_bytes());
        let input = f.holding.checked_input(wallet, mint).unwrap();
        f.holding = Arc::new(
            PreparationBalanceSnapshot::from_account(
                wallet,
                mint,
                balance,
                input,
                123,
                Instant::now(),
            )
            .unwrap(),
        );
        f.balance = balance;
        let error = f.profile().err().unwrap();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn damm_sell_message_rejects_canonical_quote_under_wrong_program() {
    let mut f = fixture(SwqosType::Default, true, 50);
    let DexParamEnum::MeteoraDammV2(pool) = &mut f.request.extension_params else {
        unreachable!()
    };
    pool.token_b_program = spl_token_2022_interface::ID;
    assert!(f
        .profile()
        .err()
        .unwrap()
        .to_string()
        .contains("canonical output"));
}
