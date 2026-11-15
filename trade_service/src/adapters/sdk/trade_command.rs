use solana_sdk::{hash::Hash, pubkey::Pubkey};

use crate::{
    application::trade_command::TradeCommand,
    domain::{
        blockhash::Blockhash,
        trade::{MevProvider, Pool},
    },
};

#[derive(Clone)]
pub struct ExecuteTrade {
    pub pool: Pool,
    pub mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool_id: Option<Pubkey>,
    pub sol_hop_pool: Option<Pool>,
    pub sol_hop_pool_id: Option<Pubkey>,
    pub amount_lamports: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_bps: u64,
    pub tip_sol: f64,
    pub recent_blockhash: Hash,
    pub wait_tx_confirmed: bool,
    pub mev_submit_verify_ms: u64,
    pub simulate: bool,
    pub use_mev: Option<bool>,
    pub mev_provider: Option<MevProvider>,
}

pub(crate) fn sdk_trade(
    command: &TradeCommand,
    recent_blockhash: Blockhash,
    wait_tx_confirmed: bool,
    mev_submit_verify_ms: u64,
) -> ExecuteTrade {
    ExecuteTrade {
        pool: command.route.pool,
        mint: Pubkey::new_from_array(command.route.mint.to_bytes()),
        quote_mint: Pubkey::new_from_array(command.quote_mint().to_bytes()),
        pool_id: command
            .route
            .pool_id
            .map(|v| Pubkey::new_from_array(v.to_bytes())),
        sol_hop_pool: command.route.sol_hop_pool,
        sol_hop_pool_id: command
            .route
            .sol_hop_pool_id
            .map(|v| Pubkey::new_from_array(v.to_bytes())),
        amount_lamports: command.amount.buy_units(),
        sell_percent: command.amount.sell_percent(),
        slippage_bps: command.execution.slippage_bps,
        tip_sol: command.execution.tip_sol,
        recent_blockhash: Hash::new_from_array(recent_blockhash.to_bytes()),
        wait_tx_confirmed,
        mev_submit_verify_ms,
        simulate: command.execution.mode.simulate(),
        use_mev: command.execution.use_mev,
        mev_provider: command.execution.mev_provider,
    }
}
