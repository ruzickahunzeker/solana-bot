use super::pumpapi_assets::{check_mint, check_vault, decode_data, MAX_ACCOUNT_BYTES};
use crate::domain::trade::{Address, USD1_MINT, USDC_MINT, WSOL_MINT};
use anyhow::{ensure, Context, Result};
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token_2022_interface::extension::{
    interest_bearing_mint::InterestBearingConfig,
    metadata_pointer::MetadataPointer,
    mint_close_authority::MintCloseAuthority,
    transfer_fee::{TransferFeeAmount, TransferFeeConfig},
    transfer_hook::TransferHookAccount,
    BaseStateWithExtensions, ExtensionType, StateWithExtensions,
};

const LABEL: &str = "preparation input";

pub(crate) struct RawInputBalance {
    pub(crate) units: u64,
    pub(crate) account: Address,
    pub(crate) program: Address,
    pub(crate) decimals: u8,
}

pub(crate) fn input_keys(wallet: Address, mint: Address) -> [Pubkey; 3] {
    let wallet = Pubkey::new_from_array(wallet.to_bytes());
    let mint = Pubkey::new_from_array(mint.to_bytes());
    let ata = |program| {
        spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
            &wallet, &mint, &program,
        )
    };
    [
        mint,
        ata(spl_token_interface::ID),
        ata(spl_token_2022_interface::ID),
    ]
}

/// Positional RPC evidence for the exact keys above, not a final-message authorization.
pub(crate) fn inspect(
    wallet: Address,
    mint: Address,
    values: &[Option<UiAccount>],
) -> Result<RawInputBalance> {
    ensure!(
        values.len() == 3,
        "preparation raw account count is invalid"
    );
    let keys = input_keys(wallet, mint);
    ensure!(
        keys[0] != keys[1] && keys[0] != keys[2] && keys[1] != keys[2],
        "preparation account roles alias each other"
    );
    let mint_account = values[0].as_ref().context("preparation mint is missing")?;
    let program = mint_account.owner.parse::<Pubkey>()?;
    let input_index = if program == spl_token_interface::ID {
        1
    } else {
        ensure!(
            program == spl_token_2022_interface::ID,
            "unsupported preparation token program"
        );
        2
    };
    ensure!(
        values[3 - input_index].as_ref().is_none_or(|account| {
            account.owner == Pubkey::default().to_string()
                && !account.executable
                && account.lamports > 0
                && account.space.is_none_or(|space| space == 0)
                && matches!(&account.data, UiAccountData::Binary(value, UiAccountEncoding::Base64) if value.is_empty())
        }),
        "preparation has an account for the wrong token program"
    );
    // Both schedules must be structurally valid; no epoch fee/output quote is inferred here.
    let evidence = check_mint(mint_account, program, None, u64::MAX, LABEL)?;
    let canonical_decimals = match keys[0].to_string().as_str() {
        WSOL_MINT => Some(9),
        USDC_MINT | USD1_MINT => Some(6),
        _ => None,
    };
    ensure!(
        canonical_decimals.is_none_or(
            |decimals| program == spl_token_interface::ID && decimals == evidence.decimals
        ),
        "preparation canonical quote mint metadata is invalid"
    );
    if program == spl_token_2022_interface::ID {
        let data = decode_data(mint_account, program, MAX_ACCOUNT_BYTES, LABEL)?;
        let state = StateWithExtensions::<spl_token_2022_interface::state::Mint>::unpack(&data)?;
        for extension in state.get_extension_types()? {
            match extension {
                ExtensionType::TransferFeeConfig => {
                    let fee = state.get_extension::<TransferFeeConfig>()?;
                    ensure!(
                        u16::from(fee.older_transfer_fee.transfer_fee_basis_points) <= 10_000
                            && u16::from(fee.newer_transfer_fee.transfer_fee_basis_points)
                                <= 10_000
                            && u64::from(fee.older_transfer_fee.epoch)
                                <= u64::from(fee.newer_transfer_fee.epoch),
                        "preparation transfer fee schedules are invalid"
                    );
                }
                ExtensionType::MetadataPointer => {
                    state.get_extension::<MetadataPointer>()?;
                }
                ExtensionType::MintCloseAuthority => {
                    state.get_extension::<MintCloseAuthority>()?;
                }
                ExtensionType::InterestBearingConfig => {
                    state.get_extension::<InterestBearingConfig>()?;
                }
                // Metadata is non-execution data; the shared decoder checks its TLV framing.
                ExtensionType::TokenMetadata | ExtensionType::TransferHook => {}
                _ => anyhow::bail!("preparation mint extension is not audited"),
            }
        }
    }
    let units = if let Some(account) = &values[input_index] {
        let units = check_vault(
            account,
            keys[0],
            program,
            &evidence.required,
            Pubkey::new_from_array(wallet.to_bytes()),
            LABEL,
        )?;
        let data = decode_data(account, program, MAX_ACCOUNT_BYTES, LABEL)?;
        let (delegated, native) = if program == spl_token_2022_interface::ID {
            let state =
                StateWithExtensions::<spl_token_2022_interface::state::Account>::unpack(&data)?;
            for extension in state.get_extension_types()? {
                match extension {
                    ExtensionType::TransferFeeAmount => {
                        state.get_extension::<TransferFeeAmount>()?;
                    }
                    ExtensionType::TransferHookAccount => {
                        state.get_extension::<TransferHookAccount>()?;
                    }
                    ExtensionType::ImmutableOwner => {
                        state.get_extension::<spl_token_2022_interface::extension::immutable_owner::ImmutableOwner>()?;
                    }
                    _ => anyhow::bail!("preparation account extension is not audited"),
                }
            }
            (
                state.base.delegated_amount,
                Option::<u64>::from(state.base.is_native),
            )
        } else {
            let state = spl_token_interface::state::Account::unpack(&data)?;
            (state.delegated_amount, Option::<u64>::from(state.is_native))
        };
        ensure!(
            delegated == 0,
            "preparation account has residual delegated amount"
        );
        let is_wsol = keys[0].to_string() == WSOL_MINT;
        ensure!(
            native.is_some() == is_wsol,
            "preparation native token state is inconsistent"
        );
        if let Some(reserve) = native {
            ensure!(
                program == spl_token_interface::ID
                    && reserve
                        .checked_add(units)
                        .is_some_and(|required| account.lamports >= required),
                "preparation native balance does not cover reserve and token units"
            );
        } else {
            ensure!(
                units <= evidence.supply,
                "preparation input exceeds mint supply"
            );
        }
        units
    } else {
        0
    };
    Ok(RawInputBalance {
        units,
        account: Address::from_bytes(keys[input_index].to_bytes()),
        program: Address::from_bytes(program.to_bytes()),
        decimals: evidence.decimals,
    })
}

#[cfg(test)]
pub(crate) mod tests;
