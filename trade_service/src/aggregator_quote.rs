use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use solana_sdk::message::VersionedMessage;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

use crate::domain::quote::{BoundQuote, QuoteEstimate, QuotePreflight};
use crate::models::WSOL_MINT;
use crate::sdk_client::{check_target_token_instruction, SdkTradeClient};

const JUPITER_QUOTE_URL: &str = "https://lite-api.jup.ag/swap/v1/quote";
const JUPITER_SWAP_URL: &str = "https://lite-api.jup.ag/swap/v1/swap";
const MAX_COMPUTE_UNIT_LIMIT: u64 = 1_400_000;
const MAX_PRIORITY_FEE_LAMPORTS: u64 = 100_000;
const MAX_PRIORITY_FEE_METADATA_ROUNDING_LAMPORTS: u64 = 1;
const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
const MAX_EXTRA_SYSTEM_SPEND_LAMPORTS: u64 = 20_000_000;
const MAX_WRITABLE_ACCOUNTS: usize = 64;
const MAX_BASE_FEE_LAMPORTS: u64 = 50_000;

#[cfg(test)]
pub use crate::api::dto::sol_buy::{
    BoundSolBuyQuote, SolBuyPreflightResponse, SolBuySubmitRequest, SolBuySubmitResponse,
};
pub use crate::api::dto::sol_buy::{QuoteLeg, SolBuyQuoteRequest, SolBuyQuoteResponse};

fn require_writable_target_ata(
    message: &VersionedMessage,
    resolved_keys: &[Pubkey],
    target_ata: &Pubkey,
) -> Result<u8> {
    let target_index = resolved_keys
        .iter()
        .position(|key| key == target_ata)
        .context("aggregator transaction does not reference wallet target ATA")?;
    if !message.is_maybe_writable_with_reserved_addresses(target_index, None::<&HashSet<Pubkey>>) {
        bail!("aggregator target ATA is not writable");
    }
    u8::try_from(target_index).context("aggregator target ATA index is out of range")
}

fn read_explicit_system_spend(
    instructions: &[solana_sdk::message::compiled_instruction::CompiledInstruction],
    keys: &[Pubkey],
    payer: &Pubkey,
    input_lamports: u64,
) -> Result<u64> {
    let system_program = Pubkey::default();
    let mut total = 0u64;
    for instruction in instructions {
        let program = keys
            .get(usize::from(instruction.program_id_index))
            .context("invalid system instruction program index")?;
        if *program != system_program {
            continue;
        }
        if instruction.accounts.len() != 2
            || instruction.data.len() != 12
            || instruction.data[..4] != [2, 0, 0, 0]
        {
            bail!("unsupported system instruction in aggregator transaction");
        }
        let sender = keys
            .get(usize::from(instruction.accounts[0]))
            .context("invalid system transfer sender index")?;
        if sender != payer {
            bail!("aggregator system transfer sender does not match payer");
        }
        let lamports = u64::from_le_bytes(instruction.data[4..12].try_into()?);
        total = total
            .checked_add(lamports)
            .context("system transfer total overflow")?;
    }
    if total > input_lamports.saturating_add(MAX_EXTRA_SYSTEM_SPEND_LAMPORTS) {
        bail!("aggregator explicit system transfers exceed preflight cap");
    }
    Ok(total)
}

fn read_compute_budget_fee(
    instructions: &[solana_sdk::message::compiled_instruction::CompiledInstruction],
    keys: &[Pubkey],
) -> Result<(u64, u64)> {
    let program = Pubkey::from_str(COMPUTE_BUDGET_PROGRAM)?;
    let mut limit = None;
    let mut price = None;
    for instruction in instructions {
        if keys[usize::from(instruction.program_id_index)] != program {
            continue;
        }
        if !instruction.accounts.is_empty() {
            bail!("compute budget instruction has unexpected accounts");
        }
        match instruction.data.as_slice() {
            [2, raw @ ..] if raw.len() == 4 && limit.is_none() => {
                limit = Some(u32::from_le_bytes(raw.try_into()?) as u64);
            }
            [3, raw @ ..] if raw.len() == 8 && price.is_none() => {
                price = Some(u64::from_le_bytes(raw.try_into()?));
            }
            _ => bail!("unsupported or duplicate compute budget instruction"),
        }
    }
    let limit = limit.context("aggregator transaction lacks explicit compute unit limit")?;
    if limit == 0 || limit > MAX_COMPUTE_UNIT_LIMIT {
        bail!("aggregator compute unit limit exceeds preflight cap");
    }
    let price = price.unwrap_or(0);
    let priority_fee = limit
        .checked_mul(price)
        .context("priority fee overflow")?
        .div_ceil(1_000_000);
    if priority_fee > MAX_PRIORITY_FEE_LAMPORTS {
        bail!("aggregator priority fee exceeds preflight cap");
    }
    Ok((limit, priority_fee))
}

fn validate_fee_metadata(
    reported_limit: Option<u64>,
    reported_fee: Option<u64>,
    decoded_limit: u64,
    decoded_fee: u64,
) -> Result<()> {
    if let Some(limit) = reported_limit {
        if limit != decoded_limit {
            bail!("aggregator compute limit mismatch: reported {limit}, decoded {decoded_limit}");
        }
    }
    if let Some(fee) = reported_fee {
        if fee.abs_diff(decoded_fee) > MAX_PRIORITY_FEE_METADATA_ROUNDING_LAMPORTS {
            bail!("aggregator priority fee mismatch: reported {fee}, decoded {decoded_fee}");
        }
    }
    Ok(())
}

pub use crate::adapters::cache::quotes::QuoteStore;

#[derive(Deserialize)]
struct JupiterQuote {
    #[serde(rename = "inputMint")]
    input_mint: String,
    #[serde(rename = "outputMint")]
    output_mint: String,
    #[serde(rename = "inAmount")]
    in_amount: String,
    #[serde(rename = "outAmount")]
    out_amount: String,
    #[serde(rename = "otherAmountThreshold")]
    other_amount_threshold: String,
    #[serde(rename = "swapMode")]
    swap_mode: String,
    #[serde(rename = "slippageBps")]
    slippage_bps: u16,
    #[serde(rename = "priceImpactPct")]
    price_impact_pct: String,
    #[serde(rename = "routePlan")]
    route_plan: Vec<JupiterLeg>,
}

#[derive(Deserialize)]
struct JupiterLeg {
    #[serde(rename = "swapInfo")]
    swap_info: JupiterSwapInfo,
}

#[derive(Deserialize)]
struct JupiterSwapInfo {
    label: String,
    #[serde(rename = "inputMint")]
    input_mint: String,
    #[serde(rename = "outputMint")]
    output_mint: String,
}

pub fn validate_request(request: &SolBuyQuoteRequest) -> Result<()> {
    if request.wallet_id.trim().is_empty() || request.wallet_id.len() > 128 {
        bail!("wallet_id is required and must be at most 128 characters");
    }
    Pubkey::from_str(&request.mint).context("mint must be a valid pubkey")?;
    if request.mint == WSOL_MINT {
        bail!("output mint must differ from WSOL");
    }
    if request.amount_lamports < 100_000 || request.amount_lamports > 100_000_000 {
        bail!("amount_lamports must be between 0.0001 and 0.1 SOL");
    }
    if request.slippage_bps == 0 || request.slippage_bps > 3000 {
        bail!("slippage_bps must be between 1 and 3000");
    }
    Ok(())
}

pub async fn quote(request: &SolBuyQuoteRequest) -> Result<SolBuyQuoteResponse> {
    validate_request(request)?;
    let quote: JupiterQuote = serde_json::from_value(fetch_quote(request).await?)?;
    validate_quote(request, quote)
}

async fn fetch_quote(request: &SolBuyQuoteRequest) -> Result<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let response = client
        .get(JUPITER_QUOTE_URL)
        .query(&[
            ("inputMint", WSOL_MINT.to_string()),
            ("outputMint", request.mint.clone()),
            ("amount", request.amount_lamports.to_string()),
            ("slippageBps", request.slippage_bps.to_string()),
        ])
        .send()
        .await
        .context("aggregator quote unavailable")?;
    if !response.status().is_success() {
        bail!("aggregator quote returned {}", response.status());
    }
    response.json().await.context("invalid aggregator quote")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JupiterSwapRequest<'a> {
    quote_response: &'a serde_json::Value,
    user_public_key: &'a str,
    wrap_and_unwrap_sol: bool,
    dynamic_compute_unit_limit: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterSwapResponse {
    swap_transaction: Option<String>,
    simulation_error: Option<serde_json::Value>,
    prioritization_fee_lamports: Option<u64>,
    compute_unit_limit: Option<u64>,
}

pub(crate) async fn prepare_transaction(
    bound: &BoundQuote,
    wallet: &SdkTradeClient,
    read_rpc: &crate::adapters::rpc::read_runtime::SharedReadRuntime,
) -> Result<(VersionedTransaction, QuotePreflight)> {
    let request = SolBuyQuoteRequest {
        wallet_id: bound.wallet_id.clone(),
        mint: bound.quote.output_mint.clone(),
        amount_lamports: bound.quote.amount_lamports,
        slippage_bps: bound.quote_slippage_bps,
    };
    let fresh_raw = fetch_quote(&request).await?;
    let fresh = QuoteEstimate::from(validate_quote(
        &request,
        serde_json::from_value(fresh_raw.clone())?,
    )?);
    if fresh.route != bound.quote.route
        || fresh.minimum_output_units < bound.quote.minimum_output_units
    {
        bail!("aggregator route or minimum output changed; request a new quote");
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let response = client
        .post(JUPITER_SWAP_URL)
        .json(&JupiterSwapRequest {
            quote_response: &fresh_raw,
            user_public_key: &bound.wallet_pubkey,
            wrap_and_unwrap_sol: true,
            dynamic_compute_unit_limit: true,
        })
        .send()
        .await
        .context("aggregator unsigned transaction unavailable")?;
    if !response.status().is_success() {
        bail!(
            "aggregator unsigned transaction returned {}",
            response.status()
        );
    }
    let swap: JupiterSwapResponse = response
        .json()
        .await
        .context("invalid aggregator swap response")?;
    if swap.simulation_error.is_some() {
        bail!("aggregator reported simulation error");
    }
    let encoded = swap
        .swap_transaction
        .context("aggregator did not return a transaction")?;
    if encoded.len() > 2048 {
        bail!("aggregator transaction response is too large");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("invalid transaction base64")?;
    if bytes.len() > 1232 {
        bail!("aggregator transaction exceeds Solana packet size");
    }
    let transaction: VersionedTransaction =
        bincode::deserialize(&bytes).context("invalid unsigned transaction")?;
    transaction
        .sanitize()
        .map_err(|_| anyhow::anyhow!("aggregator transaction failed Solana sanitization"))?;
    let payer = transaction
        .message
        .static_account_keys()
        .first()
        .context("transaction has no payer")?;
    if payer.to_string() != bound.wallet_pubkey
        || transaction.message.header().num_required_signatures != 1
        || transaction.signatures.len() != 1
        || transaction.signatures[0] != Signature::default()
    {
        bail!("aggregator transaction payer or signature count does not match quote wallet");
    }
    let instruction_count = transaction.message.instructions().len();
    if instruction_count == 0 || instruction_count > 32 {
        bail!("aggregator transaction has unsupported instruction count");
    }
    if matches!(&transaction.message, VersionedMessage::V1(_)) {
        bail!("aggregator message version is not supported for account audit");
    }
    let static_keys = transaction.message.static_account_keys();
    let lookups = transaction.message.address_table_lookups().unwrap_or(&[]);
    let lookup_count = lookups.len();
    if lookup_count > 4 {
        bail!("aggregator transaction has too many address lookup tables");
    }
    let mut writable = Vec::new();
    let mut readonly = Vec::new();
    for lookup in lookups {
        let table = wallet
            .resolve_aggregator_lookup_table(&lookup.account_key, read_rpc)
            .await?;
        for index in &lookup.writable_indexes {
            writable.push(
                *table
                    .get(usize::from(*index))
                    .context("invalid writable lookup index")?,
            );
        }
        for index in &lookup.readonly_indexes {
            readonly.push(
                *table
                    .get(usize::from(*index))
                    .context("invalid readonly lookup index")?,
            );
        }
    }
    let mut resolved_keys = Vec::with_capacity(static_keys.len() + writable.len() + readonly.len());
    resolved_keys.extend_from_slice(static_keys);
    resolved_keys.extend(writable);
    resolved_keys.extend(readonly);
    if resolved_keys.len() > 256 {
        bail!("aggregator transaction has too many resolved accounts");
    }
    if resolved_keys.iter().copied().collect::<HashSet<_>>().len() != resolved_keys.len() {
        bail!("aggregator transaction has duplicate resolved accounts");
    }
    let writable_account_count = (0..resolved_keys.len())
        .filter(|index| {
            transaction
                .message
                .is_maybe_writable_with_reserved_addresses(*index, None::<&HashSet<Pubkey>>)
        })
        .count();
    if writable_account_count > MAX_WRITABLE_ACCOUNTS {
        bail!("aggregator transaction has too many writable accounts");
    }
    let mint = Pubkey::from_str(&bound.quote.output_mint).context("invalid bound target mint")?;
    let (target_ata, token_program) = wallet.aggregator_target_ata(&mint, read_rpc).await?;
    let target_index =
        require_writable_target_ata(&transaction.message, &resolved_keys, &target_ata)?;
    let mut top_level_programs = Vec::new();
    for instruction in transaction.message.instructions() {
        if instruction.data.len() > 1024 {
            bail!("aggregator instruction data exceeds audit limit");
        }
        let program_index = usize::from(instruction.program_id_index);
        if program_index >= resolved_keys.len()
            || instruction
                .accounts
                .iter()
                .any(|index| usize::from(*index) >= resolved_keys.len())
        {
            bail!("aggregator instruction references an invalid account index");
        }
        let accounts = instruction
            .accounts
            .iter()
            .map(|index| resolved_keys[usize::from(*index)])
            .collect::<Vec<_>>();
        check_target_token_instruction(
            &resolved_keys[program_index],
            &instruction.data,
            &accounts,
            &target_ata,
        )?;
        let program = resolved_keys[program_index].to_string();
        if !top_level_programs.contains(&program) {
            top_level_programs.push(program);
        }
    }
    let (compute_unit_limit, calculated_priority_fee_lamports) =
        read_compute_budget_fee(transaction.message.instructions(), &resolved_keys)?;
    let explicit_system_spend_lamports = read_explicit_system_spend(
        transaction.message.instructions(),
        &resolved_keys,
        payer,
        bound.quote.amount_lamports,
    )?;
    validate_fee_metadata(
        swap.compute_unit_limit,
        swap.prioritization_fee_lamports,
        compute_unit_limit,
        calculated_priority_fee_lamports,
    )?;
    let simulation = wallet
        .simulate_unsigned_aggregator_transaction(
            &transaction,
            target_index,
            &mint,
            &target_ata,
            &token_program,
            &resolved_keys,
        )
        .await?;
    if let Some(delta) = simulation.target_token_delta_units {
        if delta < bound.quote.minimum_output_units {
            bail!("local simulation target token output is below quote minimum");
        }
    }
    if let Some(fee) = simulation.fee_lamports {
        if fee > calculated_priority_fee_lamports.saturating_add(MAX_BASE_FEE_LAMPORTS) {
            bail!("local simulation fee exceeds preflight cap");
        }
    }
    if let Some(debit) = simulation.payer_debit_lamports {
        let maximum_debit = bound
            .quote
            .amount_lamports
            .saturating_add(MAX_EXTRA_SYSTEM_SPEND_LAMPORTS)
            .saturating_add(MAX_PRIORITY_FEE_LAMPORTS)
            .saturating_add(MAX_BASE_FEE_LAMPORTS);
        if debit > maximum_debit {
            bail!("local simulation payer debit exceeds preflight cap");
        }
    }
    if let Some(inner_spend) = simulation.inner_payer_system_spend_lamports {
        let combined_spend = explicit_system_spend_lamports
            .checked_add(inner_spend)
            .context("combined System transfer total overflow")?;
        if combined_spend
            > bound
                .quote
                .amount_lamports
                .saturating_add(MAX_EXTRA_SYSTEM_SPEND_LAMPORTS)
        {
            bail!("simulated inner payer System transfers exceed preflight cap");
        }
    }
    let preflight = QuotePreflight {
        quote_id: bound.quote_id.clone(),
        wallet_id: bound.wallet_id.clone(),
        quote_still_valid: true,
        unsigned_transaction_built: true,
        aggregator_simulation_ok: true,
        local_simulation_ok: simulation.ok,
        instruction_count,
        address_lookup_table_count: lookup_count,
        resolved_account_count: resolved_keys.len(),
        instruction_audit_passed: true,
        executable: simulation.ok,
        reason: if simulation.ok {
            "route, target account, fee bounds and local unsigned simulation passed"
        } else {
            "local unsigned simulation failed; transaction is not executable"
        }
        .to_string(),
        prioritization_fee_lamports: calculated_priority_fee_lamports,
        compute_unit_limit,
        calculated_priority_fee_lamports,
        explicit_system_spend_lamports,
        writable_account_count,
        target_ata: target_ata.to_string(),
        top_level_programs,
        simulated_fee_lamports: simulation.fee_lamports,
        simulated_units_consumed: simulation.units_consumed,
        simulated_payer_debit_lamports: simulation.payer_debit_lamports,
        simulated_inner_instruction_groups: simulation.inner_instruction_groups,
        simulated_target_token_delta_units: simulation.target_token_delta_units,
        simulated_target_token_post_units: simulation.target_token_post_units,
        baseline_slot: simulation.baseline_slot,
        simulation_slot: simulation.simulation_slot,
        observed_same_slot_payer_debit_lamports: simulation.observed_same_slot_payer_debit_lamports,
        observed_same_slot_target_token_delta_units: simulation
            .observed_same_slot_target_token_delta_units,
        simulated_inner_instruction_count: simulation.inner_instruction_count,
        simulated_inner_programs: simulation.inner_programs,
        simulated_inner_payer_system_spend_lamports: simulation.inner_payer_system_spend_lamports,
    };
    Ok((transaction, preflight))
}

fn validate_quote(
    request: &SolBuyQuoteRequest,
    quote: JupiterQuote,
) -> Result<SolBuyQuoteResponse> {
    if quote.input_mint != WSOL_MINT
        || quote.output_mint != request.mint
        || quote.swap_mode != "ExactIn"
        || quote.slippage_bps != request.slippage_bps
    {
        bail!("aggregator quote does not match requested trade");
    }
    if quote.in_amount.parse::<u64>()? != request.amount_lamports {
        bail!("aggregator quote changed input amount");
    }
    let expected_output_units = quote.out_amount.parse::<u64>()?;
    let minimum_output_units = quote.other_amount_threshold.parse::<u64>()?;
    if expected_output_units == 0
        || minimum_output_units == 0
        || minimum_output_units > expected_output_units
    {
        bail!("aggregator quote has invalid output amount");
    }
    let minimum_allowed = (expected_output_units as u128)
        .saturating_mul(10_000 - request.slippage_bps as u128)
        / 10_000;
    if (minimum_output_units as u128) < minimum_allowed {
        bail!("aggregator quote exceeds requested slippage");
    }
    if quote.route_plan.is_empty() || quote.route_plan.len() > 4 {
        bail!("aggregator quote has unsupported route length");
    }
    let mut current_mint = WSOL_MINT.to_string();
    let mut route = Vec::with_capacity(quote.route_plan.len());
    for leg in quote.route_plan {
        if leg.swap_info.input_mint != current_mint || leg.swap_info.label.is_empty() {
            bail!("aggregator quote has discontinuous route");
        }
        Pubkey::from_str(&leg.swap_info.output_mint).context("invalid route mint")?;
        current_mint = leg.swap_info.output_mint.clone();
        route.push(QuoteLeg {
            label: leg.swap_info.label,
            input_mint: leg.swap_info.input_mint,
            output_mint: leg.swap_info.output_mint,
        });
    }
    if current_mint != request.mint {
        bail!("aggregator route does not end at requested mint");
    }
    let price_impact = quote.price_impact_pct.parse::<f64>()?;
    if !price_impact.is_finite() || price_impact < 0.0 || price_impact > 10.0 {
        bail!("aggregator quote exceeds price impact limit");
    }
    Ok(SolBuyQuoteResponse {
        input_mint: WSOL_MINT.to_string(),
        output_mint: request.mint.clone(),
        amount_lamports: request.amount_lamports,
        expected_output_units,
        minimum_output_units,
        price_impact_percent: quote.price_impact_pct,
        route,
        executable: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_metadata_allows_one_lamport_rounding_only() {
        assert!(validate_fee_metadata(Some(234_553), Some(99_999), 234_553, 100_000).is_ok());
        assert!(validate_fee_metadata(Some(234_554), Some(99_999), 234_553, 100_000).is_err());
        assert!(validate_fee_metadata(Some(234_553), Some(99_998), 234_553, 100_000).is_err());
        assert!(validate_fee_metadata(None, None, 234_553, 100_000).is_ok());
    }

    #[test]
    fn target_ata_must_be_present_and_writable() {
        let payer = Pubkey::new_unique();
        let ata = Pubkey::new_unique();
        let message = VersionedMessage::Legacy(solana_sdk::message::Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![payer, ata],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            instructions: vec![],
        });
        assert!(require_writable_target_ata(&message, &[payer, ata], &ata).is_ok());
        assert!(
            require_writable_target_ata(&message, &[payer, ata], &Pubkey::new_unique()).is_err()
        );
        let readonly_message = VersionedMessage::Legacy(solana_sdk::message::Message {
            header: solana_sdk::message::MessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            },
            account_keys: vec![payer, ata],
            recent_blockhash: solana_sdk::hash::Hash::default(),
            instructions: vec![],
        });
        assert!(require_writable_target_ata(&readonly_message, &[payer, ata], &ata).is_err());
    }

    fn system_transfer(
        sender_index: u8,
        lamports: u64,
    ) -> solana_sdk::message::compiled_instruction::CompiledInstruction {
        solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![sender_index, 2],
            data: [vec![2, 0, 0, 0], lamports.to_le_bytes().to_vec()].concat(),
        }
    }

    #[test]
    fn system_spend_rejects_wrong_payer_unsupported_instruction_and_excess() {
        let payer = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let keys = [Pubkey::default(), payer, other];
        assert_eq!(
            read_explicit_system_spend(&[system_transfer(1, 1_000_000)], &keys, &payer, 1_000_000)
                .unwrap(),
            1_000_000
        );
        assert!(
            read_explicit_system_spend(&[system_transfer(2, 1)], &keys, &payer, 1_000_000).is_err()
        );
        assert!(read_explicit_system_spend(
            &[system_transfer(1, 21_000_001)],
            &keys,
            &payer,
            1_000_000
        )
        .is_err());
        let mut unsupported = system_transfer(1, 1);
        unsupported.data[0] = 0;
        assert!(read_explicit_system_spend(&[unsupported], &keys, &payer, 1_000_000).is_err());
        assert!(read_explicit_system_spend(
            &[system_transfer(1, u64::MAX), system_transfer(1, 1)],
            &keys,
            &payer,
            u64::MAX
        )
        .is_err());
    }

    #[test]
    fn compute_budget_fee_is_bounded_and_duplicates_rejected() {
        let program = Pubkey::from_str(COMPUTE_BUDGET_PROGRAM).unwrap();
        let keys = [program];
        let limit = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: [vec![2], 1_000_000u32.to_le_bytes().to_vec()].concat(),
        };
        let price = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: [vec![3], 10_000u64.to_le_bytes().to_vec()].concat(),
        };
        assert_eq!(
            read_compute_budget_fee(&[limit.clone(), price], &keys).unwrap(),
            (1_000_000, 10_000)
        );
        assert!(read_compute_budget_fee(&[limit.clone(), limit], &keys).is_err());
        let excessive = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: [vec![3], 200_000u64.to_le_bytes().to_vec()].concat(),
        };
        let normal = solana_sdk::message::compiled_instruction::CompiledInstruction {
            program_id_index: 0,
            accounts: vec![],
            data: [vec![2], 1_000_000u32.to_le_bytes().to_vec()].concat(),
        };
        assert!(read_compute_budget_fee(&[normal, excessive], &keys).is_err());
    }

    fn sample_quote() -> JupiterQuote {
        serde_json::from_value(serde_json::json!({
            "inputMint": WSOL_MINT,
            "outputMint": "2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL",
            "inAmount": "1000000",
            "outAmount": "19441607746",
            "otherAmountThreshold": "16525366585",
            "swapMode": "ExactIn",
            "slippageBps": 1500,
            "priceImpactPct": "0.0373",
            "routePlan": [
                {"swapInfo": {"label":"Byreal","inputMint":WSOL_MINT,"outputMint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"}},
                {"swapInfo": {"label":"Raydium CLMM","inputMint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v","outputMint":"Xsf9mBktVB9BSU5kf4nHxPq5hCBJ2j2ui3ecFGxPRGc"}},
                {"swapInfo": {"label":"Raydium CP","inputMint":"Xsf9mBktVB9BSU5kf4nHxPq5hCBJ2j2ui3ecFGxPRGc","outputMint":"2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL"}}
            ]
        })).unwrap()
    }

    fn sample_request() -> SolBuyQuoteRequest {
        SolBuyQuoteRequest {
            wallet_id: "2TE".into(),
            mint: "2GieYk6QjWsAqmiY68EMbRqdKda7oRRjLBPbmbnW7LpL".into(),
            amount_lamports: 1_000_000,
            slippage_bps: 1500,
        }
    }

    #[test]
    fn rejects_invalid_amount_and_slippage() {
        let mut request = sample_request();
        assert!(validate_request(&request).is_ok());
        request.amount_lamports = 0;
        assert!(validate_request(&request).is_err());
        request.amount_lamports = 1_000_000;
        request.slippage_bps = 5000;
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn accepts_contiguous_three_hop_quote() {
        let quote = validate_quote(&sample_request(), sample_quote()).unwrap();
        assert_eq!(quote.route.len(), 3);
        assert!(!quote.executable);
        assert_eq!(quote.amount_lamports, 1_000_000);
    }

    #[test]
    fn rejects_changed_input_and_broken_route() {
        let mut changed = sample_quote();
        changed.in_amount = "2000000".into();
        assert!(validate_quote(&sample_request(), changed).is_err());
        let mut broken = sample_quote();
        broken.route_plan[1].swap_info.input_mint = WSOL_MINT.into();
        assert!(validate_quote(&sample_request(), broken).is_err());
    }
}
