use sol_trade_sdk::{
    common::{GasFeeStrategy, GasFeeStrategyType},
    swqos::{SwqosType, TradeType},
    trading::{core::params::DexParamEnum, factory::DexType},
    TradeBuyParams, TradeSellParams, TradeTokenType,
};

use crate::domain::trade::Pool;

use super::{
    protocol_params::quote_tokens::trade_token_type_for_sell, trade_command::ExecuteTrade,
};

/// Final request projection only; loaded protocol parameters are passed through unchanged.
pub(crate) fn ordinary_buy_params(
    trade: &ExecuteTrade,
    params: DexParamEnum,
    input_token_type: TradeTokenType,
    amount: u64,
    mint_account_exists: bool,
    lane: SwqosType,
) -> TradeBuyParams {
    TradeBuyParams {
        dex_type: dex_type(trade.pool),
        input_token_type,
        mint: trade.mint,
        input_token_amount: amount,
        slippage_basis_points: Some(trade.slippage_bps),
        recent_blockhash: Some(trade.recent_blockhash),
        extension_params: params,
        address_lookup_table_accounts: Vec::new(),
        wait_tx_confirmed: should_wait_lane_confirmation(lane, trade.wait_tx_confirmed),
        wait_for_all_submits: false,
        create_input_token_ata: true,
        close_input_token_ata: false,
        create_mint_ata: !mint_account_exists,
        durable_nonce: None,
        fixed_output_token_amount: match trade.pool {
            Pool::MeteoraDammV2 => Some(1),
            _ => None,
        },
        gas_fee_strategy: default_fee_strategy(lane, trade.tip_sol),
        simulate: trade.simulate,
        use_exact_sol_amount: Some(true),
        grpc_recv_us: None,
    }
}

pub(crate) fn ordinary_sell_params(
    trade: &ExecuteTrade,
    params: DexParamEnum,
    fixed_output_token_amount: Option<u64>,
    percent: u64,
    lane: SwqosType,
) -> anyhow::Result<TradeSellParams> {
    Ok(TradeSellParams {
        dex_type: dex_type(trade.pool),
        output_token_type: if trade.sol_hop_pool_id.is_some() {
            TradeTokenType::SOL
        } else {
            trade_token_type_for_sell(trade.pool, &trade.quote_mint)?
        },
        mint: trade.mint,
        input_token_amount: 0,
        slippage_basis_points: Some(trade.slippage_bps),
        recent_blockhash: Some(trade.recent_blockhash),
        with_tip: true,
        extension_params: params,
        address_lookup_table_accounts: Vec::new(),
        wait_tx_confirmed: should_wait_lane_confirmation(lane, trade.wait_tx_confirmed),
        wait_for_all_submits: false,
        create_output_token_ata: true,
        close_output_token_ata: false,
        close_mint_token_ata: percent == 100,
        durable_nonce: None,
        fixed_output_token_amount,
        gas_fee_strategy: if trade.simulate && trade.pool == Pool::MeteoraDammV2 {
            simulation_fee_strategy()
        } else {
            default_fee_strategy(lane, trade.tip_sol)
        },
        simulate: trade.simulate,
        grpc_recv_us: None,
    })
}

fn dex_type(pool: Pool) -> DexType {
    match pool {
        Pool::Pump => DexType::PumpFun,
        Pool::PumpAmm => DexType::PumpSwap,
        Pool::Bonk => DexType::Bonk,
        Pool::StonkFun | Pool::StonkFunSwap => DexType::StonkFun,
        Pool::MeteoraDammV2 => DexType::MeteoraDammV2,
        Pool::MeteoraDbc => DexType::MeteoraDbc,
        Pool::RaydiumAmmV4 => DexType::RaydiumAmmV4,
        Pool::RaydiumCpmm => DexType::RaydiumCpmm,
    }
}

pub(crate) fn default_fee_strategy(swqos_type: SwqosType, tip_sol: f64) -> GasFeeStrategy {
    let strategy = GasFeeStrategy::new();
    let set_strategy = |strategy: &GasFeeStrategy, lane: SwqosType, tip: f64| {
        strategy.set(
            lane,
            TradeType::Buy,
            GasFeeStrategyType::Normal,
            300_000,
            1_000,
            tip,
        );
        strategy.set(
            lane,
            TradeType::Sell,
            GasFeeStrategyType::Normal,
            300_000,
            1_000,
            tip,
        );
    };

    // SDK 6.x reads Default while simulating, including a non-Default submit lane.
    set_strategy(&strategy, SwqosType::Default, 0.0);
    if swqos_type != SwqosType::Default {
        set_strategy(&strategy, swqos_type, tip_sol);
    }
    strategy
}

pub(crate) fn simulation_fee_strategy() -> GasFeeStrategy {
    default_fee_strategy(SwqosType::Default, 0.0)
}

fn should_wait_lane_confirmation(swqos_type: SwqosType, wait_tx_confirmed: bool) -> bool {
    let _ = swqos_type;
    wait_tx_confirmed
}
