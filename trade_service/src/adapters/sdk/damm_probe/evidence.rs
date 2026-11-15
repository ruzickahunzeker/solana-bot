//! Partial identity/token evidence, not an executable-route or signing authorization.
use anyhow::{bail, Context, Result};
use bincode::Options;
use sol_trade_sdk::{
    constants::{USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    instruction::utils::{meteora_damm_v2::accounts, meteora_damm_v2_types::pool_account_decode},
};
use solana_account_decoder::UiAccountEncoding;
#[cfg(test)]
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcAccountInfoConfig};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::clock::Clock;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use super::plan::{PreparedDammProbe, ProbeAccounts};
#[cfg(test)]
use super::simulate_prepared_damm_probe;
// Reuse the existing generic validators without changing the paused backend's code.
use crate::adapters::sdk::pumpapi_assets::{
    check_mint, check_vault, decode_data, MAX_ACCOUNT_BYTES,
};

const LABEL: &str = "SDK DAMM v2 quote";

pub(crate) struct DammProbeEvidence<'a> {
    pub(super) rpc: &'a RpcClient,
    pub(super) prepared: &'a PreparedDammProbe,
    pub(super) slot: u64,
    pub(super) started: Instant,
    #[cfg(test)]
    output_before: u64,
}

impl DammProbeEvidence<'_> {
    #[cfg(test)]
    pub(crate) fn output_before(&self) -> u64 {
        self.output_before
    }
}

pub(crate) async fn inspect_prepared_damm_probe_at_slot<'a>(
    rpc: &'a RpcClient,
    prepared: &'a PreparedDammProbe,
    min_context_slot: u64,
) -> Result<DammProbeEvidence<'a>> {
    if min_context_slot == 0 {
        return Err(crate::domain::preparation_balance::reject_balance(
            "DAMM quote requires a captured nonzero balance slot",
        ));
    }
    inspect_with_deadline(rpc, prepared, Some(min_context_slot)).await
}

#[cfg(test)]
pub(crate) async fn inspect_prepared_damm_probe<'a>(
    rpc: &'a RpcClient,
    prepared: &'a PreparedDammProbe,
) -> Result<DammProbeEvidence<'a>> {
    inspect_with_deadline(rpc, prepared, None).await
}

async fn inspect_with_deadline<'a>(
    rpc: &'a RpcClient,
    prepared: &'a PreparedDammProbe,
    min_context_slot: Option<u64>,
) -> Result<DammProbeEvidence<'a>> {
    tokio::time::timeout(
        Duration::from_secs(5),
        inspect(rpc, prepared, Instant::now(), min_context_slot),
    )
    .await
    .context("SDK DAMM v2 quote account evidence timed out")?
}

async fn inspect<'a>(
    rpc: &'a RpcClient,
    prepared: &'a PreparedDammProbe,
    started: Instant,
    min_context_slot: Option<u64>,
) -> Result<DammProbeEvidence<'a>> {
    let intent = prepared.accounts();
    let expected = &intent.pool;
    if expected.referral_token_account.is_some() {
        bail!("SDK DAMM v2 quote referral account evidence is not audited");
    }
    let keys = [
        expected.pool,
        expected.token_a_mint,
        expected.token_b_mint,
        expected.token_a_vault,
        expected.token_b_vault,
        solana_sdk::sysvar::clock::id(),
        intent.input_ata,
        intent.output_ata,
    ];
    if keys.iter().collect::<HashSet<_>>().len() != keys.len() {
        bail!("SDK DAMM v2 quote account roles alias each other");
    }
    let response = rpc
        .get_multiple_ui_accounts_with_config(
            &keys,
            RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                commitment: Some(CommitmentConfig::processed()),
                data_slice: None,
                min_context_slot,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", crate::sanitize::redact_urls(&error.to_string())))?;
    if response.value.len() != keys.len() || response.context.slot == 0 {
        bail!("SDK DAMM v2 quote account count or context is invalid");
    }
    let slot = response.context.slot;
    if min_context_slot.is_some_and(|minimum| slot < minimum) {
        return Err(crate::domain::preparation_balance::reject_balance(
            "DAMM quote account context precedes the captured balance slot",
        ));
    }
    let _snapshot = check_snapshot(prepared, slot, &response.value, false)?;
    Ok(DammProbeEvidence {
        rpc,
        prepared,
        slot,
        started,
        #[cfg(test)]
        output_before: _snapshot.output_amount,
    })
}

pub(super) struct AssetSnapshot {
    pub mint: solana_sdk::pubkey::Pubkey,
    pub program: solana_sdk::pubkey::Pubkey,
    pub decimals: u8,
    pub fee: Option<spl_token_2022_interface::extension::transfer_fee::TransferFee>,
}
pub(super) struct AccountSnapshot {
    pub input: AssetSnapshot,
    pub output: AssetSnapshot,
    pub input_amount: u64,
    pub output_amount: u64,
    pub input_vault_amount: u64,
    pub output_vault_amount: u64,
}

pub(super) fn check_snapshot(
    prepared: &PreparedDammProbe,
    slot: u64,
    values: &[Option<solana_account_decoder::UiAccount>],
    post_simulation: bool,
) -> Result<AccountSnapshot> {
    check_snapshot_for_accounts(
        prepared.accounts(),
        slot,
        values,
        post_simulation,
        InputRequirement::Existing,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InputRequirement {
    Existing,
    #[cfg(test)]
    ExistingBuy,
    #[cfg(test)]
    NativeDeposit,
}

/// Pure account decoding/validation; no RPC, signing or executable route capability.
pub(super) fn check_snapshot_for_accounts(
    intent: &ProbeAccounts,
    slot: u64,
    values: &[Option<solana_account_decoder::UiAccount>],
    post_simulation: bool,
    input_requirement: InputRequirement,
) -> Result<AccountSnapshot> {
    if values.len() != 8 || slot == 0 {
        bail!("SDK DAMM v2 quote account count or context is invalid");
    }
    let requires_balance = match input_requirement {
        InputRequirement::Existing => true,
        #[cfg(test)]
        InputRequirement::ExistingBuy => true,
        #[cfg(test)]
        InputRequirement::NativeDeposit => false,
    };
    let requires_input = requires_balance || post_simulation;
    if requires_input && values[..7].iter().any(Option::is_none) {
        bail!("SDK DAMM v2 quote required account is missing");
    }
    let expected = &intent.pool;
    let required = values[..6]
        .iter()
        .map(|account| {
            account
                .as_ref()
                .context("SDK DAMM v2 quote required account is missing")
        })
        .collect::<Result<Vec<_>>>()?;
    let pool_data = decode_data(
        required[0],
        accounts::METEORA_DAMM_V2,
        MAX_ACCOUNT_BYTES,
        LABEL,
    )?;
    let pool =
        pool_account_decode(&pool_data).context("SDK DAMM v2 quote pool layout is invalid")?;
    if pool.token_a_mint != expected.token_a_mint
        || pool.token_b_mint != expected.token_b_mint
        || pool.token_a_vault != expected.token_a_vault
        || pool.token_b_vault != expected.token_b_vault
    {
        bail!("SDK DAMM v2 quote pool identity changed since parameter loading");
    }
    let clock_data = decode_data(required[5], solana_sdk::sysvar::id(), 40, LABEL)?;
    let clock: Clock = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(&clock_data)
        .context("SDK DAMM v2 quote Clock layout is invalid")?;
    if clock.slot != slot {
        bail!("SDK DAMM v2 quote bank and Clock context differ");
    }
    let mut mint_evidence = Vec::with_capacity(2);
    let mut assets = Vec::with_capacity(2);
    for (account, mint, program) in [
        (required[1], pool.token_a_mint, expected.token_a_program),
        (required[2], pool.token_b_mint, expected.token_b_program),
    ] {
        let decimals = if mint == WSOL_TOKEN_ACCOUNT {
            Some(9)
        } else if mint == USDC_TOKEN_ACCOUNT {
            Some(6)
        } else {
            None
        };
        if decimals.is_some() && program != spl_token_interface::ID {
            bail!("SDK DAMM v2 quote canonical Quote token program is invalid");
        }
        let evidence = check_mint(account, program, decimals, clock.epoch, LABEL)?;
        let fee = if program == spl_token_2022_interface::ID {
            use spl_token_2022_interface::extension::{
                transfer_fee::TransferFeeConfig, BaseStateWithExtensions, ExtensionType,
                StateWithExtensions,
            };
            let data = decode_data(account, program, MAX_ACCOUNT_BYTES, LABEL)?;
            let mint = StateWithExtensions::<spl_token_2022_interface::state::Mint>::unpack(&data)?;
            if mint
                .get_extension_types()?
                .contains(&ExtensionType::TransferFeeConfig)
            {
                Some(
                    *mint
                        .get_extension::<TransferFeeConfig>()?
                        .get_epoch_fee(clock.epoch),
                )
            } else {
                None
            }
        } else {
            None
        };
        assets.push(AssetSnapshot {
            mint,
            program,
            decimals: evidence.decimals,
            fee,
        });
        mint_evidence.push(evidence);
    }
    let vault_a = check_vault(
        required[3],
        pool.token_a_mint,
        expected.token_a_program,
        &mint_evidence[0].required,
        accounts::AUTHORITY,
        LABEL,
    )?;
    let vault_b = check_vault(
        required[4],
        pool.token_b_mint,
        expected.token_b_program,
        &mint_evidence[1].required,
        accounts::AUTHORITY,
        LABEL,
    )?;
    let a_in = intent.input_mint == pool.token_a_mint;
    let (input_program, output_program, input_extensions, output_extensions) = if a_in {
        (
            expected.token_a_program,
            expected.token_b_program,
            &mint_evidence[0].required,
            &mint_evidence[1].required,
        )
    } else {
        (
            expected.token_b_program,
            expected.token_a_program,
            &mint_evidence[1].required,
            &mint_evidence[0].required,
        )
    };
    #[cfg(test)]
    if input_requirement == InputRequirement::NativeDeposit
        && (intent.input_mint != WSOL_TOKEN_ACCOUNT || input_program != spl_token_interface::ID)
    {
        bail!("DAMM native deposit requires canonical WSOL input");
    }
    let balance = values[6]
        .as_ref()
        .map(|account| {
            #[cfg(test)]
            if !post_simulation
                && input_requirement != InputRequirement::Existing
                && prefunded_ata(account)?
            {
                return Ok(0);
            }
            check_vault(
                account,
                intent.input_mint,
                input_program,
                input_extensions,
                intent.wallet,
                "SDK DAMM v2 quote input ATA",
            )
        })
        .transpose()?
        .unwrap_or(0);
    if !post_simulation && requires_balance && balance < intent.amount {
        bail!("SDK DAMM v2 quote input ATA balance is insufficient");
    }
    if post_simulation && values[7].is_none() {
        bail!("SDK DAMM v2 simulated output ATA is missing");
    }
    let output_before = values[7]
        .as_ref()
        .map(|account| {
            #[cfg(test)]
            if !post_simulation
                && input_requirement != InputRequirement::Existing
                && prefunded_ata(account)?
            {
                return Ok(0);
            }
            check_vault(
                account,
                intent.output_mint,
                output_program,
                output_extensions,
                intent.wallet,
                "SDK DAMM v2 quote output ATA",
            )
        })
        .transpose()?
        .unwrap_or(0);
    let b = assets.pop().expect("two validated mints");
    let a = assets.pop().expect("two validated mints");
    let (input, output, input_vault_amount, output_vault_amount) = if a_in {
        (a, b, vault_a, vault_b)
    } else {
        (b, a, vault_b, vault_a)
    };
    Ok(AccountSnapshot {
        input,
        output,
        input_amount: balance,
        output_amount: output_before,
        input_vault_amount,
        output_vault_amount,
    })
}

#[cfg(test)]
fn prefunded_ata(account: &solana_account_decoder::UiAccount) -> Result<bool> {
    let system = solana_system_interface::program::id();
    if account.owner != system.to_string() {
        return Ok(false);
    }
    decode_data(account, system, 0, "DAMM buy prefunded ATA")?;
    Ok(true)
}

#[cfg(test)]
pub(crate) async fn simulate_evidenced_damm_probe(
    evidence: &DammProbeEvidence<'_>,
) -> Result<Response<RpcSimulateTransactionResult>> {
    let response =
        simulate_prepared_damm_probe(evidence.rpc, evidence.prepared, evidence.slot).await?;
    if response.context.slot < evidence.slot {
        bail!("SDK DAMM v2 quote simulation context precedes account evidence");
    }
    Ok(response)
}

#[cfg(test)]
pub(crate) mod tests;
