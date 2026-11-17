use crate::adapters::rpc::{
    aggregator_simulation::{AggregatorSimulationReadSession, UnsignedAggregatorSimulation},
    read_http::ReadHttpTransport,
    read_runtime::SharedReadRuntime,
};
use anyhow::{bail, Context, Result};
use solana_sdk::{pubkey::Pubkey, transaction::VersionedTransaction};

pub(crate) mod audit;
#[cfg(test)]
mod tests;
use audit::*;

pub struct AggregatorSimulation {
    pub ok: bool,
    pub fee_lamports: Option<u64>,
    pub units_consumed: Option<u64>,
    pub payer_debit_lamports: Option<u64>,
    pub inner_instruction_groups: Option<usize>,
    pub target_token_delta_units: Option<u64>,
    pub target_token_post_units: Option<u64>,
    pub baseline_slot: u64,
    pub simulation_slot: u64,
    pub observed_same_slot_payer_debit_lamports: Option<u64>,
    pub observed_same_slot_target_token_delta_units: Option<u64>,
    pub inner_instruction_count: Option<usize>,
    pub inner_programs: Option<Vec<String>>,
    pub inner_payer_system_spend_lamports: Option<u64>,
}

pub(crate) async fn simulate<T: ReadHttpTransport>(
    runtime: &SharedReadRuntime<T>,
    url: &str,
    wallet: Pubkey,
    transaction: &VersionedTransaction,
    target_account_index: u8,
    target_mint: Pubkey,
    target_ata: Pubkey,
    target_token_program: Pubkey,
    resolved_keys: &[Pubkey],
) -> Result<AggregatorSimulation> {
    let plan = UnsignedAggregatorSimulation::new(
        transaction,
        wallet,
        target_ata,
        target_account_index,
        resolved_keys,
    )?;
    let mut session = AggregatorSimulationReadSession::new(runtime, url, plan)?;
    let baseline = session.baseline().await?;
    let (target_mint, target_ata, target_token_program) =
        (&target_mint, &target_ata, &target_token_program);
    let baseline_payer = baseline
        .value
        .first()
        .and_then(Option::as_ref)
        .context("aggregator payer account is missing from baseline")?;
    let baseline_token_amount = baseline
        .value
        .get(1)
        .and_then(Option::as_ref)
        .map(|account| {
            target_account_amount_from_data(
                &account.owner,
                &account.data,
                target_token_program,
                target_mint,
                &wallet,
            )
        })
        .transpose()?
        .unwrap_or(0);
    let result = session.simulate().await?;
    let simulation_slot = result.context.slot;
    let value = result.value;
    let inner_inventory = value
        .inner_instructions
        .as_ref()
        .map(|groups| {
            audit_inner_instruction_inventory(
                groups,
                resolved_keys,
                transaction.message.instructions().len(),
            )
        })
        .transpose()?;
    if let Some(groups) = value.inner_instructions.as_ref() {
        check_inner_target_token_instructions(groups, resolved_keys, target_ata)?;
    }
    let inner_payer_system_spend_lamports = value
        .inner_instructions
        .as_ref()
        .map(|groups| inner_payer_system_spend(groups, resolved_keys, &wallet))
        .transpose()?
        .flatten();
    let payer_debit_lamports = value
        .pre_balances
        .as_ref()
        .and_then(|balances| balances.first())
        .zip(
            value
                .post_balances
                .as_ref()
                .and_then(|balances| balances.first()),
        )
        .and_then(|(before, after)| before.checked_sub(*after));
    let target_token_delta_units =
        value
            .pre_token_balances
            .as_ref()
            .and_then(|balances| target_token_amount(balances, target_account_index, target_mint))
            .zip(value.post_token_balances.as_ref().and_then(|balances| {
                target_token_amount(balances, target_account_index, target_mint)
            }))
            .and_then(|(before, after)| after.checked_sub(before));
    let payer_post_lamports = value
        .accounts
        .as_ref()
        .and_then(|accounts| accounts.first())
        .and_then(Option::as_ref)
        .map(|account| account.lamports);
    let target_token_post_units = value
        .accounts
        .as_ref()
        .and_then(|accounts| accounts.get(1))
        .and_then(Option::as_ref)
        .map(|account| {
            simulated_target_account_amount(account, target_token_program, target_mint, &wallet)
        })
        .transpose()?;
    let (same_slot_payer_debit, same_slot_token_delta) = same_slot_deltas(
        baseline.context.slot,
        simulation_slot,
        baseline_payer.lamports,
        payer_post_lamports,
        baseline_token_amount,
        target_token_post_units,
    );
    if let (Some(post), Some(delta)) = (target_token_post_units, target_token_delta_units) {
        if post < delta {
            bail!("simulated target ATA balance is inconsistent with token delta");
        }
    }
    Ok(AggregatorSimulation {
        ok: value.err.is_none(),
        fee_lamports: value.fee,
        units_consumed: value.units_consumed,
        payer_debit_lamports,
        inner_instruction_groups: value.inner_instructions.as_ref().map(Vec::len),
        target_token_delta_units,
        target_token_post_units,
        baseline_slot: baseline.context.slot,
        simulation_slot,
        observed_same_slot_payer_debit_lamports: same_slot_payer_debit,
        observed_same_slot_target_token_delta_units: same_slot_token_delta,
        inner_instruction_count: inner_inventory.as_ref().map(|inventory| inventory.0),
        inner_programs: inner_inventory.map(|inventory| inventory.1),
        inner_payer_system_spend_lamports,
    })
}
