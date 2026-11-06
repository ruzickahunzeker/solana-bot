//! Read-only Pump.fun token-quote account snapshots and narrowly protected
//! USDC V3 quotes. Neither grants a trade or aggregator execution decision.

use anyhow::{ensure, Context, Result};
use sol_trade_sdk::common::bonding_curve::BondingCurveAccount;
use sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast;
use sol_trade_sdk::instruction::pump_v3_quote::{quote_pump_buy_v3_exact_in, PumpV3QuoteState};
use sol_trade_sdk::instruction::utils::{pumpfun::accounts, pumpswap};
use sol_trade_sdk::trading::core::params::TokenTransferFee;
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::{account::Account, pubkey::Pubkey};
use spl_token_2022_interface::{
    extension::{
        transfer_fee::TransferFeeConfig, BaseStateWithExtensions, ExtensionType,
        StateWithExtensions,
    },
    state::{Account as Token2022Account, Mint as Token2022Mint},
};
use spl_token_interface::state::{Account as LegacyTokenAccount, Mint as LegacyMint};
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub(crate) struct PumpFunGlobalFees {
    pub token_total_supply: u64,
    pub fallback_protocol_bps: u64,
    pub fallback_creator_bps: u64,
    pub whitelisted_quote_mint: Pubkey,
    pub creator_fee_configurable: bool,
    pub max_configurable_creator_fee_bps: u64,
    pub max_curve_depth: Option<u8>,
}

pub(crate) struct PumpFunQuoteAccountsSnapshot {
    pub slot: u64,
    pub epoch: u64,
    pub global: PumpFunGlobalFees,
    pub config: pumpswap::PumpSwapFeeConfig,
    pub curve: BondingCurveAccount,
    pub base_supply: u64,
    pub quote_supply: u64,
    pub base_vault_amount: u64,
    pub quote_vault_amount: u64,
    pub base_transfer_fee: TokenTransferFee,
    pub quote_transfer_fee: TokenTransferFee,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PumpFunProtectedQuote {
    pub expected_base_out: u64,
    pub minimum_base_out: u64,
    pub protocol_bps: u64,
    pub creator_bps: u64,
    pub validated_slot: u64,
}

impl PumpFunQuoteAccountsSnapshot {
    /// Current narrow V3 scope: a USDC-whitelisted active curve and one stable
    /// tier beginning at zero. Other schedules remain unresolved until their
    /// market-cap conversion and quote eligibility are independently proved.
    pub(crate) fn quote_usdc_v3_exact_in(
        &self,
        quote_mint: Pubkey,
        quote_budget: u64,
        slippage_bps: u64,
    ) -> Result<PumpFunProtectedQuote> {
        ensure!(
            quote_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT
                && self.global.whitelisted_quote_mint == quote_mint,
            "Pump.fun V3 quote mint is not the verified USDC whitelist entry"
        );
        ensure!(
            quote_budget > 0 && slippage_bps < 10_000,
            "invalid Pump.fun quote amount or slippage"
        );
        ensure!(
            self.base_transfer_fee.basis_points == 0 && self.quote_transfer_fee.basis_points == 0,
            "Pump.fun V3 quote requires zero Token-2022 transfer fees"
        );
        ensure!(
            !self.curve.is_cashback_coin,
            "Pump.fun V3 does not support cashback curves"
        );
        ensure!(
            self.curve.effective_quote_mint() == quote_mint,
            "Pump.fun V3 curve quote mint changed"
        );
        ensure!(
            !self.global.creator_fee_configurable || self.curve.creator_fee_bps == 0,
            "Pump.fun V3 configurable creator fee needs a verified fee override"
        );
        ensure!(
            self.base_vault_amount >= self.curve.real_token_reserves
                && self.base_supply >= self.curve.real_token_reserves
                && self.quote_supply > 0
                && self.quote_vault_amount >= self.curve.real_sol_reserves,
            "Pump.fun curve mint or vault balance is inconsistent"
        );
        let [tier] = self.config.stable_fee_tiers.as_slice() else {
            anyhow::bail!("Pump.fun USDC fee schedule needs a single verified stable tier")
        };
        ensure!(
            tier.market_cap_lamports_threshold == 0,
            "Pump.fun USDC fee tier requires market-cap selection"
        );
        ensure!(
            tier.fees.lp_fee_basis_points == 0,
            "Pump.fun curve fee tier has an unexpected LP fee"
        );
        let protocol_bps = tier.fees.protocol_fee_basis_points;
        let creator_bps = if self.curve.creator == Pubkey::default() {
            0
        } else {
            tier.fees.coin_creator_fee_basis_points
        };
        ensure!(
            protocol_bps
                .checked_add(creator_bps)
                .is_some_and(|sum| sum <= 10_000),
            "Pump.fun V3 fee rates are invalid"
        );
        let quote = quote_pump_buy_v3_exact_in(
            &PumpV3QuoteState {
                virtual_base: self.curve.virtual_token_reserves,
                virtual_quote: self.curve.virtual_sol_reserves,
                remaining_base: self.curve.real_token_reserves,
                real_quote: self.curve.real_sol_reserves,
                curve_base_balance: self.base_vault_amount,
                migration_fee: 0,
                protocol_bps,
                creator_bps,
                complete: self.curve.complete,
                mayhem: self.curve.is_mayhem_mode,
            },
            quote_budget,
        )?;
        ensure!(
            quote.pool_base == 0
                && quote.base_out > 0
                && quote.base_out < self.curve.real_token_reserves
                && quote.quote_in <= quote_budget,
            "Pump.fun V3 quote reaches graduation or has no protected fill"
        );
        let minimum_base_out =
            u64::try_from((quote.base_out as u128) * (10_000 - slippage_bps) as u128 / 10_000)?;
        ensure!(
            minimum_base_out > 0,
            "Pump.fun V3 quote minimum rounds to zero"
        );
        Ok(PumpFunProtectedQuote {
            expected_base_out: quote.base_out,
            minimum_base_out,
            protocol_bps,
            creator_bps,
            validated_slot: self.slot,
        })
    }
}

pub(crate) fn decode_pumpfun_global(account: &Account) -> Result<PumpFunGlobalFees> {
    ensure!(
        account.owner == accounts::PUMPFUN,
        "Pump.fun Global owner does not match Pump program"
    );
    let data = &account.data;
    ensure!(
        data.starts_with(&[167, 232, 232, 177, 200, 108, 114, 127]),
        "Pump.fun Global discriminator is invalid"
    );
    // Offsets follow the fixed-size Global layout in the vendored Pump IDL.
    // The final max_curve_depth was appended after the 1087-byte legacy body.
    ensure!(data.len() >= 1087, "Pump.fun Global account is truncated");
    ensure!(
        data[1088.min(data.len())..].iter().all(|byte| *byte == 0),
        "Pump.fun Global has unknown nonzero trailing fields"
    );
    let read_u64 = |offset| -> Result<u64> {
        Ok(u64::from_le_bytes(
            data[offset..offset + 8]
                .try_into()
                .expect("validated Global length"),
        ))
    };
    let read_bool = |offset| -> Result<bool> {
        match data[offset] {
            0 => Ok(false),
            1 => Ok(true),
            _ => anyhow::bail!("Pump.fun Global has an invalid boolean field"),
        }
    };
    ensure!(read_bool(8)?, "Pump.fun Global is not initialized");
    let token_total_supply = read_u64(97)?;
    let fallback_protocol_bps = read_u64(105)?;
    let fallback_creator_bps = read_u64(154)?;
    let creator_fee_configurable = read_bool(1045)?;
    let max_configurable_creator_fee_bps = read_u64(1046)?;
    ensure!(
        token_total_supply > 0
            && fallback_protocol_bps
                .checked_add(fallback_creator_bps)
                .is_some_and(|sum| sum <= 10_000)
            && max_configurable_creator_fee_bps <= 10_000,
        "Pump.fun Global fee or supply fields are invalid"
    );
    let whitelisted_quote_mint = Pubkey::new_from_array(
        data[1013..1045]
            .try_into()
            .expect("validated Global length"),
    );
    Ok(PumpFunGlobalFees {
        token_total_supply,
        fallback_protocol_bps,
        fallback_creator_bps,
        whitelisted_quote_mint,
        creator_fee_configurable,
        max_configurable_creator_fee_bps,
        max_curve_depth: data.get(1087).copied(),
    })
}

pub(crate) async fn load_pumpfun_quote_accounts(
    rpc: &sol_trade_sdk::common::SolanaRpcClient,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    base_program: Pubkey,
    quote_program: Pubkey,
) -> Result<PumpFunQuoteAccountsSnapshot> {
    ensure!(
        fee_config_address_is_canonical(),
        "Pump.fun FeeConfig address is not canonical"
    );
    let supported_program = |program| {
        program == sol_trade_sdk::common::spl_token::ID
            || program == sol_trade_sdk::common::spl_token_2022::ID
    };
    ensure!(
        supported_program(base_program) && supported_program(quote_program),
        "Pump.fun token-quote mint owner is unsupported"
    );
    ensure!(
        quote_mint != sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
            && quote_mint != sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT,
        "Pump.fun token-quote snapshot requires a non-native quote"
    );
    let curve_address =
        sol_trade_sdk::instruction::utils::pumpfun::get_bonding_curve_pda(&base_mint)
            .context("Pump.fun curve PDA is unavailable")?;
    let base_vault = get_associated_token_address_with_program_id_fast(
        &curve_address,
        &base_mint,
        &base_program,
    );
    let quote_vault = get_associated_token_address_with_program_id_fast(
        &curve_address,
        &quote_mint,
        &quote_program,
    );
    let (epoch_before, response, epoch_after) =
        tokio::time::timeout(Duration::from_secs(8), async {
            let before = rpc.get_epoch_info().await?;
            let response = rpc.get_multiple_accounts_with_commitment(
            &[
                sol_trade_sdk::instruction::utils::pumpfun::global_constants::GLOBAL_ACCOUNT,
                accounts::FEE_CONFIG,
                curve_address,
                base_mint,
                quote_mint,
                base_vault,
                quote_vault,
            ],
            CommitmentConfig::processed(),
        ).await?;
            let after = rpc.get_epoch_info().await?;
            Ok::<_, anyhow::Error>((before, response, after))
        })
        .await
        .context("Pump.fun quote account RPC timed out")?
        .map_err(|error| anyhow::anyhow!("{}", crate::sanitize::redact_urls(&error.to_string())))?;
    ensure!(
        epoch_before.epoch == epoch_after.epoch,
        "Pump.fun token fee epoch changed during snapshot"
    );
    let epoch_start = epoch_before
        .absolute_slot
        .checked_sub(epoch_before.slot_index)
        .context("Pump.fun epoch start underflows")?;
    let epoch_end = epoch_start
        .checked_add(epoch_before.slots_in_epoch)
        .context("Pump.fun epoch end overflows")?;
    ensure!(
        (epoch_start..epoch_end).contains(&response.context.slot),
        "Pump.fun account snapshot is outside the token fee epoch"
    );
    let account = |index: usize, name: &'static str| -> Result<&Account> {
        response
            .value
            .get(index)
            .and_then(Option::as_ref)
            .with_context(|| format!("Pump.fun {name} account is missing"))
    };
    let global = decode_pumpfun_global(account(0, "Global")?)?;
    let config = decode_pumpfun_fee_config(account(1, "FeeConfig")?)?;
    let curve = decode_pumpfun_curve(account(2, "curve")?, curve_address, base_mint, quote_mint)?;
    let (base_supply, base_transfer_fee) =
        decode_mint_fee(account(3, "base mint")?, base_program, epoch_before.epoch)?;
    let (quote_supply, quote_transfer_fee) =
        decode_mint_fee(account(4, "quote mint")?, quote_program, epoch_before.epoch)?;
    let base_vault_amount = decode_curve_vault(
        account(5, "base vault")?,
        base_program,
        base_mint,
        curve_address,
    )?;
    let quote_vault_amount = decode_curve_vault(
        account(6, "quote vault")?,
        quote_program,
        quote_mint,
        curve_address,
    )?;
    Ok(PumpFunQuoteAccountsSnapshot {
        slot: response.context.slot,
        epoch: epoch_before.epoch,
        global,
        config,
        curve,
        base_supply,
        quote_supply,
        base_vault_amount,
        quote_vault_amount,
        base_transfer_fee,
        quote_transfer_fee,
    })
}

fn decode_pumpfun_curve(
    account: &Account,
    curve_address: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
) -> Result<BondingCurveAccount> {
    ensure!(
        account.owner == accounts::PUMPFUN,
        "Pump.fun curve owner is invalid"
    );
    ensure!(
        account
            .data
            .starts_with(&[23, 183, 248, 55, 96, 216, 172, 96]),
        "Pump.fun curve discriminator is invalid"
    );
    let mut curve = BondingCurveAccount::decode_body(&account.data[8..])
        .context("Pump.fun curve body is invalid")?;
    curve.account = curve_address;
    ensure!(
        !curve.complete
            && curve.effective_quote_mint() == quote_mint
            && quote_mint != base_mint
            && curve.virtual_token_reserves > 0
            && curve.virtual_sol_reserves > 0
            && curve.real_token_reserves > 0,
        "Pump.fun curve is complete or does not match the token-quote route"
    );
    Ok(curve)
}

fn decode_mint_fee(
    account: &Account,
    token_program: Pubkey,
    epoch: u64,
) -> Result<(u64, TokenTransferFee)> {
    ensure!(
        account.owner == token_program,
        "Pump.fun mint token program changed"
    );
    if token_program == sol_trade_sdk::common::spl_token::ID {
        let mint = LegacyMint::unpack(&account.data).context("invalid SPL mint")?;
        return Ok((mint.supply, TokenTransferFee::default()));
    }
    ensure!(
        token_program == sol_trade_sdk::common::spl_token_2022::ID,
        "unsupported Pump.fun mint token program"
    );
    let mint = StateWithExtensions::<Token2022Mint>::unpack(&account.data)
        .context("invalid Token-2022 mint")?;
    ensure!(
        spl_token_2022_interface::extension::transfer_hook::get_program_id(&mint).is_none(),
        "Pump.fun mint has an unsupported transfer hook"
    );
    let transfer_fee = if mint
        .get_extension_types()
        .context("invalid Token-2022 mint extensions")?
        .contains(&ExtensionType::TransferFeeConfig)
    {
        let config = mint
            .get_extension::<TransferFeeConfig>()
            .context("invalid Token-2022 transfer fee configuration")?;
        let active = config.get_epoch_fee(epoch);
        let basis_points = u16::from(active.transfer_fee_basis_points);
        ensure!(
            basis_points <= 10_000,
            "invalid Token-2022 transfer fee basis points"
        );
        TokenTransferFee {
            basis_points,
            maximum_fee: active.maximum_fee.into(),
        }
    } else {
        TokenTransferFee::default()
    };
    Ok((mint.base.supply, transfer_fee))
}

fn decode_curve_vault(
    account: &Account,
    token_program: Pubkey,
    mint: Pubkey,
    curve_address: Pubkey,
) -> Result<u64> {
    ensure!(
        account.owner == token_program,
        "Pump.fun curve vault program changed"
    );
    let (account_mint, account_owner, amount) =
        if token_program == sol_trade_sdk::common::spl_token::ID {
            let token = LegacyTokenAccount::unpack(&account.data)
                .context("invalid SPL Pump.fun curve vault")?;
            (token.mint, token.owner, token.amount)
        } else if token_program == sol_trade_sdk::common::spl_token_2022::ID {
            let token = StateWithExtensions::<Token2022Account>::unpack(&account.data)
                .context("invalid Token-2022 Pump.fun curve vault")?;
            (token.base.mint, token.base.owner, token.base.amount)
        } else {
            anyhow::bail!("unsupported Pump.fun curve vault token program")
        };
    ensure!(
        account_mint == mint && account_owner == curve_address,
        "Pump.fun curve vault mint or authority is invalid"
    );
    Ok(amount)
}

pub(crate) fn decode_pumpfun_fee_config(account: &Account) -> Result<pumpswap::PumpSwapFeeConfig> {
    ensure!(
        account.owner == accounts::FEE_PROGRAM,
        "Pump.fun FeeConfig owner does not match pump-fees program"
    );
    // The Pump and PumpSwap FeeConfig types have the same Anchor discriminator
    // and Borsh field layout in the vendored IDLs. The SDK's PumpSwap decoder
    // accepts legacy truncated accounts, so require the current Pump layout
    // (including stable tiers and exotic fees) before reusing it here.
    let data = &account.data;
    let mut offset = 8_usize + 1 + 32 + 24; // discriminator, bump, admin, flat fees
    for _ in 0..2 {
        let length = data
            .get(offset..offset + 4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes")) as usize)
            .context("Pump.fun FeeConfig tier vector is missing")?;
        offset = offset
            .checked_add(4)
            .and_then(|value| {
                length
                    .checked_mul(40)
                    .and_then(|bytes| value.checked_add(bytes))
            })
            .context("Pump.fun FeeConfig tier length overflows")?;
        ensure!(
            offset <= data.len(),
            "Pump.fun FeeConfig tier data is truncated"
        );
    }
    let logical_end = offset
        .checked_add(24)
        .context("Pump.fun FeeConfig exotic fee boundary overflows")?;
    ensure!(
        logical_end <= data.len(),
        "Pump.fun FeeConfig exotic fees are truncated"
    );
    // Mainnet currently allocates 4097 bytes but uses only the Borsh prefix.
    // Reject nonzero tail data rather than silently skipping future fields.
    ensure!(
        data[logical_end..].iter().all(|byte| *byte == 0),
        "Pump.fun FeeConfig has unknown nonzero trailing fields"
    );
    let config = pumpswap::decode_fee_config(data)
        .context("Pump.fun FeeConfig discriminator or fields are invalid")?;
    for fees in std::iter::once(&config.flat_fees)
        .chain(config.fee_tiers.iter().map(|tier| &tier.fees))
        .chain(config.stable_fee_tiers.iter().map(|tier| &tier.fees))
        .chain(std::iter::once(&config.exotic_flat_fees))
    {
        ensure!(
            fees.lp_fee_basis_points
                .checked_add(fees.protocol_fee_basis_points)
                .and_then(|sum| sum.checked_add(fees.coin_creator_fee_basis_points))
                .is_some_and(|sum| sum <= 10_000),
            "Pump.fun FeeConfig has invalid fee basis points"
        );
    }
    Ok(config)
}

pub(crate) fn fee_config_address_is_canonical() -> bool {
    let (address, _) = Pubkey::find_program_address(
        &[b"fee_config", accounts::PUMPFUN.as_ref()],
        &accounts::FEE_PROGRAM,
    );
    address == accounts::FEE_CONFIG
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(owner: Pubkey, data: Vec<u8>) -> Account {
        Account {
            lamports: 1,
            data,
            owner,
            executable: false,
            rent_epoch: 0,
        }
    }

    fn current_fee_config_data() -> Vec<u8> {
        let mut data = vec![143, 52, 146, 187, 219, 123, 76, 155];
        data.push(1); // bump
        data.extend_from_slice(&[0_u8; 32]); // admin
        for bps in [0_u64, 95, 30] {
            data.extend_from_slice(&bps.to_le_bytes());
        }
        data.extend_from_slice(&0_u32.to_le_bytes()); // fee tiers
        data.extend_from_slice(&0_u32.to_le_bytes()); // stable tiers
        for bps in [0_u64, 95, 30] {
            data.extend_from_slice(&bps.to_le_bytes());
        }
        data
    }

    fn current_global_data() -> Vec<u8> {
        let mut data = vec![0_u8; 1088];
        data[..8].copy_from_slice(&[167, 232, 232, 177, 200, 108, 114, 127]);
        data[8] = 1; // initialized
        data[97..105].copy_from_slice(&1_000_000_000_000_000_u64.to_le_bytes());
        data[105..113].copy_from_slice(&95_u64.to_le_bytes());
        data[154..162].copy_from_slice(&30_u64.to_le_bytes());
        data[1013..1045].copy_from_slice(sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT.as_ref());
        data[1045] = 1; // creator fee configurable
        data[1046..1054].copy_from_slice(&300_u64.to_le_bytes());
        data[1087] = 5; // max curve depth
        data
    }

    #[test]
    fn pump_global_decodes_current_and_legacy_tail_but_rejects_unknown_fields() {
        let data = current_global_data();
        let decoded = decode_pumpfun_global(&account(accounts::PUMPFUN, data.clone())).unwrap();
        assert_eq!(decoded.token_total_supply, 1_000_000_000_000_000);
        assert_eq!(
            decoded.whitelisted_quote_mint,
            sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT
        );
        assert_eq!(decoded.max_curve_depth, Some(5));
        let legacy =
            decode_pumpfun_global(&account(accounts::PUMPFUN, data[..1087].to_vec())).unwrap();
        assert_eq!(legacy.max_curve_depth, None);
        assert!(decode_pumpfun_global(&account(accounts::FEE_PROGRAM, data.clone())).is_err());
        let mut invalid = data.clone();
        invalid[1045] = 2;
        assert!(decode_pumpfun_global(&account(accounts::PUMPFUN, invalid)).is_err());
        let mut future = data;
        future.push(1);
        assert!(decode_pumpfun_global(&account(accounts::PUMPFUN, future)).is_err());
    }

    #[test]
    fn pump_fee_config_requires_canonical_owner_and_complete_layout() {
        assert!(fee_config_address_is_canonical());
        let data = current_fee_config_data();
        let decoded = decode_pumpfun_fee_config(&account(accounts::FEE_PROGRAM, data.clone()))
            .expect("current Pump FeeConfig layout");
        assert_eq!(decoded.flat_fees.protocol_fee_basis_points, 95);
        assert!(decode_pumpfun_fee_config(&account(accounts::PUMPFUN, data.clone())).is_err());
        assert!(decode_pumpfun_fee_config(&account(
            accounts::FEE_PROGRAM,
            data[..data.len() - 1].to_vec()
        ))
        .is_err());
        let mut wrong_discriminator = data;
        wrong_discriminator[0] = 0;
        assert!(
            decode_pumpfun_fee_config(&account(accounts::FEE_PROGRAM, wrong_discriminator))
                .is_err()
        );
        let mut padded = current_fee_config_data();
        padded.resize(4097, 0);
        assert!(decode_pumpfun_fee_config(&account(accounts::FEE_PROGRAM, padded.clone())).is_ok());
        *padded.last_mut().unwrap() = 1;
        assert!(decode_pumpfun_fee_config(&account(accounts::FEE_PROGRAM, padded)).is_err());
    }

    #[test]
    fn pump_curve_and_vault_snapshot_rejects_wrong_quote_or_authority() {
        let base = Pubkey::new_unique();
        let quote = sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT;
        let curve_address =
            sol_trade_sdk::instruction::utils::pumpfun::get_bonding_curve_pda(&base).unwrap();
        let mut data = vec![0_u8; 8 + 107];
        data[..8].copy_from_slice(&[23, 183, 248, 55, 96, 216, 172, 96]);
        data[8..16].copy_from_slice(&1_000_000_u64.to_le_bytes());
        data[16..24].copy_from_slice(&10_000_u64.to_le_bytes());
        data[24..32].copy_from_slice(&800_000_u64.to_le_bytes());
        data[40..48].copy_from_slice(&1_000_000_u64.to_le_bytes());
        data[83..115].copy_from_slice(quote.as_ref());
        let decoded = decode_pumpfun_curve(
            &account(accounts::PUMPFUN, data.clone()),
            curve_address,
            base,
            quote,
        )
        .unwrap();
        assert_eq!(decoded.effective_quote_mint(), quote);
        assert!(decode_pumpfun_curve(
            &account(accounts::PUMPFUN, data.clone()),
            curve_address,
            base,
            Pubkey::new_unique(),
        )
        .is_err());
        data[48] = 1; // complete
        assert!(decode_pumpfun_curve(
            &account(accounts::PUMPFUN, data),
            curve_address,
            base,
            quote,
        )
        .is_err());

        let mut token = LegacyTokenAccount::default();
        token.mint = quote;
        token.owner = curve_address;
        token.amount = 1234;
        token.state = spl_token_interface::state::AccountState::Initialized;
        let mut bytes = vec![0_u8; LegacyTokenAccount::LEN];
        LegacyTokenAccount::pack(token, &mut bytes).unwrap();
        let vault = account(sol_trade_sdk::common::spl_token::ID, bytes);
        assert_eq!(
            decode_curve_vault(
                &vault,
                sol_trade_sdk::common::spl_token::ID,
                quote,
                curve_address,
            )
            .unwrap(),
            1234
        );
        assert!(decode_curve_vault(
            &vault,
            sol_trade_sdk::common::spl_token::ID,
            quote,
            Pubkey::new_unique(),
        )
        .is_err());
    }

    #[test]
    fn usdc_v3_quote_uses_stable_schedule_and_rejects_unverified_conditions() {
        let usdc = sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT;
        let mut snapshot = PumpFunQuoteAccountsSnapshot {
            slot: 123,
            epoch: 4,
            global: decode_pumpfun_global(&account(accounts::PUMPFUN, current_global_data()))
                .unwrap(),
            config: pumpswap::PumpSwapFeeConfig {
                flat_fees: pumpswap::PumpSwapFeeBasisPoints::new(0, 5, 5),
                fee_tiers: Vec::new(),
                stable_fee_tiers: vec![pumpswap::PumpSwapFeeTier {
                    market_cap_lamports_threshold: 0,
                    fees: pumpswap::PumpSwapFeeBasisPoints::new(0, 95, 30),
                }],
                exotic_flat_fees: pumpswap::PumpSwapFeeBasisPoints::new(0, 10, 10),
            },
            curve: BondingCurveAccount {
                virtual_token_reserves: 1_000_000,
                virtual_sol_reserves: 10_000,
                real_token_reserves: 800_000,
                real_sol_reserves: 100,
                creator: Pubkey::new_unique(),
                quote_mint: usdc,
                ..Default::default()
            },
            base_supply: 1_000_000,
            quote_supply: 1_000_000,
            base_vault_amount: 800_000,
            quote_vault_amount: 100,
            base_transfer_fee: TokenTransferFee::default(),
            quote_transfer_fee: TokenTransferFee::default(),
        };
        let quote = snapshot.quote_usdc_v3_exact_in(usdc, 100, 100).unwrap();
        assert_eq!(
            (quote.expected_base_out, quote.minimum_base_out),
            (9606, 9509)
        );
        assert_eq!(
            (quote.protocol_bps, quote.creator_bps, quote.validated_slot),
            (95, 30, 123)
        );

        snapshot
            .config
            .stable_fee_tiers
            .push(pumpswap::PumpSwapFeeTier {
                market_cap_lamports_threshold: 500,
                fees: pumpswap::PumpSwapFeeBasisPoints::new(0, 20, 5),
            });
        assert!(snapshot.quote_usdc_v3_exact_in(usdc, 100, 100).is_err());
        snapshot.config.stable_fee_tiers.pop();
        snapshot.base_transfer_fee.basis_points = 1;
        assert!(snapshot.quote_usdc_v3_exact_in(usdc, 100, 100).is_err());
        snapshot.base_transfer_fee.basis_points = 0;
        snapshot.curve.is_cashback_coin = true;
        assert!(snapshot.quote_usdc_v3_exact_in(usdc, 100, 100).is_err());
        snapshot.curve.is_cashback_coin = false;
        snapshot.curve.quote_mint = Pubkey::new_unique();
        assert!(snapshot.quote_usdc_v3_exact_in(usdc, 100, 100).is_err());
        snapshot.curve.quote_mint = usdc;
        snapshot.global.creator_fee_configurable = true;
        snapshot.curve.creator_fee_bps = 200;
        assert!(snapshot.quote_usdc_v3_exact_in(usdc, 100, 100).is_err());
        snapshot.curve.creator_fee_bps = 0;
        snapshot.quote_vault_amount = 99;
        assert!(snapshot.quote_usdc_v3_exact_in(usdc, 100, 100).is_err());
        snapshot.quote_vault_amount = 100;
        snapshot.curve.real_token_reserves = 9606;
        assert!(snapshot.quote_usdc_v3_exact_in(usdc, 100, 100).is_err());
        assert!(snapshot
            .quote_usdc_v3_exact_in(Pubkey::new_unique(), 100, 100)
            .is_err());
    }
}
