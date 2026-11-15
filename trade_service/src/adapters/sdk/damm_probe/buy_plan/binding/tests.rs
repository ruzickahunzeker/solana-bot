use super::*;
use crate::adapters::sdk::{
    damm_probe::buy_plan::evidence::tests::binding_fixture, ordinary_params::ordinary_buy_params,
};
use solana_sdk::hash::Hash;

fn intent(plan: &PreparedDammBuyProbe, simulate: bool) -> ExecuteTrade {
    ExecuteTrade {
        pool: Pool::MeteoraDammV2,
        mint: plan.accounts.output_mint,
        quote_mint: plan.accounts.input_mint,
        pool_id: Some(plan.accounts.pool.pool),
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(plan.accounts.amount),
        sell_percent: None,
        slippage_bps: 1500,
        tip_sol: 0.0001,
        recent_blockhash: plan.transaction.message.recent_blockhash,
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 250,
        simulate,
        use_mev: None,
        mev_provider: None,
    }
}

async fn fixture(
    native: bool,
    extended: bool,
    exists: bool,
    simulate: bool,
    provider: SwqosType,
) -> (DammBuyQuote, ExecuteTrade, TradeBuyParams, DammBuyContext) {
    let (plan, evidence) = binding_fixture(native, extended, exists).await;
    let command = intent(&plan, simulate);
    let context = DammBuyContext {
        wallet: plan.accounts.wallet,
        provider,
        version: TradeTransactionVersion::V0,
        seed_optimized: false,
    };
    let input = if native {
        TradeTokenType::WSOL
    } else {
        TradeTokenType::USDC
    };
    let minimum = evidence.minimum(&plan, command.slippage_bps).unwrap();
    let request = ordinary_buy_params(
        &command,
        DexParamEnum::MeteoraDammV2(plan.accounts.pool.clone()),
        input,
        plan.accounts.amount,
        exists,
        provider,
        Some(minimum),
    )
    .unwrap();
    let binding = DammBuyQuote::new(context, &command, plan, evidence).unwrap();
    (binding, command, request, context)
}

#[tokio::test]
async fn damm_buy_binding_matches_real_net_evidence_for_both_quotes_token_programs_modes_and_providers(
) {
    let mut count = 0;
    for native in [false, true] {
        for extended in [false, true] {
            for exists in [false, true] {
                for simulate in [false, true] {
                    for provider in [
                        SwqosType::Default,
                        SwqosType::Jito,
                        SwqosType::Temporal,
                        SwqosType::FlashBlock,
                        SwqosType::Astralane,
                        SwqosType::BlockRazor,
                        SwqosType::Speedlanding,
                    ] {
                        let (quote, command, request, context) =
                            fixture(native, extended, exists, simulate, provider).await;
                        quote.validate(context, &command, &request).unwrap();
                        quote.recheck_request(context, &request).unwrap();
                        assert_eq!(
                            request.fixed_output_token_amount,
                            Some(if extended { 100 } else { 102 })
                        );
                        assert_eq!(quote.context(), (42, 43, 5000));
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(count, 112);
}

#[tokio::test]
async fn damm_buy_binding_rejects_each_changed_execution_intent_field() {
    let (quote, command, request, context) =
        fixture(true, false, true, false, SwqosType::Temporal).await;
    for change in 0..17 {
        let mut current = command.clone();
        let key = Pubkey::new_from_array([99; 32]);
        match change {
            0 => current.pool = Pool::Pump,
            1 => current.mint = key,
            2 => current.quote_mint = key,
            3 => current.pool_id = Some(key),
            4 => current.sol_hop_pool = Some(Pool::RaydiumCpmm),
            5 => current.sol_hop_pool_id = Some(key),
            6 => current.amount_lamports = Some(101),
            7 => current.sell_percent = Some(100),
            8 => current.slippage_bps = 1000,
            9 => current.tip_sol = 0.002,
            10 => current.recent_blockhash = Hash::new_from_array([99; 32]),
            11 => current.wait_tx_confirmed = true,
            12 => current.mev_submit_verify_ms = 999,
            13 => current.simulate = true,
            14 => current.use_mev = Some(true),
            15 => current.mev_provider = Some(crate::domain::trade::MevProvider::Jito),
            _ => current.tip_sol = -0.0,
        }
        assert!(
            quote.validate(context, &current, &request).is_err(),
            "change={change}"
        );
    }
}

#[tokio::test]
async fn damm_buy_binding_checks_request_amount_minimum_flags_hash_and_variant() {
    let (quote, command, base, context) =
        fixture(true, false, false, false, SwqosType::Temporal).await;
    for change in 0..20 {
        let mut request = base.clone();
        match change {
            0 => request.input_token_amount += 1,
            1 => request.fixed_output_token_amount = None,
            2 => request.fixed_output_token_amount = Some(1),
            3 => request.fixed_output_token_amount = Some(0),
            4 => request.mint = Pubkey::new_from_array([99; 32]),
            5 => request.input_token_type = TradeTokenType::SOL,
            6 => request.slippage_basis_points = Some(1000),
            7 => request.recent_blockhash = Some(Hash::default()),
            8 => request.wait_tx_confirmed = true,
            9 => request.wait_for_all_submits = true,
            10 => request.create_input_token_ata = false,
            11 => request.close_input_token_ata = true,
            12 => request.create_mint_ata = false,
            13 => request.simulate = true,
            14 => request.use_exact_sol_amount = None,
            15 => request.grpc_recv_us = Some(1),
            16 => request.dex_type = DexType::PumpFun,
            17 => request.extension_params = DexParamEnum::Bonk(Default::default()),
            18 => {
                let DexParamEnum::MeteoraDammV2(p) = &mut request.extension_params else {
                    unreachable!()
                };
                p.token_a_vault = Pubkey::new_from_array([99; 32]);
            }
            _ => {
                let DexParamEnum::MeteoraDammV2(p) = &mut request.extension_params else {
                    unreachable!()
                };
                p.swap_mode = 2;
            }
        }
        assert!(
            quote.validate(context, &command, &request).is_err(),
            "change={change}"
        );
    }
}

#[tokio::test]
async fn damm_buy_binding_rejects_wallet_provider_version_and_seed_changes() {
    let (quote, command, request, context) =
        fixture(false, false, true, false, SwqosType::Jito).await;
    for change in 0..4 {
        let mut current = context;
        match change {
            0 => current.wallet = Pubkey::new_from_array([99; 32]),
            1 => current.provider = SwqosType::Default,
            2 => current.version = TradeTransactionVersion::V1,
            _ => current.seed_optimized = true,
        }
        assert!(quote.validate(current, &command, &request).is_err());
    }
}

#[tokio::test]
async fn damm_buy_binding_expiry_is_checked_again_without_renewal_or_requoting() {
    let (mut quote, command, request, context) =
        fixture(true, false, true, false, SwqosType::Temporal).await;
    quote.validate(context, &command, &request).unwrap();
    quote.evidence.expire_for_tests();
    for _ in 0..2 {
        assert!(quote.recheck_request(context, &request).is_err());
    }
}

#[tokio::test]
async fn damm_buy_binding_cannot_be_created_from_another_or_expired_discovery_packet() {
    for change in 0..5 {
        let (plan, mut evidence) = binding_fixture(true, false, true).await;
        let mut command = intent(&plan, false);
        let mut context = DammBuyContext {
            wallet: plan.accounts.wallet,
            provider: SwqosType::Default,
            version: TradeTransactionVersion::V0,
            seed_optimized: false,
        };
        match change {
            0 => command.amount_lamports = Some(101),
            1 => command.pool_id = Some(Pubkey::new_from_array([99; 32])),
            2 => context.wallet = Pubkey::new_from_array([99; 32]),
            3 => evidence.expire_for_tests(),
            _ => command.slippage_bps = 9999,
        }
        assert!(DammBuyQuote::new(context, &command, plan, evidence).is_err());
    }
}

#[tokio::test]
async fn damm_buy_binding_rechecks_shared_fee_strategy_mutation_against_frozen_numbers() {
    let (quote, command, request, context) =
        fixture(true, false, true, false, SwqosType::Temporal).await;
    quote.validate(context, &command, &request).unwrap();
    request.gas_fee_strategy.set(
        SwqosType::Temporal,
        TradeType::Buy,
        GasFeeStrategyType::Normal,
        300_000,
        2000,
        command.tip_sol,
    );
    assert!(quote.recheck_request(context, &request).is_err());
}

#[tokio::test]
async fn damm_buy_binding_skips_target_creation_only_with_existing_simulated_token_evidence() {
    for exists in [false, true] {
        let (quote, command, mut request, context) =
            fixture(false, false, exists, false, SwqosType::Default).await;
        request.create_mint_ata = false;
        assert_eq!(quote.validate(context, &command, &request).is_ok(), exists);
    }
}

struct Preparation {
    rpc: solana_client::nonblocking::rpc_client::RpcClient,
    params: sol_trade_sdk::trading::core::params::MeteoraDammV2Params,
    context: DammBuyContext,
    exists: bool,
    captured: std::sync::Mutex<Option<DammBuyQuote>>,
}

impl crate::adapters::sdk::ordinary_preparation::OrdinaryPreparationSource for Preparation {
    async fn load_params(&self, _: &ExecuteTrade) -> Result<DexParamEnum> {
        Ok(DexParamEnum::MeteoraDammV2(self.params.clone()))
    }
    async fn token_balance(&self, _: Pubkey) -> Result<u64> {
        panic!("DAMM buy input is checked by the actual quote snapshot");
    }
    async fn mint_account_exists(&self, _: Pubkey) -> Result<bool> {
        Ok(self.exists)
    }
    async fn damm_sell_output(
        &self,
        _: &ExecuteTrade,
        _: &sol_trade_sdk::trading::core::params::MeteoraDammV2Params,
        _: u64,
    ) -> Result<u64> {
        panic!("buy cannot use a sell quote");
    }
    async fn damm_buy_output(
        &self,
        command: &ExecuteTrade,
        params: &sol_trade_sdk::trading::core::params::MeteoraDammV2Params,
        amount: u64,
    ) -> Result<u64> {
        assert_eq!(command.amount_lamports, Some(amount));
        let quote = DammBuyQuote::read(self.context, command, params, &self.rpc).await?;
        let units = quote.output_units();
        assert!(self.captured.lock().unwrap().replace(quote).is_none());
        Ok(units)
    }
}

#[tokio::test]
async fn damm_buy_binding_actual_read_preparation_projection_and_entry_check_are_connected_without_signing(
) {
    for native in [false, true] {
        for extended in [false, true] {
            for exists in [false, true] {
                let (plan, read) =
                    super::super::evidence::tests::preparation_fixture(native, extended, exists);
                let command = intent(&plan, false);
                let context = DammBuyContext {
                    wallet: plan.accounts.wallet,
                    provider: SwqosType::Temporal,
                    version: TradeTransactionVersion::V0,
                    seed_optimized: false,
                };
                let source = Preparation {
                    rpc: read.client(),
                    params: plan.accounts.pool,
                    context,
                    exists,
                    captured: Default::default(),
                };
                let mut request = crate::adapters::sdk::ordinary_preparation::buy(
                    &source,
                    &command,
                    context.provider,
                )
                .await
                .unwrap();
                let quote = source.captured.lock().unwrap().take().unwrap();
                quote.validate(context, &command, &request).unwrap();
                quote.recheck_request(context, &request).unwrap();
                assert_eq!(
                    request.fixed_output_token_amount,
                    Some(if extended { 100 } else { 102 })
                );
                request.fixed_output_token_amount = Some(1);
                assert!(quote.recheck_request(context, &request).is_err());
                assert_eq!(
                    read.methods(),
                    [
                        solana_client::rpc_request::RpcRequest::GetMultipleAccounts,
                        solana_client::rpc_request::RpcRequest::SimulateTransaction
                    ]
                );
            }
        }
    }
}

#[tokio::test]
async fn damm_buy_binding_actual_read_rejects_unsupported_sdk_mode_before_any_quote() {
    let (plan, read) = super::super::evidence::tests::preparation_fixture(true, false, false);
    let command = intent(&plan, false);
    for seed in [false, true] {
        let context = DammBuyContext {
            wallet: plan.accounts.wallet,
            provider: SwqosType::Default,
            version: if seed {
                TradeTransactionVersion::V0
            } else {
                TradeTransactionVersion::V1
            },
            seed_optimized: seed,
        };
        assert!(
            DammBuyQuote::read(context, &command, &plan.accounts.pool, &read.client())
                .await
                .is_err()
        );
    }
    assert!(read.methods().is_empty());
}
