use super::{preparation_balance, trade_command::ExecuteTrade};
use crate::domain::{
    preparation_balance::{reject_balance, PreparationBalanceSnapshot},
    trade::{Address, Pool, Side},
};
use anyhow::{ensure, Result};
use sol_trade_sdk::{
    constants::{SOL_TOKEN_ACCOUNT, USD1_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    trading::{
        core::params::{BonkParams, DexParamEnum, StonkFunMemeLeg, StonkFunSolHop},
        factory::DexType,
    },
    TradeBuyParams, TradeSellParams, TradeTokenType,
};
use solana_sdk::pubkey::Pubkey;

pub(crate) enum SdkInput<'a> {
    Buy(&'a TradeBuyParams),
    Sell(&'a TradeSellParams),
}

/// Binds the captured canonical input to the selected SDK parameters, not the final message.
pub(crate) fn validate(
    snapshot: Option<&PreparationBalanceSnapshot>,
    wallet: Address,
    trade: &ExecuteTrade,
    side: Side,
    request: SdkInput<'_>,
) -> Result<()> {
    preparation_balance::validate(snapshot, wallet, trade, side)?;
    let (dex, params, input) = match request {
        SdkInput::Buy(request) => {
            ensure!(
                side == Side::Buy
                    && request.mint == trade.mint
                    && Some(request.input_token_amount) == trade.amount_lamports
                    && request.simulate == trade.simulate,
                "prepared buy input does not match intent"
            );
            (
                request.dex_type,
                &request.extension_params,
                asset_mint(request.input_token_type),
            )
        }
        SdkInput::Sell(request) => {
            ensure!(
                side == Side::Sell
                    && request.mint == trade.mint
                    && request.simulate == trade.simulate,
                "prepared sell input does not match intent"
            );
            let output = if trade.sol_hop_pool_id.is_some() {
                WSOL_TOKEN_ACCOUNT
            } else {
                trade.quote_mint
            };
            ensure!(
                asset_mint(request.output_token_type) == output,
                "prepared sell output mint differs from intent"
            );
            (request.dex_type, &request.extension_params, request.mint)
        }
    };
    let Some(expected) = preparation_balance::balance_mint(trade, side) else {
        return Ok(());
    };
    ensure!(
        input == expected,
        "SDK input mint differs from captured balance mint"
    );
    let (program, decimals) = selected_program(trade, side, dex, params)?;
    let holding = snapshot.ok_or_else(|| reject_balance("preparation snapshot is missing"))?;
    let evidence = holding.checked_input(wallet, Address::from_bytes(input.to_bytes()))?;
    ensure!(
        evidence.program.to_bytes() == program.to_bytes(),
        "SDK input program differs from raw mint owner"
    );
    let canonical_decimals = if input == WSOL_TOKEN_ACCOUNT {
        Some(9)
    } else if input == USDC_TOKEN_ACCOUNT || input == USD1_TOKEN_ACCOUNT {
        Some(6)
    } else {
        None
    };
    ensure!(
        canonical_decimals.is_none_or(
            |decimals| program == spl_token_interface::ID && evidence.decimals == decimals
        ),
        "SDK canonical input asset metadata is invalid"
    );
    ensure!(
        decimals.is_none_or(|decimals| decimals == evidence.decimals),
        "SDK input precision differs from raw mint"
    );
    let account = spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
        &Pubkey::new_from_array(wallet.to_bytes()), &input, &program,
    );
    ensure!(
        evidence.account.to_bytes() == account.to_bytes(),
        "SDK canonical input differs from captured raw account"
    );
    Ok(())
}

pub(crate) fn validate_preview(
    snapshot: Option<&PreparationBalanceSnapshot>,
    wallet: Address,
    trade: &ExecuteTrade,
    request: &TradeSellParams,
) -> Result<()> {
    ensure!(
        trade.pool == Pool::MeteoraDammV2 && !trade.simulate && request.simulate,
        "SDK preview must be the DAMM non-broadcast request"
    );
    let mut preview_intent = trade.clone();
    preview_intent.simulate = true;
    validate(
        snapshot,
        wallet,
        &preview_intent,
        Side::Sell,
        SdkInput::Sell(request),
    )
}

fn asset_mint(asset: TradeTokenType) -> Pubkey {
    match asset {
        TradeTokenType::SOL | TradeTokenType::WSOL => WSOL_TOKEN_ACCOUNT,
        TradeTokenType::USDC => USDC_TOKEN_ACCOUNT,
        TradeTokenType::USD1 => USD1_TOKEN_ACCOUNT,
        TradeTokenType::Token(mint) => normalize(mint),
    }
}
fn normalize(mint: Pubkey) -> Pubkey {
    if mint == SOL_TOKEN_ACCOUNT {
        WSOL_TOKEN_ACCOUNT
    } else {
        mint
    }
}
fn supported(program: Pubkey) -> Result<Pubkey> {
    ensure!(
        [spl_token_interface::ID, spl_token_2022_interface::ID].contains(&program),
        "SDK input token program is unsupported or missing"
    );
    Ok(program)
}
fn pair(
    input: Pubkey,
    output: Pubkey,
    a: Pubkey,
    b: Pubkey,
    ap: Pubkey,
    bp: Pubkey,
) -> Result<(Pubkey, Pubkey)> {
    ensure!(a != b && input != output, "SDK input pair aliases itself");
    let programs = if input == a && output == b {
        (ap, bp)
    } else if input == b && output == a {
        (bp, ap)
    } else {
        return Err(reject_balance("SDK pool mints differ from input intent"));
    };
    Ok((supported(programs.0)?, supported(programs.1)?))
}
fn curve_programs(params: &BonkParams, quote: Pubkey) -> Result<(Pubkey, Pubkey)> {
    let actual_quote = if params.quote_mint != Pubkey::default() {
        params.quote_mint
    } else if params.global_config
        == sol_trade_sdk::instruction::utils::bonk::accounts::USD1_GLOBAL_CONFIG
    {
        USD1_TOKEN_ACCOUNT
    } else {
        WSOL_TOKEN_ACCOUNT
    };
    ensure!(
        actual_quote == quote,
        "SDK curve quote differs from input intent"
    );
    Ok((
        supported(params.mint_token_program)?,
        supported(if params.quote_token_program == Pubkey::default() {
            spl_token_interface::ID
        } else {
            params.quote_token_program
        })?,
    ))
}

fn selected_program(
    trade: &ExecuteTrade,
    side: Side,
    dex: DexType,
    params: &DexParamEnum,
) -> Result<(Pubkey, Option<u8>)> {
    ensure!(
        trade.sol_hop_pool.is_some() == trade.sol_hop_pool_id.is_some(),
        "incomplete SDK input hop intent"
    );
    ensure!(
        trade.mint != trade.quote_mint,
        "SDK input pair aliases itself"
    );
    let (input, output) = if side == Side::Buy {
        (trade.quote_mint, trade.mint)
    } else {
        (trade.mint, trade.quote_mint)
    };
    if let Some(hop_id) = trade.sol_hop_pool_id {
        ensure!(
            dex == DexType::StonkFun && trade.sol_hop_pool == Some(Pool::RaydiumCpmm),
            "SDK input hop backend differs from intent"
        );
        let DexParamEnum::StonkFunViaSol(wrapper) = params else {
            return Err(reject_balance("SDK input hop wrapper is missing"));
        };
        let meme_programs = match (&trade.pool, &wrapper.meme_leg) {
            (Pool::StonkFun, StonkFunMemeLeg::Curve(params)) => {
                curve_programs(params, trade.quote_mint)?
            }
            (Pool::StonkFunSwap, StonkFunMemeLeg::Graduated(params)) => pair(
                trade.mint,
                trade.quote_mint,
                params.base_mint,
                params.quote_mint,
                params.base_token_program,
                params.quote_token_program,
            )?,
            _ => {
                return Err(reject_balance(
                    "SDK meme endpoint differs from input intent",
                ))
            }
        };
        let StonkFunSolHop::RaydiumCpmm(hop) = &wrapper.sol_hop else {
            return Err(reject_balance("SDK input hop venue is not audited"));
        };
        ensure!(
            hop.pool_state == hop_id && side == Side::Sell,
            "SDK input hop identity differs from intent"
        );
        let hop_programs = pair(
            trade.quote_mint,
            WSOL_TOKEN_ACCOUNT,
            hop.base_mint,
            hop.quote_mint,
            hop.base_token_program,
            hop.quote_token_program,
        )?;
        ensure!(
            meme_programs.1 == hop_programs.0 && hop_programs.1 == spl_token_interface::ID,
            "SDK input hop token programs disagree"
        );
        return Ok((meme_programs.0, None));
    }
    let selected = match (trade.pool, dex, params) {
        (Pool::Pump, DexType::PumpFun, DexParamEnum::PumpFun(params)) => {
            let curve_quote = params.bonding_curve.effective_quote_mint();
            let quote = if params.quote_mint != Pubkey::default() {
                normalize(params.quote_mint)
            } else if curve_quote != Pubkey::default() {
                normalize(curve_quote)
            } else {
                WSOL_TOKEN_ACCOUNT
            };
            ensure!(
                quote == trade.quote_mint,
                "SDK Pump quote differs from input intent"
            );
            if trade.mint.to_string().ends_with("pump") || params.token_program == Pubkey::default()
            {
                spl_token_2022_interface::ID
            } else {
                supported(params.token_program)?
            }
        }
        (Pool::PumpAmm, DexType::PumpSwap, DexParamEnum::PumpSwap(params)) => {
            pair(
                input,
                output,
                params.base_mint,
                params.quote_mint,
                params.base_token_program,
                params.quote_token_program,
            )?
            .0
        }
        (Pool::Bonk, DexType::Bonk, DexParamEnum::Bonk(params))
        | (Pool::StonkFun, DexType::StonkFun, DexParamEnum::StonkFun(params)) => {
            let programs = curve_programs(params, trade.quote_mint)?;
            if side == Side::Sell {
                programs.0
            } else {
                programs.1
            }
        }
        (Pool::StonkFunSwap, DexType::StonkFun, DexParamEnum::StonkFunSwap(params))
        | (Pool::RaydiumCpmm, DexType::RaydiumCpmm, DexParamEnum::RaydiumCpmm(params)) => {
            pair(
                input,
                output,
                params.base_mint,
                params.quote_mint,
                params.base_token_program,
                params.quote_token_program,
            )?
            .0
        }
        (Pool::MeteoraDammV2, DexType::MeteoraDammV2, DexParamEnum::MeteoraDammV2(params)) => {
            pair(
                input,
                output,
                params.token_a_mint,
                params.token_b_mint,
                params.token_a_program,
                params.token_b_program,
            )?
            .0
        }
        (Pool::MeteoraDbc, DexType::MeteoraDbc, DexParamEnum::MeteoraDbc(params)) => {
            ensure!(
                params.base_mint == trade.mint
                    && params.quote_mint == trade.quote_mint
                    && params.pool_config.quote_mint == trade.quote_mint,
                "SDK DBC endpoint differs from input intent"
            );
            let program = pair(
                input,
                output,
                params.base_mint,
                params.quote_mint,
                params.base_token_program,
                params.quote_token_program,
            )?
            .0;
            return Ok((
                program,
                (side == Side::Sell).then_some(params.pool_config.token_decimal),
            ));
        }
        (Pool::RaydiumAmmV4, DexType::RaydiumAmmV4, DexParamEnum::RaydiumAmmV4(params)) => {
            pair(
                input,
                output,
                params.coin_mint,
                params.pc_mint,
                spl_token_interface::ID,
                spl_token_interface::ID,
            )?
            .0
        }
        _ => {
            return Err(reject_balance(
                "SDK input protocol parameters differ from intent",
            ))
        }
    };
    Ok((supported(selected)?, None))
}

#[cfg(test)]
mod tests;
