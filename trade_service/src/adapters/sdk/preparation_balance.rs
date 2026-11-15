use super::trade_command::ExecuteTrade;
use crate::domain::{
    preparation_balance::{reject_balance, PreparationBalanceSnapshot},
    trade::{Address, Pool, Side, WSOL_MINT},
};
use solana_sdk::pubkey::Pubkey;

pub(crate) fn require_canonical_mode(use_seed: bool) -> anyhow::Result<()> {
    if use_seed {
        return Err(reject_balance(
            "seed input account preparation balance is not audited",
        ));
    }
    Ok(())
}

pub(crate) fn balance_mint(trade: &ExecuteTrade, side: Side) -> Option<Pubkey> {
    if side == Side::Sell {
        return Some(trade.mint);
    }
    (matches!(
        trade.pool,
        Pool::PumpAmm | Pool::StonkFun | Pool::StonkFunSwap | Pool::RaydiumCpmm
    ) && trade.quote_mint.to_string() != WSOL_MINT
        && trade.sol_hop_pool_id.is_none())
    .then_some(trade.quote_mint)
}
pub(crate) fn validate(
    snapshot: Option<&PreparationBalanceSnapshot>,
    wallet: Address,
    trade: &ExecuteTrade,
    side: Side,
) -> anyhow::Result<()> {
    let Some(mint) = balance_mint(trade, side) else {
        if snapshot.is_some() {
            return Err(reject_balance("unexpected preparation balance snapshot"));
        }
        return Ok(());
    };
    let snapshot = snapshot
        .ok_or_else(|| reject_balance("required preparation balance snapshot is missing"))?;
    let balance = snapshot.checked_units(wallet, Address::from_bytes(mint.to_bytes()))?;
    if side == Side::Buy {
        let amount = trade
            .amount_lamports
            .ok_or_else(|| reject_balance("buy input amount is missing"))?;
        if amount == 0 || amount > balance {
            return Err(reject_balance("buy input exceeds preparation balance"));
        }
    } else {
        let percent = trade
            .sell_percent
            .ok_or_else(|| reject_balance("sell percent is missing"))?;
        if percent == 0 || percent > 100 {
            return Err(reject_balance("sell percent is invalid"));
        }
        let input = balance
            .checked_mul(percent)
            .ok_or_else(|| reject_balance("sell amount overflow before SDK entry"))?
            / 100;
        if input == 0 {
            return Err(reject_balance(
                "sell input amount rounds to zero before SDK entry",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
