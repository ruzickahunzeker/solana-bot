use crate::domain::trade::{Pool, USD1_MINT, USDC_MINT, WSOL_MINT};
use anyhow::bail;
use sol_trade_sdk::TradeTokenType;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

pub(crate) fn trade_token_type_for_buy(
    pool: Pool,
    quote_mint: &Pubkey,
) -> anyhow::Result<TradeTokenType> {
    if is_usdc_quote(quote_mint) {
        if pool == Pool::Bonk {
            bail!("bonk does not support USDC quote in this SDK; use WSOL or USD1");
        }
        return Ok(TradeTokenType::USDC);
    }
    if is_usd1_quote(quote_mint) {
        if !matches!(pool, Pool::Bonk | Pool::StonkFun | Pool::StonkFunSwap) {
            bail!("USD1 quote is only supported for bonk or stonkfun in this SDK");
        }
        return Ok(TradeTokenType::USD1);
    }
    if is_wsol_quote(quote_mint) {
        return Ok(match pool {
            Pool::Pump => TradeTokenType::SOL,
            Pool::PumpAmm
            | Pool::Bonk
            | Pool::StonkFun
            | Pool::StonkFunSwap
            | Pool::MeteoraDammV2
            | Pool::MeteoraDbc
            | Pool::RaydiumAmmV4
            | Pool::RaydiumCpmm => TradeTokenType::WSOL,
        });
    }
    if matches!(
        pool,
        Pool::PumpAmm | Pool::StonkFun | Pool::StonkFunSwap | Pool::RaydiumCpmm
    ) {
        return Ok(TradeTokenType::Token(*quote_mint));
    }
    bail!("unsupported quote_mint {quote_mint}")
}

pub(crate) fn trade_token_type_for_sell(
    pool: Pool,
    quote_mint: &Pubkey,
) -> anyhow::Result<TradeTokenType> {
    if is_usdc_quote(quote_mint) {
        if pool == Pool::Bonk {
            bail!("bonk does not support USDC quote in this SDK; use WSOL or USD1");
        }
        return Ok(TradeTokenType::USDC);
    }
    if is_usd1_quote(quote_mint) {
        if !matches!(pool, Pool::Bonk | Pool::StonkFun | Pool::StonkFunSwap) {
            bail!("USD1 quote is only supported for bonk or stonkfun in this SDK");
        }
        return Ok(TradeTokenType::USD1);
    }
    if is_wsol_quote(quote_mint) {
        return Ok(TradeTokenType::SOL);
    }
    if matches!(
        pool,
        Pool::PumpAmm | Pool::StonkFun | Pool::StonkFunSwap | Pool::RaydiumCpmm
    ) {
        return Ok(TradeTokenType::Token(*quote_mint));
    }
    bail!("unsupported quote_mint {quote_mint}")
}

pub(crate) fn is_wsol_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(WSOL_MINT)
}
pub(super) fn is_usd1_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(USD1_MINT)
}
fn is_usdc_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(USDC_MINT)
}
fn parse_known_pubkey(value: &str) -> Pubkey {
    Pubkey::from_str(value).expect("known mint pubkey must be valid")
}
