use anyhow::{bail, Context, Result};
use sol_trade_sdk::instruction::utils::meteora_damm_v2::{accounts, SWAP_MODE_EXACT_IN};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status_client_types::{
    option_serializer::OptionSerializer, UiTransactionTokenBalance,
};
use std::collections::HashSet;

use super::{
    evidence::{check_snapshot, AccountSnapshot, AssetSnapshot, DammProbeEvidence},
    simulate_packet, DammOutputEvidence,
};
mod trace;

/// Output of this unsigned simulation, never a delta against an earlier RPC bank.
pub(crate) async fn evidence_damm_probe_output(
    evidence: &DammProbeEvidence<'_>,
) -> Result<DammOutputEvidence> {
    let prepared = evidence.prepared;
    let intent = prepared.accounts();
    if intent.pool.swap_mode != SWAP_MODE_EXACT_IN {
        bail!("DAMM v2 quote output evidence requires exact-in mode");
    }
    let pool = &intent.pool;
    let keys = [
        pool.pool,
        pool.token_a_mint,
        pool.token_b_mint,
        pool.token_a_vault,
        pool.token_b_vault,
        solana_sdk::sysvar::clock::id(),
        intent.input_ata,
        prepared.output_ata(),
    ];
    let response = simulate_packet(
        evidence.rpc,
        prepared.transaction(),
        &keys,
        evidence.slot,
        true,
    )
    .await?;
    if response.context.slot < evidence.slot {
        bail!("SDK DAMM v2 quote simulation context precedes account evidence");
    }
    if let Some(error) = response.value.err {
        bail!("DAMM v2 quote simulation failed: {error:?}");
    }
    if response.value.replacement_blockhash.is_some()
        || response
            .value
            .loaded_addresses
            .as_ref()
            .is_some_and(|loaded| !loaded.writable.is_empty() || !loaded.readonly.is_empty())
    {
        bail!("DAMM v2 quote simulation changed the fixed legacy message context");
    }
    let values = response
        .value
        .accounts
        .as_ref()
        .context("DAMM v2 simulation did not return account evidence")?;
    let snapshot = check_snapshot(prepared, response.context.slot, values, true)?;
    let movements = trace::transfers(
        prepared,
        response
            .value
            .inner_instructions
            .as_deref()
            .context("DAMM v2 simulation omitted token transfer evidence")?,
    )?;
    if movements.input.amount != intent.amount {
        bail!("DAMM v2 simulation input transfer differs from requested amount");
    }
    let credited_input = net_amount(&movements.input, &snapshot.input)?;
    let output = net_amount(&movements.output, &snapshot.output)?;
    if output == 0 || credited_input == 0 {
        bail!("DAMM v2 simulation returned no positive output");
    }
    if snapshot.output_amount < output
        || snapshot.input_vault_amount < credited_input
        || snapshot
            .input_amount
            .checked_add(movements.input.amount)
            .is_none()
        || snapshot
            .output_vault_amount
            .checked_add(movements.output.amount)
            .is_none()
    {
        bail!("DAMM v2 token transfer evidence contradicts simulated account balances");
    }
    match (
        &response.value.pre_token_balances,
        &response.value.post_token_balances,
    ) {
        (None, None) => {}
        (Some(pre), Some(post)) => cross_check(
            pre,
            post,
            prepared,
            &snapshot,
            &movements,
            credited_input,
            output,
        )?,
        _ => bail!("DAMM v2 simulation returned incomplete same-bank token balances"),
    }
    DammOutputEvidence::checked(
        prepared,
        &snapshot,
        output,
        evidence.slot,
        response.context.slot,
        evidence.started,
    )
}

#[cfg(test)]
pub(crate) async fn quote_evidenced_damm_probe(evidence: &DammProbeEvidence<'_>) -> Result<u64> {
    Ok(evidence_damm_probe_output(evidence).await?.output_units())
}

fn net_amount(transfer: &trace::Transfer, asset: &AssetSnapshot) -> Result<u64> {
    if transfer.program != asset.program
        || transfer.mint.is_some_and(|mint| mint != asset.mint)
        || transfer
            .decimals
            .is_some_and(|decimals| decimals != asset.decimals)
    {
        bail!("DAMM v2 transfer mint, program or decimals differs from simulated mint");
    }
    if asset.fee.is_some() && transfer.decimals.is_none() {
        bail!("DAMM v2 unchecked transfer cannot prove Token-2022 transfer fees");
    }
    let fee = asset
        .fee
        .as_ref()
        .map(|fee| {
            fee.calculate_fee(transfer.amount)
                .context("DAMM v2 transfer fee calculation overflow")
        })
        .transpose()?
        .unwrap_or(0);
    if transfer.fee.is_some_and(|declared| declared != fee) {
        bail!("DAMM v2 declared transfer fee differs from simulated epoch fee");
    }
    transfer
        .amount
        .checked_sub(fee)
        .context("DAMM v2 fee exceeds transfer amount")
}

fn checked_rows(rows: &[UiTransactionTokenBalance], keys: &[Pubkey]) -> Result<()> {
    let mut seen = HashSet::new();
    if rows.len() > keys.len() {
        bail!("DAMM v2 simulation has too many token balance rows");
    }
    for row in rows {
        if usize::from(row.account_index) >= keys.len() || !seen.insert(row.account_index) {
            bail!("DAMM v2 simulation has duplicate or invalid token balance indexes");
        }
    }
    Ok(())
}

fn amount_row(
    rows: &[UiTransactionTokenBalance],
    index: u8,
    asset: &AssetSnapshot,
    owner: Pubkey,
) -> Result<Option<u64>> {
    let Some(row) = rows.iter().find(|row| row.account_index == index) else {
        return Ok(None);
    };
    if row.mint != asset.mint.to_string()
        || row.ui_token_amount.decimals != asset.decimals
        || !matches!(&row.owner, OptionSerializer::Some(actual) if *actual == owner.to_string())
        || !matches!(&row.program_id, OptionSerializer::Some(actual) if *actual == asset.program.to_string())
    {
        bail!("DAMM v2 same-bank token balance identity is inconsistent");
    }
    Ok(Some(trace::integer(&row.ui_token_amount.amount)?))
}

fn cross_check(
    pre: &[UiTransactionTokenBalance],
    post: &[UiTransactionTokenBalance],
    prepared: &super::plan::PreparedDammProbe,
    snapshot: &AccountSnapshot,
    movements: &trace::Transfers,
    credited_input: u64,
    output: u64,
) -> Result<()> {
    let keys = &prepared.transaction().message.account_keys;
    checked_rows(pre, keys)?;
    checked_rows(post, keys)?;
    let intent = prepared.accounts();
    let a_in = intent.input_mint == intent.pool.token_a_mint;
    let (input_vault, output_vault) = if a_in {
        (intent.pool.token_a_vault, intent.pool.token_b_vault)
    } else {
        (intent.pool.token_b_vault, intent.pool.token_a_vault)
    };
    let roles = [
        (
            intent.input_ata,
            &snapshot.input,
            intent.wallet,
            snapshot.input_amount,
            false,
        ),
        (
            intent.output_ata,
            &snapshot.output,
            intent.wallet,
            snapshot.output_amount,
            movements.output_initialized,
        ),
        (
            input_vault,
            &snapshot.input,
            accounts::AUTHORITY,
            snapshot.input_vault_amount,
            false,
        ),
        (
            output_vault,
            &snapshot.output,
            accounts::AUTHORITY,
            snapshot.output_vault_amount,
            false,
        ),
    ];
    let mut before = Vec::with_capacity(4);
    for (key, asset, owner, amount, may_be_new) in roles {
        let index = u8::try_from(
            keys.iter()
                .position(|actual| *actual == key)
                .context("DAMM v2 token balance account is not in message")?,
        )?;
        let previous = match amount_row(pre, index, asset, owner)? {
            Some(_) if may_be_new => {
                bail!("DAMM v2 output initialization contradicts an existing pre-token balance")
            }
            Some(value) => value,
            None if may_be_new => 0,
            None => bail!("DAMM v2 simulation omitted a required pre-token balance"),
        };
        if amount_row(post, index, asset, owner)? != Some(amount) {
            bail!("DAMM v2 simulation post-token balance differs from account data");
        }
        before.push(previous);
    }
    if before[0].checked_sub(movements.input.amount) != Some(snapshot.input_amount)
        || before[1].checked_add(output) != Some(snapshot.output_amount)
        || before[2].checked_add(credited_input) != Some(snapshot.input_vault_amount)
        || before[3].checked_sub(movements.output.amount) != Some(snapshot.output_vault_amount)
    {
        bail!("DAMM v2 same-bank balances disagree with this swap's token transfers");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
