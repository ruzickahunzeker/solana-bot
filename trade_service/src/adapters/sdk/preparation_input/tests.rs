use super::*;
use crate::{
    adapters::sdk::{
        ordinary_lanes::{
            self, OrdinarySdkLane, SdkInvocationFailure, SdkLaneObservation, SdkLaneReport,
        },
        ordinary_params::{ordinary_buy_params, ordinary_sell_params},
    },
    domain::preparation_balance::{PreparationInput, MAX_BALANCE_AGE},
};
use sol_trade_sdk::{
    swqos::SwqosType,
    trading::core::params::{
        MeteoraDammV2Params, MeteoraDbcParams, PumpFunParams, PumpSwapParams, RaydiumAmmV4Params,
        RaydiumCpmmParams, StonkFunViaSolParams,
    },
};
use solana_sdk::{account::Account, hash::Hash};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn k(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}
fn a(key: Pubkey) -> Address {
    Address::from_bytes(key.to_bytes())
}
fn trade(pool: Pool, quote: Pubkey) -> ExecuteTrade {
    ExecuteTrade {
        pool,
        mint: k(7),
        quote_mint: quote,
        pool_id: Some(k(8)),
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(1000),
        sell_percent: Some(100),
        slippage_bps: 1500,
        tip_sol: 0.0001,
        recent_blockhash: Hash::new_from_array([9; 32]),
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 0,
        simulate: false,
        use_mev: None,
        mev_provider: None,
    }
}
fn cpmm(base: Pubkey, quote: Pubkey, bp: Pubkey, qp: Pubkey) -> RaydiumCpmmParams {
    RaydiumCpmmParams::from_trade(
        k(8),
        k(10),
        base,
        quote,
        k(11),
        k(12),
        bp,
        qp,
        k(13),
        1_000_000,
        2_000_000,
    )
}
fn curve(t: &ExecuteTrade, mp: Pubkey, qp: Pubkey) -> BonkParams {
    BonkParams {
        mint_token_program: mp,
        quote_mint: t.quote_mint,
        quote_token_program: qp,
        pool_state: k(8),
        global_config: k(10),
        platform_config: k(11),
        platform_associated_account: k(12),
        creator_associated_account: k(13),
        ..Default::default()
    }
}
fn params(t: &ExecuteTrade, mp: Pubkey, qp: Pubkey, reversed: bool) -> DexParamEnum {
    let (base, quote, bp, quote_program) = if reversed {
        (t.quote_mint, t.mint, qp, mp)
    } else {
        (t.mint, t.quote_mint, mp, qp)
    };
    match t.pool {
        Pool::Pump => {
            let mut p = PumpFunParams::immediate_sell(k(10), mp, false);
            p.quote_mint = t.quote_mint;
            DexParamEnum::PumpFun(p)
        }
        Pool::Bonk => DexParamEnum::Bonk(curve(t, mp, qp)),
        Pool::StonkFun => DexParamEnum::StonkFun(curve(t, mp, qp)),
        Pool::StonkFunSwap => DexParamEnum::StonkFunSwap(cpmm(base, quote, bp, quote_program)),
        Pool::RaydiumCpmm => DexParamEnum::RaydiumCpmm(cpmm(base, quote, bp, quote_program)),
        Pool::PumpAmm => DexParamEnum::PumpSwap(PumpSwapParams::new(
            k(8),
            base,
            quote,
            k(11),
            k(12),
            1_000_000,
            2_000_000,
            0,
            k(13),
            k(14),
            bp,
            quote_program,
            k(15),
            Pubkey::default(),
            false,
            0,
        )),
        Pool::MeteoraDammV2 => DexParamEnum::MeteoraDammV2(MeteoraDammV2Params::new(
            k(8),
            k(11),
            k(12),
            base,
            quote,
            bp,
            quote_program,
        )),
        Pool::RaydiumAmmV4 => DexParamEnum::RaydiumAmmV4(RaydiumAmmV4Params::new(
            k(8),
            base,
            quote,
            k(11),
            k(12),
            1_000_000,
            2_000_000,
        )),
        Pool::MeteoraDbc => {
            use sol_trade_sdk::instruction::utils::meteora_dbc::{
                accounts, decode_pool_config, decode_virtual_pool, POOL_CONFIG_DISCRIMINATOR,
                VIRTUAL_POOL_ACCOUNT_SIZE, VIRTUAL_POOL_DISCRIMINATOR,
            };
            let account = |data| Account {
                lamports: 1,
                owner: accounts::PROGRAM,
                data,
                ..Default::default()
            };
            let mut data = vec![0; 8 + 1040];
            data[..8].copy_from_slice(&POOL_CONFIG_DISCRIMINATOR);
            let mut config = decode_pool_config(&account(data)).unwrap();
            config.quote_mint = t.quote_mint;
            config.token_decimal = 6;
            let mut data = vec![0; VIRTUAL_POOL_ACCOUNT_SIZE as usize];
            data[..8].copy_from_slice(&VIRTUAL_POOL_DISCRIMINATOR);
            DexParamEnum::MeteoraDbc(MeteoraDbcParams {
                pool: k(8),
                config: k(10),
                base_vault: k(11),
                quote_vault: k(12),
                base_mint: base,
                quote_mint: quote,
                base_token_program: bp,
                quote_token_program: quote_program,
                pool_config: config,
                virtual_pool: decode_virtual_pool(&account(data)).unwrap(),
                referral_token_account: None,
                include_instructions_sysvar: false,
            })
        }
    }
}
enum Request {
    Buy(TradeBuyParams),
    Sell(TradeSellParams),
}
impl Request {
    fn input(&self) -> SdkInput<'_> {
        match self {
            Self::Buy(p) => SdkInput::Buy(p),
            Self::Sell(p) => SdkInput::Sell(p),
        }
    }
}
fn request(t: &ExecuteTrade, p: DexParamEnum, side: Side) -> Request {
    if side == Side::Buy {
        let ty = if t.sol_hop_pool_id.is_some() {
            TradeTokenType::SOL
        } else {
            super::super::protocol_params::quote_tokens::trade_token_type_for_buy(
                t.pool,
                &t.quote_mint,
            )
            .unwrap()
        };
        Request::Buy(ordinary_buy_params(
            t,
            p,
            ty,
            t.amount_lamports.unwrap(),
            true,
            SwqosType::Temporal,
        ))
    } else {
        Request::Sell(
            ordinary_sell_params(t, p, Some(1), t.sell_percent.unwrap(), SwqosType::Temporal)
                .unwrap(),
        )
    }
}
fn holding(
    t: &ExecuteTrade,
    side: Side,
    program: Pubkey,
    decimals: u8,
) -> PreparationBalanceSnapshot {
    let mint = preparation_balance::balance_mint(t, side).unwrap();
    let account=spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&k(1),&mint,&program);
    PreparationBalanceSnapshot::from_account(
        a(k(1)),
        a(mint),
        1_000_000,
        PreparationInput {
            account: a(account),
            program: a(program),
            decimals,
        },
        42,
        Instant::now(),
    )
    .unwrap()
}
const POOLS: [Pool; 9] = [
    Pool::Pump,
    Pool::PumpAmm,
    Pool::Bonk,
    Pool::StonkFun,
    Pool::StonkFunSwap,
    Pool::MeteoraDammV2,
    Pool::MeteoraDbc,
    Pool::RaydiumAmmV4,
    Pool::RaydiumCpmm,
];

#[test]
fn preparation_input_all_current_sell_protocols_bind_canonical_account_and_program() {
    for pool in POOLS {
        for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
            if pool == Pool::RaydiumAmmV4 && program == spl_token_2022_interface::ID {
                continue;
            }
            for reversed in [false, true] {
                if reversed
                    && matches!(
                        pool,
                        Pool::Pump | Pool::Bonk | Pool::StonkFun | Pool::MeteoraDbc
                    )
                {
                    continue;
                }
                for simulate in [false, true] {
                    let mut t = trade(pool, WSOL_TOKEN_ACCOUNT);
                    t.simulate = simulate;
                    let p = params(&t, program, spl_token_interface::ID, reversed);
                    let request = request(&t, p, Side::Sell);
                    let balance = holding(&t, Side::Sell, program, 6);
                    validate(Some(&balance), a(k(1)), &t, Side::Sell, request.input()).unwrap();
                }
            }
        }
    }
}

#[test]
fn preparation_input_direct_quote_buys_bind_actual_payment_asset_both_programs_and_orientations() {
    for pool in [
        Pool::PumpAmm,
        Pool::StonkFun,
        Pool::StonkFunSwap,
        Pool::RaydiumCpmm,
    ] {
        for quote in [USDC_TOKEN_ACCOUNT, k(90)] {
            for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                if quote == USDC_TOKEN_ACCOUNT && program != spl_token_interface::ID {
                    continue;
                }
                for reversed in [false, true] {
                    if reversed && pool == Pool::StonkFun {
                        continue;
                    }
                    let t = trade(pool, quote);
                    let request = request(
                        &t,
                        params(&t, spl_token_2022_interface::ID, program, reversed),
                        Side::Buy,
                    );
                    let balance = holding(&t, Side::Buy, program, 6);
                    validate(Some(&balance), a(k(1)), &t, Side::Buy, request.input()).unwrap();
                }
            }
        }
    }
}

#[test]
fn preparation_input_wrong_program_raw_account_or_metadata_only_is_rejected() {
    let t = trade(Pool::RaydiumCpmm, WSOL_TOKEN_ACCOUNT);
    let request = request(
        &t,
        params(&t, spl_token_interface::ID, spl_token_interface::ID, false),
        Side::Sell,
    );
    let bad_program = holding(&t, Side::Sell, spl_token_2022_interface::ID, 6);
    assert!(validate(Some(&bad_program), a(k(1)), &t, Side::Sell, request.input()).is_err());
    let wrong_account = PreparationBalanceSnapshot::from_account(
        a(k(1)),
        a(t.mint),
        1_000_000,
        PreparationInput {
            account: a(k(99)),
            program: a(spl_token_interface::ID),
            decimals: 6,
        },
        42,
        Instant::now(),
    )
    .unwrap();
    assert!(validate(
        Some(&wrong_account),
        a(k(1)),
        &t,
        Side::Sell,
        request.input()
    )
    .is_err());
    let metadata =
        PreparationBalanceSnapshot::fixture(a(k(1)), a(t.mint), 1_000_000, Instant::now());
    // The old quantity-only check is not sufficient to bind an SDK input.
    preparation_balance::validate(Some(&metadata), a(k(1)), &t, Side::Sell).unwrap();
    assert!(validate(Some(&metadata), a(k(1)), &t, Side::Sell, request.input()).is_err());
    assert!(validate(None, a(k(1)), &t, Side::Sell, request.input()).is_err());
    assert!(validate(
        Some(&bad_program),
        a(k(99)),
        &t,
        Side::Sell,
        request.input()
    )
    .is_err());
    let t = trade(Pool::RaydiumCpmm, USDC_TOKEN_ACCOUNT);
    let r = self::request(
        &t,
        params(
            &t,
            spl_token_interface::ID,
            spl_token_2022_interface::ID,
            false,
        ),
        Side::Buy,
    );
    let bad = holding(&t, Side::Buy, spl_token_2022_interface::ID, 6);
    assert!(validate(Some(&bad), a(k(1)), &t, Side::Buy, r.input()).is_err());
}

#[test]
fn preparation_input_pump_default_and_suffix_override_match_effective_sdk_program() {
    for suffix in [false, true] {
        for declared in [
            Pubkey::default(),
            spl_token_interface::ID,
            spl_token_2022_interface::ID,
        ] {
            let mut t = trade(Pool::Pump, WSOL_TOKEN_ACCOUNT);
            if suffix {
                t.mint = "6mYfg1EmMVjbQv38QGY5XdRUQ4SyNpFauL1aiRiWpump"
                    .parse()
                    .unwrap();
            }
            let request = request(
                &t,
                params(&t, declared, spl_token_interface::ID, false),
                Side::Sell,
            );
            for actual in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                let balance = holding(&t, Side::Sell, actual, 6);
                let expected = if suffix || declared == Pubkey::default() {
                    spl_token_2022_interface::ID
                } else {
                    declared
                };
                assert_eq!(
                    validate(Some(&balance), a(k(1)), &t, Side::Sell, request.input()).is_ok(),
                    actual == expected
                );
            }
        }
    }
}

#[test]
fn preparation_input_curve_quote_defaults_follow_global_config_and_classic_program_default() {
    for quote in [WSOL_TOKEN_ACCOUNT, USD1_TOKEN_ACCOUNT] {
        let t = trade(Pool::StonkFun, quote);
        let mut p = curve(&t, spl_token_2022_interface::ID, Pubkey::default());
        p.quote_mint = Pubkey::default();
        if quote == USD1_TOKEN_ACCOUNT {
            p.global_config = sol_trade_sdk::instruction::utils::bonk::accounts::USD1_GLOBAL_CONFIG;
        }
        let r = request(&t, DexParamEnum::StonkFun(p.clone()), Side::Sell);
        let balance = holding(&t, Side::Sell, spl_token_2022_interface::ID, 6);
        validate(Some(&balance), a(k(1)), &t, Side::Sell, r.input()).unwrap();
        let wrong = trade(Pool::StonkFun, k(90));
        let r = request(&wrong, DexParamEnum::StonkFun(p), Side::Sell);
        assert!(validate(Some(&balance), a(k(1)), &wrong, Side::Sell, r.input()).is_err());
    }
    let t = trade(Pool::Bonk, WSOL_TOKEN_ACCOUNT);
    let balance = holding(&t, Side::Sell, spl_token_interface::ID, 6);
    let mut p = curve(&t, spl_token_interface::ID, spl_token_interface::ID);
    p.quote_mint = SOL_TOKEN_ACCOUNT;
    let r = request(&t, DexParamEnum::Bonk(p), Side::Sell);
    assert!(validate(Some(&balance), a(k(1)), &t, Side::Sell, r.input()).is_err());
}

#[test]
fn preparation_input_dbc_precision_and_fixed_base_side_are_bound_without_amount_conversion() {
    let t = trade(Pool::MeteoraDbc, WSOL_TOKEN_ACCOUNT);
    for decimals in [6, 9] {
        let balance = holding(&t, Side::Sell, spl_token_2022_interface::ID, decimals);
        let r = request(
            &t,
            params(
                &t,
                spl_token_2022_interface::ID,
                spl_token_interface::ID,
                false,
            ),
            Side::Sell,
        );
        assert_eq!(
            validate(Some(&balance), a(k(1)), &t, Side::Sell, r.input()).is_ok(),
            decimals == 6
        );
    }
    let balance = holding(&t, Side::Sell, spl_token_2022_interface::ID, 6);
    let r = request(
        &t,
        params(
            &t,
            spl_token_2022_interface::ID,
            spl_token_interface::ID,
            true,
        ),
        Side::Sell,
    );
    assert!(validate(Some(&balance), a(k(1)), &t, Side::Sell, r.input()).is_err());
}

#[test]
fn preparation_input_rejects_wrong_request_kind_asset_dex_and_parameter_variant() {
    let t = trade(Pool::RaydiumCpmm, USDC_TOKEN_ACCOUNT);
    let balance = holding(&t, Side::Buy, spl_token_interface::ID, 6);
    for damage in 0..6 {
        let Request::Buy(mut r) = request(
            &t,
            params(
                &t,
                spl_token_2022_interface::ID,
                spl_token_interface::ID,
                false,
            ),
            Side::Buy,
        ) else {
            unreachable!()
        };
        match damage {
            0 => r.input_token_type = TradeTokenType::SOL,
            1 => r.mint = k(99),
            2 => r.input_token_amount = 1001,
            3 => r.dex_type = DexType::PumpSwap,
            5 => r.simulate = true,
            _ => {
                r.extension_params = DexParamEnum::Bonk(curve(
                    &t,
                    spl_token_2022_interface::ID,
                    spl_token_interface::ID,
                ))
            }
        }
        assert!(validate(Some(&balance), a(k(1)), &t, Side::Buy, SdkInput::Buy(&r)).is_err());
    }
    let r = request(
        &t,
        params(
            &t,
            spl_token_2022_interface::ID,
            spl_token_interface::ID,
            false,
        ),
        Side::Sell,
    );
    assert!(validate(Some(&balance), a(k(1)), &t, Side::Buy, r.input()).is_err());
}

#[test]
fn preparation_input_rejects_wrong_pair_alias_unsupported_program_and_sell_output() {
    let t = trade(Pool::RaydiumCpmm, WSOL_TOKEN_ACCOUNT);
    let balance = holding(&t, Side::Sell, spl_token_interface::ID, 6);
    for damage in 0..6 {
        let mut p = cpmm(
            t.mint,
            t.quote_mint,
            spl_token_interface::ID,
            spl_token_interface::ID,
        );
        match damage {
            0 => p.base_mint = k(99),
            1 => p.quote_mint = t.mint,
            2 => p.base_token_program = Pubkey::default(),
            3 => p.quote_token_program = k(99),
            _ => {}
        }
        let Request::Sell(mut r) = request(&t, DexParamEnum::RaydiumCpmm(p), Side::Sell) else {
            unreachable!()
        };
        if damage == 4 {
            r.output_token_type = TradeTokenType::USDC;
        }
        if damage == 5 {
            r.simulate = true;
        }
        assert!(validate(Some(&balance), a(k(1)), &t, Side::Sell, SdkInput::Sell(&r)).is_err());
    }
}

fn routed(t: &mut ExecuteTrade, mp: Pubkey, qp: Pubkey, reversed: bool) -> DexParamEnum {
    t.sol_hop_pool = Some(Pool::RaydiumCpmm);
    t.sol_hop_pool_id = Some(k(8));
    let hop = cpmm(
        WSOL_TOKEN_ACCOUNT,
        t.quote_mint,
        spl_token_interface::ID,
        qp,
    );
    let meme = if t.pool == Pool::StonkFun {
        StonkFunMemeLeg::Curve(curve(t, mp, qp))
    } else {
        let p = if reversed {
            cpmm(t.quote_mint, t.mint, qp, mp)
        } else {
            cpmm(t.mint, t.quote_mint, mp, qp)
        };
        StonkFunMemeLeg::Graduated(p)
    };
    DexParamEnum::StonkFunViaSol(StonkFunViaSolParams {
        meme_leg: meme,
        sol_hop: StonkFunSolHop::RaydiumCpmm(hop),
    })
}

#[test]
fn preparation_input_stonk_wrapper_binds_meme_source_and_program_continuity() {
    for pool in [Pool::StonkFun, Pool::StonkFunSwap] {
        for reversed in [false, true] {
            for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                let mut t = trade(pool, k(90));
                let p = routed(&mut t, program, spl_token_2022_interface::ID, reversed);
                let r = request(&t, p, Side::Sell);
                let balance = holding(&t, Side::Sell, program, 6);
                validate(Some(&balance), a(k(1)), &t, Side::Sell, r.input()).unwrap();
            }
        }
    }
}

#[test]
fn preparation_input_stonk_wrapper_rejects_endpoint_hop_id_pair_program_or_venue_change() {
    for damage in 0..7 {
        let mut t = trade(Pool::StonkFun, k(90));
        let mut p = routed(
            &mut t,
            spl_token_2022_interface::ID,
            spl_token_interface::ID,
            false,
        );
        let DexParamEnum::StonkFunViaSol(wrapper) = &mut p else {
            unreachable!()
        };
        match damage {
            0 => {
                wrapper.meme_leg = StonkFunMemeLeg::Graduated(cpmm(
                    t.mint,
                    t.quote_mint,
                    spl_token_2022_interface::ID,
                    spl_token_interface::ID,
                ))
            }
            1 => t.sol_hop_pool_id = Some(k(99)),
            2 => {
                let StonkFunSolHop::RaydiumCpmm(hop) = &mut wrapper.sol_hop else {
                    unreachable!()
                };
                hop.quote_mint = k(99);
            }
            3 => {
                let StonkFunSolHop::RaydiumCpmm(hop) = &mut wrapper.sol_hop else {
                    unreachable!()
                };
                hop.quote_token_program = spl_token_2022_interface::ID;
            }
            4 => {
                wrapper.sol_hop = StonkFunSolHop::RaydiumAmmV4(RaydiumAmmV4Params::new(
                    k(8),
                    WSOL_TOKEN_ACCOUNT,
                    t.quote_mint,
                    k(11),
                    k(12),
                    1000,
                    2000,
                ))
            }
            5 => t.sol_hop_pool = None,
            _ => {
                t.sol_hop_pool = None;
                t.sol_hop_pool_id = None;
            }
        }
        let r = request(&t, p, Side::Sell);
        let balance = holding(&t, Side::Sell, spl_token_2022_interface::ID, 6);
        assert!(
            validate(Some(&balance), a(k(1)), &t, Side::Sell, r.input()).is_err(),
            "damage={damage}"
        );
    }
}

#[test]
fn preparation_input_preserves_existing_untracked_buy_scope_but_cannot_use_extra_snapshot() {
    let t = trade(Pool::Pump, WSOL_TOKEN_ACCOUNT);
    let r = request(
        &t,
        params(
            &t,
            spl_token_2022_interface::ID,
            spl_token_interface::ID,
            false,
        ),
        Side::Buy,
    );
    validate(None, a(k(1)), &t, Side::Buy, r.input()).unwrap();
    let snapshot = holding(&t, Side::Sell, spl_token_2022_interface::ID, 6);
    assert!(validate(Some(&snapshot), a(k(1)), &t, Side::Buy, r.input()).is_err());
}

#[test]
fn preparation_input_damm_preview_has_the_same_raw_input_and_cannot_be_live() {
    let t = trade(Pool::MeteoraDammV2, WSOL_TOKEN_ACCOUNT);
    let holding = holding(&t, Side::Sell, spl_token_2022_interface::ID, 6);
    for damage in 0..5 {
        let Request::Sell(mut preview) = request(
            &t,
            params(
                &t,
                spl_token_2022_interface::ID,
                spl_token_interface::ID,
                false,
            ),
            Side::Sell,
        ) else {
            unreachable!()
        };
        preview.simulate = true;
        match damage {
            0 => {}
            1 => preview.simulate = false,
            2 => preview.mint = k(99),
            3 => {
                let DexParamEnum::MeteoraDammV2(p) = &mut preview.extension_params else {
                    unreachable!()
                };
                p.token_a_program = spl_token_interface::ID;
            }
            _ => preview.output_token_type = TradeTokenType::USDC,
        }
        assert_eq!(
            validate_preview(Some(&holding), a(k(1)), &t, &preview).is_ok(),
            damage == 0
        );
    }
}

struct GateLane {
    bad: bool,
    calls: Arc<AtomicUsize>,
    prepares: Arc<AtomicUsize>,
    fail_after_entry: bool,
}
impl OrdinarySdkLane for GateLane {
    type Prepared = (PreparationBalanceSnapshot, Request);
    fn provider(&self) -> SwqosType {
        SwqosType::Temporal
    }
    async fn prepare(&self, t: &ExecuteTrade) -> Result<Self::Prepared> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        let balance = holding(
            t,
            Side::Sell,
            if self.bad {
                spl_token_2022_interface::ID
            } else {
                spl_token_interface::ID
            },
            6,
        );
        Ok((
            balance,
            request(
                t,
                params(t, spl_token_interface::ID, spl_token_interface::ID, false),
                Side::Sell,
            ),
        ))
    }
    fn validate_prepared(&self, p: &Self::Prepared, t: &ExecuteTrade) -> Result<()> {
        validate(Some(&p.0), a(k(1)), t, Side::Sell, p.1.input())
    }
    async fn invoke(
        &self,
        _: Self::Prepared,
    ) -> std::result::Result<SdkLaneReport, SdkInvocationFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_after_entry {
            Err(SdkInvocationFailure {
                signatures: vec![],
                message: "fixture uncertainty".into(),
            })
        } else {
            Ok(SdkLaneReport {
                success: true,
                signatures: vec![solana_sdk::signature::Signature::from([1; 64]).to_string()],
                error: None,
            })
        }
    }
    async fn observe(&self, _: &str, _: &ExecuteTrade) -> Result<SdkLaneObservation> {
        Ok(SdkLaneObservation::Skipped)
    }
}

#[tokio::test]
async fn preparation_input_real_controller_stops_bad_binding_without_sdk_or_other_lane() {
    let calls = Arc::new(AtomicUsize::new(0));
    let prepares = Arc::new(AtomicUsize::new(0));
    let first = GateLane {
        bad: true,
        calls: calls.clone(),
        prepares: prepares.clone(),
        fail_after_entry: false,
    };
    let next = GateLane {
        bad: false,
        calls: calls.clone(),
        prepares: prepares.clone(),
        fail_after_entry: false,
    };
    assert!(
        ordinary_lanes::execute([first, next], &trade(Pool::RaydiumCpmm, WSOL_TOKEN_ACCOUNT))
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(prepares.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn preparation_input_real_controller_binding_never_authorizes_post_entry_retry() {
    for fail_after_entry in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let prepares = Arc::new(AtomicUsize::new(0));
        let first = GateLane {
            bad: false,
            calls: calls.clone(),
            prepares: prepares.clone(),
            fail_after_entry,
        };
        let next = GateLane {
            bad: false,
            calls: calls.clone(),
            prepares: prepares.clone(),
            fail_after_entry: false,
        };
        let result =
            ordinary_lanes::execute([first, next], &trade(Pool::RaydiumCpmm, WSOL_TOKEN_ACCOUNT))
                .await;
        assert_eq!(result.is_err(), fail_after_entry);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(prepares.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn preparation_input_expired_raw_identity_is_not_renewed_or_accepted() {
    let t = trade(Pool::RaydiumCpmm, WSOL_TOKEN_ACCOUNT);
    let balance = holding(&t, Side::Sell, spl_token_interface::ID, 6);
    let r = request(
        &t,
        params(&t, spl_token_interface::ID, spl_token_interface::ID, false),
        Side::Sell,
    );
    tokio::time::sleep(MAX_BALANCE_AGE + Duration::from_millis(50)).await;
    assert!(validate(Some(&balance), a(k(1)), &t, Side::Sell, r.input()).is_err());
}
