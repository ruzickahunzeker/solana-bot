use anyhow::{bail, Context};
use sol_trade_sdk::{
    swqos::SwqosType,
    trading::core::params::{DexParamEnum, MeteoraDammV2Params},
    TradeBuyParams, TradeSellParams, TradeTokenType,
};
use solana_sdk::pubkey::Pubkey;
use std::future::Future;

use crate::{domain::trade::Pool, protocol::meteora_damm_v2::minimum_output};

use super::{
    ordinary_params::{ordinary_buy_params, ordinary_sell_params},
    protocol_params::quote_tokens::{is_wsol_quote, trade_token_type_for_buy},
    trade_command::ExecuteTrade,
};

/// Preparation capabilities only. A resulting SDK request is not a signing authorization.
pub(crate) trait OrdinaryPreparationSource: Sync {
    fn load_params(
        &self,
        trade: &ExecuteTrade,
    ) -> impl Future<Output = anyhow::Result<DexParamEnum>> + Send;
    fn token_balance(&self, mint: Pubkey) -> impl Future<Output = anyhow::Result<u64>> + Send;
    fn mint_account_exists(
        &self,
        mint: Pubkey,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
    fn damm_sell_output(
        &self,
        trade: &ExecuteTrade,
        params: &MeteoraDammV2Params,
        input_amount: u64,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send;
    fn damm_buy_output(
        &self,
        trade: &ExecuteTrade,
        params: &MeteoraDammV2Params,
        input_amount: u64,
    ) -> impl Future<Output = anyhow::Result<u64>> + Send;
}

pub(crate) async fn buy(
    source: &impl OrdinaryPreparationSource,
    trade: &ExecuteTrade,
    lane: SwqosType,
) -> anyhow::Result<TradeBuyParams> {
    let mut params = source.load_params(trade).await?;
    let input_token_type = if trade.sol_hop_pool_id.is_some() {
        TradeTokenType::SOL
    } else {
        trade_token_type_for_buy(trade.pool, &trade.quote_mint)?
    };
    let amount = trade.amount_lamports.context("amount_lamports missing")?;
    if matches!(
        trade.pool,
        Pool::PumpAmm | Pool::StonkFun | Pool::StonkFunSwap | Pool::RaydiumCpmm
    ) && !is_wsol_quote(&trade.quote_mint)
        && trade.sol_hop_pool_id.is_none()
    {
        let balance = source.token_balance(trade.quote_mint).await?;
        if balance < amount {
            bail!("quote token balance insufficient: available {balance} units, requested {amount} units");
        }
    }
    let exists = source.mint_account_exists(trade.mint).await?;
    let fixed_output = if trade.pool == Pool::MeteoraDammV2 {
        if trade.sol_hop_pool.is_some()
            || trade.sol_hop_pool_id.is_some()
            || trade.sell_percent.is_some()
        {
            bail!("DAMM buy requires a direct buy-only intent");
        }
        let DexParamEnum::MeteoraDammV2(ref mut damm) = params else {
            bail!("DAMM v2 protocol params missing");
        };
        damm.swap_mode = 0;
        Some(
            source
                .damm_buy_output(trade, damm, amount)
                .await
                .and_then(|quote| minimum_output(quote, trade.slippage_bps))?,
        )
    } else {
        None
    };
    ordinary_buy_params(
        trade,
        params,
        input_token_type,
        amount,
        exists,
        lane,
        fixed_output,
    )
}

pub(crate) async fn sell(
    source: &impl OrdinaryPreparationSource,
    trade: &ExecuteTrade,
    lane: SwqosType,
) -> anyhow::Result<(TradeSellParams, u64, u64)> {
    let percent = trade.sell_percent.context("sell_percent missing")?;
    let balance = source.token_balance(trade.mint).await?;
    if balance == 0 {
        bail!("token balance is zero");
    }
    let mut params = source.load_params(trade).await?;
    let fixed_output = if trade.pool == Pool::MeteoraDammV2 {
        let DexParamEnum::MeteoraDammV2(ref mut damm) = params else {
            bail!("DAMM v2 protocol params missing");
        };
        // Preserve the legacy exact-in probe and its checked-u64 amount calculation.
        damm.swap_mode = 0;
        let input_amount = balance
            .checked_mul(percent)
            .context("sell amount overflow")?
            / 100;
        if input_amount == 0 {
            bail!("sell input amount rounds to zero");
        }
        Some(
            source
                .damm_sell_output(trade, damm, input_amount)
                .await
                .and_then(|quoted| minimum_output(quoted, trade.slippage_bps))?,
        )
    } else {
        None
    };
    let request = ordinary_sell_params(trade, params, fixed_output, percent, lane)?;
    Ok((request, balance, percent))
}

#[cfg(test)]
mod tests;
