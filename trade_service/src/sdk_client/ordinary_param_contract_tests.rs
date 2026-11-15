//! Final request projection only: opaque public sentinels, no client, payer, RPC or SDK execution.
use super::{
    default_fee_strategy, simulation_fee_strategy, trade_token_type_for_buy, ExecuteTrade,
};
use crate::adapters::sdk::ordinary_params::{ordinary_buy_params, ordinary_sell_params};
use crate::domain::trade::{MevProvider, Pool, SOL_MINT, USD1_MINT, USDC_MINT, WSOL_MINT};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sol_trade_sdk::{
    common::{GasFeeStrategy, GasFeeStrategyType},
    swqos::{SwqosType, TradeType},
    trading::core::params::{BonkParams, DexParamEnum, TokenTransferFee},
    TradeBuyParams, TradeSellParams, TradeTokenType,
};
use solana_sdk::{hash::Hash, pubkey::Pubkey};

fn opaque_params() -> DexParamEnum {
    DexParamEnum::Bonk(BonkParams {
        virtual_base: 11,
        virtual_quote: 13,
        real_base: 17,
        real_quote: 19,
        total_base_sell: 23,
        pool_state: Pubkey::new_from_array([40; 32]),
        base_vault: Pubkey::new_from_array([41; 32]),
        quote_vault: Pubkey::new_from_array([42; 32]),
        mint_token_program: Pubkey::new_from_array([43; 32]),
        quote_mint: Pubkey::new_from_array([44; 32]),
        quote_token_program: Pubkey::new_from_array([45; 32]),
        platform_config: Pubkey::new_from_array([46; 32]),
        platform_associated_account: Pubkey::new_from_array([47; 32]),
        creator_associated_account: Pubkey::new_from_array([48; 32]),
        global_config: Pubkey::new_from_array([49; 32]),
        curve_type: 2,
        trade_fee_rate: 29,
        platform_fee_rate: 31,
        creator_fee_rate: 37,
        base_transfer_fee: TokenTransferFee {
            basis_points: 41,
            maximum_fee: 43,
        },
        quote_transfer_fee: TokenTransferFee {
            basis_points: 47,
            maximum_fee: 53,
        },
    })
}

fn opaque_snapshot(params: &DexParamEnum) -> Value {
    let DexParamEnum::Bonk(BonkParams {
        virtual_base,
        virtual_quote,
        real_base,
        real_quote,
        total_base_sell,
        pool_state,
        base_vault,
        quote_vault,
        mint_token_program,
        quote_mint,
        quote_token_program,
        platform_config,
        platform_associated_account,
        creator_associated_account,
        global_config,
        curve_type,
        trade_fee_rate,
        platform_fee_rate,
        creator_fee_rate,
        base_transfer_fee,
        quote_transfer_fee,
    }) = params
    else {
        panic!("opaque projection sentinel was replaced")
    };
    json!({
        "virtual_base":virtual_base, "virtual_quote":virtual_quote, "real_base":real_base,
        "real_quote":real_quote, "total_base_sell":total_base_sell,
        "pool_state":pool_state.to_string(), "base_vault":base_vault.to_string(), "quote_vault":quote_vault.to_string(),
        "mint_token_program":mint_token_program.to_string(), "quote_mint":quote_mint.to_string(),
        "quote_token_program":quote_token_program.to_string(), "platform_config":platform_config.to_string(),
        "platform_associated_account":platform_associated_account.to_string(),
        "creator_associated_account":creator_associated_account.to_string(), "global_config":global_config.to_string(),
        "curve_type":curve_type, "trade_fee_rate":trade_fee_rate, "platform_fee_rate":platform_fee_rate,
        "creator_fee_rate":creator_fee_rate,
        "base_transfer_fee":[base_transfer_fee.basis_points,base_transfer_fee.maximum_fee],
        "quote_transfer_fee":[quote_transfer_fee.basis_points,quote_transfer_fee.maximum_fee],
    })
}

fn fee_snapshot(strategy: &GasFeeStrategy) -> Value {
    let mut sides = vec![];
    for side in [TradeType::Buy, TradeType::Sell] {
        let mut rows = strategy
            .get_strategies(side)
            .into_iter()
            .map(|(lane, kind, fee)| {
                json!([
                    lane.as_str(),
                    kind.as_str(),
                    fee.cu_limit,
                    fee.cu_price,
                    fee.tip.to_bits()
                ])
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(Value::to_string);
        sides.push(rows);
    }
    json!(sides)
}

fn buy_snapshot(params: &TradeBuyParams) -> Value {
    let TradeBuyParams {
        dex_type,
        input_token_type,
        mint,
        input_token_amount,
        slippage_basis_points,
        recent_blockhash,
        extension_params,
        address_lookup_table_accounts,
        wait_tx_confirmed,
        wait_for_all_submits,
        create_input_token_ata,
        close_input_token_ata,
        create_mint_ata,
        durable_nonce,
        fixed_output_token_amount,
        gas_fee_strategy,
        simulate,
        use_exact_sol_amount,
        grpc_recv_us,
    } = params;
    json!({
        "dex_type":format!("{dex_type:?}"), "input_token_type":format!("{input_token_type:?}"),
        "mint":mint.to_string(), "input_token_amount":input_token_amount,
        "slippage_basis_points":slippage_basis_points, "recent_blockhash":recent_blockhash.map(|v|v.to_string()),
        "extension_params":opaque_snapshot(extension_params),
        "lookup_tables":address_lookup_table_accounts.iter().map(|v|json!({"key":v.key.to_string(),"addresses":v.addresses.iter().map(ToString::to_string).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "wait_tx_confirmed":wait_tx_confirmed, "wait_for_all_submits":wait_for_all_submits,
        "create_input_token_ata":create_input_token_ata, "close_input_token_ata":close_input_token_ata,
        "create_mint_ata":create_mint_ata, "durable_nonce_present":durable_nonce.is_some(),
        "fixed_output_token_amount":fixed_output_token_amount, "fees":fee_snapshot(gas_fee_strategy),
        "simulate":simulate, "use_exact_sol_amount":use_exact_sol_amount, "grpc_recv_us":grpc_recv_us,
    })
}

fn sell_snapshot(params: &TradeSellParams) -> Value {
    let TradeSellParams {
        dex_type,
        output_token_type,
        mint,
        input_token_amount,
        slippage_basis_points,
        recent_blockhash,
        with_tip,
        extension_params,
        address_lookup_table_accounts,
        wait_tx_confirmed,
        wait_for_all_submits,
        create_output_token_ata,
        close_output_token_ata,
        close_mint_token_ata,
        durable_nonce,
        fixed_output_token_amount,
        gas_fee_strategy,
        simulate,
        grpc_recv_us,
    } = params;
    json!({
        "dex_type":format!("{dex_type:?}"), "output_token_type":format!("{output_token_type:?}"),
        "mint":mint.to_string(), "input_token_amount":input_token_amount,
        "slippage_basis_points":slippage_basis_points, "recent_blockhash":recent_blockhash.map(|v|v.to_string()),
        "with_tip":with_tip, "extension_params":opaque_snapshot(extension_params),
        "lookup_tables":address_lookup_table_accounts.iter().map(|v|json!({"key":v.key.to_string(),"addresses":v.addresses.iter().map(ToString::to_string).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "wait_tx_confirmed":wait_tx_confirmed, "wait_for_all_submits":wait_for_all_submits,
        "create_output_token_ata":create_output_token_ata, "close_output_token_ata":close_output_token_ata,
        "close_mint_token_ata":close_mint_token_ata, "durable_nonce_present":durable_nonce.is_some(),
        "fixed_output_token_amount":fixed_output_token_amount, "fees":fee_snapshot(gas_fee_strategy),
        "simulate":simulate, "grpc_recv_us":grpc_recv_us,
    })
}

fn trade(pool: Pool, scenario: usize) -> ExecuteTrade {
    let custom = Pubkey::new_from_array([90; 32]).to_string();
    let quotes = [
        WSOL_MINT, SOL_MINT, USDC_MINT, USD1_MINT, &custom, USDC_MINT, &custom, WSOL_MINT,
    ];
    let hop = scenario % 3 == 0;
    ExecuteTrade {
        pool,
        mint: Pubkey::new_from_array([7; 32]),
        quote_mint: quotes[scenario].parse().unwrap(),
        pool_id: Some(Pubkey::new_from_array([8; 32])),
        sol_hop_pool: hop.then_some(Pool::RaydiumCpmm),
        sol_hop_pool_id: hop.then_some(Pubkey::new_from_array([9; 32])),
        amount_lamports: Some([1_000_001, (1u64 << 53) + 1, u64::MAX][scenario % 3]),
        sell_percent: Some(100),
        slippage_bps: [0, 1500, 9999][scenario % 3],
        tip_sol: if scenario & 1 == 0 { 0.0 } else { 0.00012345 },
        recent_blockhash: Hash::new_from_array([10; 32]),
        wait_tx_confirmed: scenario & 2 != 0,
        mev_submit_verify_ms: 17,
        simulate: scenario & 1 != 0,
        use_mev: Some(true),
        mev_provider: Some(MevProvider::Temporal),
    }
}

#[test]
fn final_sdk_params_v1_projection_baseline() {
    let mut cases = vec![];
    for pool in [
        Pool::Pump,
        Pool::PumpAmm,
        Pool::Bonk,
        Pool::StonkFun,
        Pool::StonkFunSwap,
        Pool::MeteoraDammV2,
        Pool::MeteoraDbc,
        Pool::RaydiumAmmV4,
        Pool::RaydiumCpmm,
    ] {
        for scenario in 0..8 {
            let trade = trade(pool, scenario);
            for lane in [
                SwqosType::Default,
                SwqosType::Jito,
                SwqosType::Temporal,
                SwqosType::FlashBlock,
                SwqosType::BlockRazor,
                SwqosType::Astralane,
                SwqosType::Speedlanding,
            ] {
                let input = if trade.sol_hop_pool_id.is_some() {
                    Ok(TradeTokenType::SOL)
                } else {
                    trade_token_type_for_buy(pool, &trade.quote_mint)
                };
                let result = match input {
                    Ok(input) => {
                        let params = ordinary_buy_params(
                            &trade,
                            opaque_params(),
                            input,
                            trade.amount_lamports.unwrap(),
                            scenario & 4 != 0,
                            lane,
                            (pool == Pool::MeteoraDammV2).then_some(1),
                        )
                        .unwrap();
                        assert_eq!(params.input_token_amount, trade.amount_lamports.unwrap());
                        assert_eq!(
                            opaque_snapshot(&params.extension_params),
                            opaque_snapshot(&opaque_params())
                        );
                        buy_snapshot(&params)
                    }
                    Err(err) => json!({"error":err.to_string()}),
                };
                cases.push(json!({"pool":pool,"scenario":scenario,"lane":lane.as_str(),"side":"buy","result":result}));
                for percent in [25, 99, 100] {
                    let output = (pool == Pool::MeteoraDammV2).then_some(9_876_543_210);
                    let result = match ordinary_sell_params(
                        &trade,
                        opaque_params(),
                        output,
                        percent,
                        lane,
                    ) {
                        Ok(params) => {
                            assert_eq!(params.input_token_amount, 0);
                            assert_eq!(params.close_mint_token_ata, percent == 100);
                            assert_eq!(
                                opaque_snapshot(&params.extension_params),
                                opaque_snapshot(&opaque_params())
                            );
                            sell_snapshot(&params)
                        }
                        Err(err) => json!({"error":err.to_string()}),
                    };
                    cases.push(json!({"pool":pool,"scenario":scenario,"lane":lane.as_str(),"side":"sell","percent":percent,"result":result}));
                }
            }
        }
    }
    assert_eq!(cases.len(), 2016);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&cases).unwrap())),
        "166aa348712cd4516bccbe10f132caf91acf8a2fb5aa734f06a9d38806255261",
    );
}

#[test]
fn final_fee_map_has_only_default_and_selected_lane_and_is_independent_per_request() {
    let first = default_fee_strategy(SwqosType::Temporal, 0.00012345);
    let second = default_fee_strategy(SwqosType::Temporal, 0.00012345);
    let before = fee_snapshot(&second);
    for side in [TradeType::Buy, TradeType::Sell] {
        let entries = first.get_strategies(side);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|(lane, kind, fee)| {
            *kind == GasFeeStrategyType::Normal
                && fee.cu_limit == 300_000
                && fee.cu_price == 1_000
                && ((*lane == SwqosType::Default && fee.tip.to_bits() == 0.0f64.to_bits())
                    || (*lane == SwqosType::Temporal
                        && fee.tip.to_bits() == 0.00012345f64.to_bits()))
        }));
    }
    first.set(
        SwqosType::Default,
        TradeType::Buy,
        GasFeeStrategyType::Normal,
        1,
        2,
        3.0,
    );
    assert_eq!(fee_snapshot(&second), before);
    assert_ne!(fee_snapshot(&first), before);
}

#[test]
fn damm_preview_replacement_keeps_original_live_fee_and_all_other_fields() {
    let mut trade = trade(Pool::MeteoraDammV2, 7);
    trade.simulate = false;
    let live =
        ordinary_sell_params(&trade, opaque_params(), Some(99), 100, SwqosType::Temporal).unwrap();
    let before = sell_snapshot(&live);
    let mut preview = live.clone();
    preview.simulate = true;
    preview.gas_fee_strategy = simulation_fee_strategy();
    assert_eq!(sell_snapshot(&live), before);
    assert_eq!(
        preview
            .gas_fee_strategy
            .get_strategies(TradeType::Sell)
            .len(),
        1
    );
    let mut expected = before;
    expected["simulate"] = json!(true);
    expected["fees"] = fee_snapshot(&simulation_fee_strategy());
    assert_eq!(sell_snapshot(&preview), expected);
}
