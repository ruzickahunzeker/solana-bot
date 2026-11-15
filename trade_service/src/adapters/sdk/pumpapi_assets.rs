//! Shared read-only token/account validation for PumpAPI protocol adapters.
use crate::sanitize::redact_urls;
use anyhow::{bail, Context};
use base64::Engine;
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcAccountInfoConfig};
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token_2022_interface::extension::{
    BaseStateWithExtensions, ExtensionType, StateWithExtensions,
};

pub(super) const MAX_ACCOUNT_BYTES: usize = 8192;
pub(super) struct MintEvidence {
    pub(super) decimals: u8,
    pub(super) supply: u64,
    pub(super) required: Vec<ExtensionType>,
}

pub(super) async fn read(
    rpc: &RpcClient,
    keys: &[Pubkey],
    minimum_slot: Option<u64>,
    label: &'static str,
) -> anyhow::Result<(u64, Vec<UiAccount>)> {
    let response = rpc
        .get_multiple_ui_accounts_with_config(
            keys,
            RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                commitment: Some(CommitmentConfig::processed()),
                data_slice: None,
                min_context_slot: minimum_slot,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
    if response.value.len() != keys.len()
        || minimum_slot.is_some_and(|slot| response.context.slot < slot)
    {
        bail!("{label} account response count or bank context is inconsistent");
    }
    Ok((
        response.context.slot,
        response
            .value
            .into_iter()
            .map(|account| account.with_context(|| format!("{label} required account is missing")))
            .collect::<anyhow::Result<_>>()?,
    ))
}

pub(super) fn decode_data(
    account: &UiAccount,
    expected_owner: Pubkey,
    max: usize,
    label: &'static str,
) -> anyhow::Result<Vec<u8>> {
    if account.owner != expected_owner.to_string() || account.executable || account.lamports == 0 {
        bail!("{label} account owner or state is invalid");
    }
    let UiAccountData::Binary(encoded, UiAccountEncoding::Base64) = &account.data else {
        bail!("{label} account data must use base64")
    };
    if encoded.len() > max.div_ceil(3) * 4 {
        bail!("{label} account data exceeds size limit")
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .with_context(|| format!("{label} account base64 is invalid"))?;
    if data.len() > max
        || account
            .space
            .is_some_and(|space| space != data.len() as u64)
    {
        bail!("{label} account length is inconsistent")
    }
    Ok(data)
}

pub(super) fn check_mint(
    account: &UiAccount,
    program: Pubkey,
    decimals: Option<u8>,
    epoch: u64,
    label: &'static str,
) -> anyhow::Result<MintEvidence> {
    let data = decode_data(account, program, MAX_ACCOUNT_BYTES, label)?;
    let mut required = Vec::new();
    let (actual_decimals, supply) = if program == spl_token_interface::ID {
        let mint = spl_token_interface::state::Mint::unpack(&data)
            .with_context(|| format!("{label} SPL mint layout is invalid"))?;
        (mint.decimals, mint.supply)
    } else if program == spl_token_2022_interface::ID {
        let mint = StateWithExtensions::<spl_token_2022_interface::state::Mint>::unpack(&data)
            .with_context(|| format!("{label} Token-2022 mint layout is invalid"))?;
        let types = mint
            .get_extension_types()
            .with_context(|| format!("{label} mint extensions are invalid"))?;
        spl_token_2022_interface::extension::account_len::try_for_each_required_init_account_extension(
            mint.get_tlv_data(), |extension| {required.push(extension);Ok(())},
        )?;
        let mut seen = Vec::new();
        for extension in types {
            if seen.contains(&extension) {
                bail!("{label} mint has duplicate extensions")
            }
            seen.push(extension);
            match extension {
                ExtensionType::TransferFeeConfig => {
                    let config=mint.get_extension::<spl_token_2022_interface::extension::transfer_fee::TransferFeeConfig>()?;
                    if u16::from(config.get_epoch_fee(epoch).transfer_fee_basis_points) > 10_000 {
                        bail!("{label} transfer fee basis points are invalid")
                    }
                }
                ExtensionType::TransferHook => {
                    mint.get_extension::<spl_token_2022_interface::extension::transfer_hook::TransferHook>()?;
                    if spl_token_2022_interface::extension::transfer_hook::get_program_id(&mint)
                        .is_some()
                    {
                        bail!("{label} active transfer hook is unsupported")
                    }
                }
                ExtensionType::MetadataPointer
                | ExtensionType::TokenMetadata
                | ExtensionType::MintCloseAuthority
                | ExtensionType::InterestBearingConfig => {}
                _ => bail!("{label} mint extension is not audited"),
            }
        }
        (mint.base.decimals, mint.base.supply)
    } else {
        bail!("{label} token program is unsupported")
    };
    if decimals.is_some_and(|expected| actual_decimals != expected) {
        bail!("{label} mint decimals do not match pool")
    }
    Ok(MintEvidence {
        decimals: actual_decimals,
        supply,
        required,
    })
}

pub(super) fn check_vault(
    account: &UiAccount,
    mint: Pubkey,
    program: Pubkey,
    required: &[ExtensionType],
    authority: Pubkey,
    label: &'static str,
) -> anyhow::Result<u64> {
    let data = decode_data(account, program, MAX_ACCOUNT_BYTES, label)?;
    let (actual_mint, owner, amount, initialized, delegate, close) = if program
        == spl_token_interface::ID
    {
        let token = spl_token_interface::state::Account::unpack(&data)
            .with_context(|| format!("{label} SPL vault layout is invalid"))?;
        (
            token.mint,
            token.owner,
            token.amount,
            token.state == spl_token_interface::state::AccountState::Initialized,
            token.delegate.is_some(),
            token.close_authority.is_some(),
        )
    } else if program == spl_token_2022_interface::ID {
        let token = StateWithExtensions::<spl_token_2022_interface::state::Account>::unpack(&data)
            .with_context(|| format!("{label} Token-2022 vault layout is invalid"))?;
        let types = token.get_extension_types()?;
        if required.iter().any(|extension| !types.contains(extension)) {
            bail!("{label} vault is missing required mint extensions")
        }
        let mut seen = Vec::new();
        for extension in types {
            if seen.contains(&extension) {
                bail!("{label} vault has duplicate extensions")
            }
            seen.push(extension);
            match extension {
                ExtensionType::TransferFeeAmount | ExtensionType::ImmutableOwner => {}
                ExtensionType::TransferHookAccount => {
                    let hook=token.get_extension::<spl_token_2022_interface::extension::transfer_hook::TransferHookAccount>()?;
                    if bool::from(hook.transferring) {
                        bail!("{label} vault hook state is invalid")
                    }
                }
                _ => bail!("{label} vault extension is not audited"),
            }
        }
        let token = token.base;
        (
            token.mint,
            token.owner,
            token.amount,
            token.state == spl_token_2022_interface::state::AccountState::Initialized,
            token.delegate.is_some(),
            token.close_authority.is_some(),
        )
    } else {
        bail!("{label} vault token program is unsupported")
    };
    if actual_mint != mint || owner != authority || !initialized || delegate || close {
        bail!("{label} vault mint, authority or state is invalid")
    }
    Ok(amount)
}
