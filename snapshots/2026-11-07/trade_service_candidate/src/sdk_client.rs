use anyhow::{anyhow, bail, Context};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use sol_trade_sdk::common::{
    fast_fn::get_associated_token_address_with_program_id_fast, spl_token, spl_token_2022,
    GasFeeStrategy, GasFeeStrategyType, TradeConfig,
};
use sol_trade_sdk::instruction::meteora_damm_v2::MeteoraDammV2InstructionBuilder;
use sol_trade_sdk::instruction::pumpfun::PumpFunInstructionBuilder;
use sol_trade_sdk::instruction::pumpswap::PumpSwapInstructionBuilder;
use sol_trade_sdk::instruction::stonkfun::StonkFunInstructionBuilder;
use sol_trade_sdk::swqos::{
    AstralaneTransport, SwqosConfig, SwqosRegion, SwqosTransport, SwqosType, TradeType,
};
use sol_trade_sdk::trading::core::params::SwapParams;
use sol_trade_sdk::trading::core::params::{
    BonkParams, DexParamEnum, MeteoraDammV2Params, PumpFunParams, PumpFunQuoteHop,
    PumpFunQuoteRoute, PumpFunQuoteVenue, PumpFunV3Params, PumpSwapParams, PumpSwapQuoteHop,
    PumpSwapQuoteRoute, PumpSwapQuoteVenue, RaydiumAmmV4Params, RaydiumCpmmParams, StonkFunParams,
    StonkFunQuoteHop, StonkFunQuoteRoute, StonkFunQuoteVenue, StonkFunViaSolParams,
    TokenTransferFee,
};
use sol_trade_sdk::trading::core::traits::InstructionBuilder;
use sol_trade_sdk::trading::factory::DexType;
use sol_trade_sdk::{
    find_pool_by_mint, TradeBuyParams, TradeSellParams, TradeTokenType, TradingClient,
};
use solana_account_decoder::{UiAccountData, UiAccountEncoding};
use solana_address_lookup_table_interface::{program as lookup_program, state::AddressLookupTable};
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_config::{
    RpcSendTransactionConfig, RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_client::rpc_filter::Memcmp;
use solana_client::rpc_filter::RpcFilterType;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::hash::Hash;
use solana_sdk::message::{Message, VersionedMessage};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature, Signer};
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::VersionedTransaction;
use solana_system_interface::instruction as system_instruction;
use solana_transaction_status_client_types::UiTransactionEncoding;
use solana_transaction_status_client_types::{
    UiInnerInstructions, UiInstruction, UiParsedInstruction,
};
use spl_token_2022_interface::{
    extension::StateWithExtensions, instruction::TokenInstruction as Token2022Instruction,
    state::Account as Token2022Account,
};
use spl_token_interface::{instruction::TokenInstruction, state::Account as TokenAccount};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};
use tracing::{info, warn};

use crate::config::{MevConfig, ProviderConfig};
use crate::models::{
    MevProvider, Pool, ResolvePoolResponse, Side, SimulationReport, USD1_MINT, USDC_MINT, WSOL_MINT,
};
use crate::pumpapi_client::{self, PumpApiTrade};
use crate::sanitize::redact_urls;

pub struct SdkTradeClient {
    lanes: Vec<SdkLane>,
    sol_hop_cache: Mutex<HashMap<Pubkey, (Pubkey, Instant)>>,
    swqos_lanes: Vec<String>,
    mev_protection: bool,
    require_mev_submit: bool,
    rpc_count: usize,
}

/// Stable metadata produced by the SDK boundary after an RPC account read.
/// The backend converts this into `ResolvedRoute` without exposing SDK
/// parameter structs to HTTP or persistence layers.
#[derive(Debug, Clone)]
pub struct VerifiedRouteMetadata {
    pub program_id: Pubkey,
    pub pool_id: Option<Pubkey>,
    pub input_mint: Option<Pubkey>,
    pub output_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub token_program: Pubkey,
    pub quote_token_program: Pubkey,
    pub validated_slot: u64,
    pub pool_owner_verified: bool,
    pub mint_owner_verified: bool,
    pub quote_mint_owner_verified: bool,
}

/// Read-only evidence that SDK 11 can quote and assemble a two-leg
/// route from current RPC parameters. Optional RPC
/// simulation uses an unsigned transaction; no submission occurs.
#[derive(Debug, Clone, Serialize)]
pub struct SdkTwoHopBuildPreview {
    pub builder: &'static str,
    pub input_lamports: u64,
    pub bridge_min_output_units: u64,
    pub target_min_output_units: u64,
    pub instruction_count: usize,
    pub program_ids: Vec<String>,
    pub validated_slot: u64,
    pub snapshot_slot_span: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation: Option<SdkTwoHopSimulation>,
}

/// Wallet-bound, read-only reverse-route build evidence. This is not a trade
/// credential and does not authorize signing, simulation orders, or submission.
#[derive(Debug, Clone, Serialize)]
pub struct SdkTwoHopSellBuildPreview {
    pub builder: &'static str,
    pub input_token_units: u64,
    pub input_token_ata: String,
    pub input_token_ata_balance_units: u64,
    pub target_min_quote_units: u64,
    pub bridge_min_wsol_units: u64,
    pub instruction_count: usize,
    pub program_ids: Vec<String>,
    pub validated_slot: u64,
    pub snapshot_slot_span: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation: Option<SdkTwoHopSellSimulation>,
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SdkTwoHopSellSimulation {
    pub attempted: bool,
    pub rpc_accepted: bool,
    pub ok: bool,
    pub error: Option<String>,
    pub slot: u64,
    pub units_consumed: Option<u64>,
    pub transaction_bytes: usize,
    pub transaction_version: &'static str,
    pub input_token_debit_units: Option<u64>,
    pub wsol_credit_units: Option<u64>,
    pub balance_and_minimum_verified: bool,
}

/// Keeps the exact message used for the final RPC simulation available to the
/// SDK execution path. The read-only HTTP preview discards this value.
pub(crate) struct SdkTwoHopSellPrepared {
    pub(crate) preview: SdkTwoHopSellBuildPreview,
    unsigned: Option<SdkTwoHopUnsignedTransaction>,
}

/// The CPMM buy keeps the exact unsigned message used by its final RPC
/// simulation. The public preview only exposes the read-only summary.
pub(crate) struct SdkTwoHopBuyPrepared {
    pub(crate) preview: SdkTwoHopBuildPreview,
    unsigned: Option<SdkTwoHopUnsignedTransaction>,
}

impl SdkTwoHopBuyPrepared {
    pub(crate) fn into_signed_transaction(
        self,
        signer: &Keypair,
    ) -> anyhow::Result<VersionedTransaction> {
        let simulation = self
            .preview
            .simulation
            .context("SDK two-hop buy has no RPC simulation")?;
        anyhow::ensure!(
            simulation.attempted
                && simulation.rpc_accepted
                && simulation.ok
                && simulation.minimum_output_verified
                && simulation
                    .payer_debit_lamports
                    .is_some_and(|debit| { debit >= self.preview.input_lamports })
                && simulation
                    .target_token_delta_units
                    .is_some_and(|delta| { delta >= self.preview.target_min_output_units }),
            "SDK two-hop buy simulation did not verify the protected output"
        );
        self.unsigned
            .context("SDK two-hop buy has no prepared transaction")?
            .into_signed_transaction(signer)
    }
}

impl SdkTwoHopSellPrepared {
    pub(crate) fn into_signed_transaction(
        self,
        signer: &Keypair,
    ) -> anyhow::Result<VersionedTransaction> {
        let simulation = self
            .preview
            .simulation
            .context("SDK two-hop sell has no RPC simulation")?;
        anyhow::ensure!(
            simulation.attempted
                && simulation.rpc_accepted
                && simulation.ok
                && simulation.balance_and_minimum_verified
                && simulation.input_token_debit_units == Some(self.preview.input_token_units)
                && simulation
                    .wsol_credit_units
                    .is_some_and(|credit| { credit >= self.preview.bridge_min_wsol_units }),
            "SDK two-hop sell simulation did not verify the protected balances"
        );
        self.unsigned
            .context("SDK two-hop sell has no prepared transaction")?
            .into_signed_transaction(signer)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SdkTwoHopSimulation {
    pub attempted: bool,
    pub rpc_accepted: bool,
    pub ok: bool,
    pub error: Option<String>,
    pub slot: u64,
    pub units_consumed: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub transaction_bytes: usize,
    pub transaction_version: &'static str,
    pub payer_debit_lamports: Option<u64>,
    pub target_token_post_units: Option<u64>,
    pub target_token_delta_units: Option<u64>,
    pub minimum_output_verified: bool,
}

struct SdkLane {
    swqos_type: SwqosType,
    is_helius: bool,
    client: TradingClient,
}

fn is_helius_rpc(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host == "helius-rpc.com" || host.ends_with(".helius-rpc.com"))
        })
        .unwrap_or(false)
}

fn pool_resolution_lane_order(helius_lanes: &[bool], swap_helius: bool) -> Vec<usize> {
    let mut order = (0..helius_lanes.len()).collect::<Vec<_>>();
    let helius = helius_lanes
        .iter()
        .enumerate()
        .filter_map(|(index, is_helius)| is_helius.then_some(index))
        .take(2)
        .collect::<Vec<_>>();
    if helius.len() == 2 {
        order.retain(|index| !helius.contains(index));
        let first = usize::from(swap_helius);
        order.insert(0, helius[1 - first]);
        order.insert(0, helius[first]);
    }
    order
}

#[derive(Clone)]
pub struct ExecuteTrade {
    pub pool: Pool,
    pub mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool_id: Option<Pubkey>,
    pub sol_hop_pool: Option<Pool>,
    pub sol_hop_pool_id: Option<Pubkey>,
    pub amount_lamports: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_bps: u64,
    pub tip_sol: f64,
    pub recent_blockhash: Hash,
    pub wait_tx_confirmed: bool,
    pub mev_submit_verify_ms: u64,
    pub simulate: bool,
    pub use_mev: Option<bool>,
    pub mev_provider: Option<MevProvider>,
}

pub struct ExecuteResult {
    pub success: bool,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub simulation: Option<SimulationReport>,
}

pub struct SolTransferResult {
    pub status: String,
    pub signature: String,
    pub amount_lamports: u64,
    pub fee_lamports: u64,
    pub balance_before_lamports: u64,
}

pub struct AggregatorSimulation {
    pub ok: bool,
    pub baseline_payer_lamports: u64,
    pub simulated_pre_payer_lamports: Option<u64>,
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

pub(crate) fn check_target_token_instruction(
    program: &Pubkey,
    data: &[u8],
    accounts: &[Pubkey],
    target_ata: &Pubkey,
) -> anyhow::Result<()> {
    if accounts.first() != Some(target_ata) {
        return Ok(());
    }
    let initialization = if *program == spl_token::ID {
        matches!(
            TokenInstruction::unpack(data).context("invalid SPL target ATA instruction")?,
            TokenInstruction::InitializeAccount
                | TokenInstruction::InitializeAccount2 { .. }
                | TokenInstruction::InitializeAccount3 { .. }
                | TokenInstruction::InitializeImmutableOwner
        )
    } else if *program == spl_token_2022::ID {
        matches!(
            Token2022Instruction::unpack(data)
                .context("invalid Token-2022 target ATA instruction")?,
            Token2022Instruction::InitializeAccount
                | Token2022Instruction::InitializeAccount2 { .. }
                | Token2022Instruction::InitializeAccount3 { .. }
                | Token2022Instruction::InitializeImmutableOwner
        )
    } else {
        return Ok(());
    };
    if !initialization {
        bail!("token instruction attempts to modify target ATA beyond initialization");
    }
    Ok(())
}

fn check_inner_target_token_instructions(
    groups: &[UiInnerInstructions],
    keys: &[Pubkey],
    target_ata: &Pubkey,
) -> anyhow::Result<()> {
    for group in groups {
        for instruction in &group.instructions {
            match instruction {
                UiInstruction::Compiled(ix) => {
                    let program = keys
                        .get(usize::from(ix.program_id_index))
                        .context("invalid inner token program index")?;
                    let accounts = ix
                        .accounts
                        .iter()
                        .map(|index| {
                            keys.get(usize::from(*index))
                                .copied()
                                .context("invalid inner token account index")
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    if *program == spl_token::ID || *program == spl_token_2022::ID {
                        let data = bs58::decode(&ix.data)
                            .into_vec()
                            .context("invalid inner token instruction data")?;
                        check_target_token_instruction(program, &data, &accounts, target_ata)?;
                    }
                }
                UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix)) => {
                    let program =
                        Pubkey::from_str(&ix.program_id).context("invalid inner token program")?;
                    if program == spl_token::ID || program == spl_token_2022::ID {
                        let accounts = ix
                            .accounts
                            .iter()
                            .map(|address| {
                                Pubkey::from_str(address).context("invalid inner token account")
                            })
                            .collect::<anyhow::Result<Vec<_>>>()?;
                        let data = bs58::decode(&ix.data)
                            .into_vec()
                            .context("invalid inner token instruction data")?;
                        check_target_token_instruction(&program, &data, &accounts, target_ata)?;
                    }
                }
                UiInstruction::Parsed(UiParsedInstruction::Parsed(ix)) => {
                    let program =
                        Pubkey::from_str(&ix.program_id).context("invalid inner token program")?;
                    let target = target_ata.to_string();
                    let target_is_source_or_account =
                        ["/info/source", "/info/account"].iter().any(|path| {
                            ix.parsed.pointer(path).and_then(|value| value.as_str())
                                == Some(target.as_str())
                        });
                    if (program == spl_token::ID || program == spl_token_2022::ID)
                        && target_is_source_or_account
                        && !matches!(
                            ix.parsed.get("type").and_then(|value| value.as_str()),
                            Some(
                                "initializeAccount"
                                    | "initializeAccount2"
                                    | "initializeAccount3"
                                    | "initializeImmutableOwner"
                            )
                        )
                    {
                        bail!("parsed token instruction references target ATA unexpectedly");
                    }
                }
            }
        }
    }
    Ok(())
}

fn decoded_system_transfer(
    data: &[u8],
    accounts: &[Pubkey],
    payer: &Pubkey,
) -> Option<(u64, Option<Pubkey>)> {
    if data.len() != 12 || data[..4] != [2, 0, 0, 0] || accounts.len() != 2 {
        return None;
    }
    if accounts[0] != *payer {
        return Some((0, None));
    }
    Some((
        u64::from_le_bytes(data[4..12].try_into().ok()?),
        Some(accounts[1]),
    ))
}

fn inner_payer_system_spend(
    groups: &[UiInnerInstructions],
    keys: &[Pubkey],
    payer: &Pubkey,
    wsol_ata: &Pubkey,
) -> anyhow::Result<Option<u64>> {
    let system_program = Pubkey::default().to_string();
    let mut total = 0u64;
    let mut complete = true;
    for group in groups {
        for instruction in &group.instructions {
            let transfer = match instruction {
                UiInstruction::Compiled(ix)
                    if keys.get(usize::from(ix.program_id_index)) == Some(&Pubkey::default()) =>
                {
                    let accounts = ix
                        .accounts
                        .iter()
                        .map(|index| {
                            keys.get(usize::from(*index))
                                .copied()
                                .context("invalid inner System account")
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    bs58::decode(&ix.data)
                        .into_vec()
                        .ok()
                        .and_then(|data| decoded_system_transfer(&data, &accounts, payer))
                }
                UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix))
                    if ix.program_id == system_program =>
                {
                    let accounts = ix
                        .accounts
                        .iter()
                        .map(|address| {
                            Pubkey::from_str(address).context("invalid inner System account")
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    bs58::decode(&ix.data)
                        .into_vec()
                        .ok()
                        .and_then(|data| decoded_system_transfer(&data, &accounts, payer))
                }
                UiInstruction::Parsed(UiParsedInstruction::Parsed(ix))
                    if ix.program_id == system_program =>
                {
                    if ix.parsed.get("type").and_then(|value| value.as_str()) != Some("transfer") {
                        None
                    } else {
                        match ix
                            .parsed
                            .pointer("/info/source")
                            .and_then(|value| value.as_str())
                        {
                            Some(source) if source == payer.to_string() => {
                                let amount = ix
                                    .parsed
                                    .pointer("/info/lamports")
                                    .and_then(|value| value.as_u64());
                                let recipient = ix
                                    .parsed
                                    .pointer("/info/destination")
                                    .and_then(|value| value.as_str())
                                    .and_then(|value| Pubkey::from_str(value).ok());
                                amount.zip(recipient.map(Some))
                            }
                            Some(_) => Some((0, None)),
                            None => None,
                        }
                    }
                }
                _ => continue,
            };
            let Some((amount, recipient)) = transfer else {
                complete = false;
                continue;
            };
            if recipient.is_some_and(|recipient| recipient != *wsol_ata) {
                bail!("simulated inner payer System transfer has unexpected recipient");
            }
            total = total
                .checked_add(amount)
                .context("inner System transfer total overflow")?;
        }
    }
    Ok(complete.then_some(total))
}

fn audit_inner_instruction_inventory(
    groups: &[UiInnerInstructions],
    keys: &[Pubkey],
    outer_count: usize,
) -> anyhow::Result<(usize, Vec<String>)> {
    let mut count = 0usize;
    let mut programs = Vec::new();
    for group in groups {
        if usize::from(group.index) >= outer_count {
            bail!("simulated inner instruction group has invalid outer index");
        }
        for instruction in &group.instructions {
            count += 1;
            if count > 128 {
                bail!("simulated transaction has too many inner instructions");
            }
            let program = match instruction {
                UiInstruction::Compiled(ix) => {
                    for index in &ix.accounts {
                        if usize::from(*index) >= keys.len() {
                            bail!("simulated inner instruction has invalid account index");
                        }
                    }
                    *keys
                        .get(usize::from(ix.program_id_index))
                        .context("simulated inner instruction has invalid program index")?
                }
                UiInstruction::Parsed(UiParsedInstruction::Parsed(ix)) => {
                    Pubkey::from_str(&ix.program_id)
                        .context("invalid simulated inner program ID")?
                }
                UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix)) => {
                    for address in &ix.accounts {
                        let key =
                            Pubkey::from_str(address).context("invalid simulated inner account")?;
                        if !keys.contains(&key) {
                            bail!("simulated inner account is not in transaction message");
                        }
                    }
                    Pubkey::from_str(&ix.program_id)
                        .context("invalid simulated inner program ID")?
                }
            };
            if !keys.contains(&program) {
                bail!("simulated inner program is not in transaction message");
            }
            let program = program.to_string();
            if !programs.contains(&program) {
                programs.push(program);
            }
        }
    }
    Ok((count, programs))
}

fn validate_aggregator_lookup_table(
    table: &AddressLookupTable<'_>,
    observed_slot: u64,
    writable_indexes: &[u8],
    readonly_indexes: &[u8],
) -> anyhow::Result<()> {
    if table.meta.deactivation_slot != u64::MAX {
        bail!("aggregator lookup table is deactivating or deactivated");
    }
    if table.meta.last_extended_slot > observed_slot
        || usize::from(table.meta.last_extended_slot_start_index) > table.addresses.len()
    {
        bail!("aggregator lookup table has inconsistent extension metadata");
    }
    let active_len = if table.meta.last_extended_slot == observed_slot {
        usize::from(table.meta.last_extended_slot_start_index)
    } else {
        table.addresses.len()
    };
    if writable_indexes
        .iter()
        .chain(readonly_indexes)
        .any(|index| usize::from(*index) >= active_len)
    {
        bail!("aggregator lookup table references a missing or not yet active address");
    }
    Ok(())
}

fn same_slot_deltas(
    baseline_slot: u64,
    simulation_slot: u64,
    payer_before: u64,
    payer_after: Option<u64>,
    token_before: u64,
    token_after: Option<u64>,
) -> (Option<u64>, Option<u64>) {
    if baseline_slot != simulation_slot {
        return (None, None);
    }
    (
        payer_after.and_then(|after| payer_before.checked_sub(after)),
        token_after.and_then(|after| after.checked_sub(token_before)),
    )
}

fn target_account_amount_from_data(
    account_owner: &Pubkey,
    data: &[u8],
    token_program: &Pubkey,
    mint: &Pubkey,
    wallet: &Pubkey,
) -> anyhow::Result<u64> {
    if account_owner != token_program {
        bail!("target ATA has unexpected token program");
    }
    let (account_mint, account_wallet, amount) = if *token_program == spl_token::ID {
        let account = TokenAccount::unpack(data).context("invalid SPL target ATA")?;
        (account.mint, account.owner, account.amount)
    } else if *token_program == spl_token_2022::ID {
        let account = StateWithExtensions::<Token2022Account>::unpack(data)
            .context("invalid Token-2022 target ATA")?;
        (account.base.mint, account.base.owner, account.base.amount)
    } else {
        bail!("unsupported target ATA token program");
    };
    if account_mint.as_ref() != mint.as_ref() || account_wallet.as_ref() != wallet.as_ref() {
        bail!("target ATA has unexpected mint or wallet owner");
    }
    Ok(amount)
}

fn simulated_target_account_amount(
    account: &solana_account_decoder::UiAccount,
    token_program: &Pubkey,
    mint: &Pubkey,
    wallet: &Pubkey,
) -> anyhow::Result<u64> {
    let owner = Pubkey::from_str(&account.owner).context("invalid simulated ATA program owner")?;
    let data = account
        .data
        .decode()
        .context("simulated target ATA has no binary data")?;
    target_account_amount_from_data(&owner, &data, token_program, mint, wallet)
}

fn target_token_amount(
    balances: &[solana_transaction_status_client_types::UiTransactionTokenBalance],
    account_index: u8,
    mint: &Pubkey,
) -> Option<u64> {
    let mut matching = balances.iter().filter(|balance| {
        balance.account_index == account_index && balance.mint == mint.to_string()
    });
    let amount = matching
        .next()?
        .ui_token_amount
        .amount
        .parse::<u64>()
        .ok()?;
    if matching.next().is_some() {
        return None;
    }
    Some(amount)
}

fn sell_balance_and_minimum_verified(
    simulation_ok: bool,
    pre_input: Option<u64>,
    post_input: Option<u64>,
    post_input_account: Option<u64>,
    pre_wsol: Option<u64>,
    post_wsol: Option<u64>,
    post_wsol_account: Option<u64>,
    expected_input_balance: u64,
    input_units: u64,
    minimum_wsol_units: u64,
) -> bool {
    simulation_ok
        && pre_input == Some(expected_input_balance)
        && pre_input
            .zip(post_input)
            .and_then(|(before, after)| before.checked_sub(after))
            == Some(input_units)
        && post_input == post_input_account
        && pre_wsol
            .zip(post_wsol)
            .and_then(|(before, after)| after.checked_sub(before))
            .is_some_and(|credit| credit >= minimum_wsol_units)
        && post_wsol == post_wsol_account
}

fn sdk_two_hop_simulation_result(
    response: solana_client::rpc_response::Response<
        solana_client::rpc_response::RpcSimulateTransactionResult,
    >,
    target_index: usize,
    target_mint: &Pubkey,
    target_token_program: &Pubkey,
    payer: &Pubkey,
    target_min_output_units: u64,
    transaction_bytes: usize,
    transaction_version: &'static str,
) -> anyhow::Result<SdkTwoHopSimulation> {
    let value = response.value;
    let target_index_u8 = u8::try_from(target_index).context("target ATA index exceeds u8")?;
    let before = value
        .pre_token_balances
        .as_ref()
        .and_then(|balances| target_token_amount(balances, target_index_u8, target_mint))
        .or_else(|| {
            (value
                .pre_balances
                .as_ref()
                .and_then(|balances| balances.get(target_index))
                == Some(&0))
            .then_some(0)
        });
    let after = value
        .post_token_balances
        .as_ref()
        .and_then(|balances| target_token_amount(balances, target_index_u8, target_mint));
    let target_token_delta_units = before
        .zip(after)
        .and_then(|(before, after)| after.checked_sub(before));
    let target_token_post_units = value
        .accounts
        .as_ref()
        .and_then(|accounts| accounts.first())
        .and_then(Option::as_ref)
        .map(|account| {
            simulated_target_account_amount(account, target_token_program, target_mint, payer)
        })
        .transpose()?;
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
    let minimum_output_verified = value.err.is_none()
        && target_token_delta_units.is_some_and(|delta| delta >= target_min_output_units)
        && target_token_post_units.is_some_and(|post| after == Some(post));
    Ok(SdkTwoHopSimulation {
        attempted: true,
        rpc_accepted: true,
        ok: value.err.is_none(),
        error: value.err.map(|error| redact_urls(&error.to_string())),
        slot: response.context.slot,
        units_consumed: value.units_consumed,
        fee_lamports: value.fee,
        transaction_bytes,
        transaction_version,
        payer_debit_lamports,
        target_token_post_units,
        target_token_delta_units,
        minimum_output_verified,
    })
}

#[cfg(test)]
mod aggregator_simulation_tests {
    use super::*;
    use base64::Engine;
    use solana_address_lookup_table_interface::state::LookupTableMeta;
    use solana_client::rpc_client::RpcClientConfig;
    use solana_client::rpc_config::RpcSendTransactionConfig;
    use solana_client::rpc_request::RpcRequest;
    use solana_client::rpc_sender::{RpcSender, RpcTransportStats};
    use std::borrow::Cow;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CaptureRpcSender(std::sync::Mutex<Option<serde_json::Value>>);

    struct OfflineSdkSend {
        calls: AtomicUsize,
        params: std::sync::Mutex<Option<serde_json::Value>>,
        delay: Duration,
        signature: Signature,
    }

    struct SharedOfflineSdkSend(Arc<OfflineSdkSend>);

    struct OfflineSdkStatus {
        calls: AtomicUsize,
        requested_signature: std::sync::Mutex<Option<String>>,
        response: serde_json::Value,
    }

    struct SharedOfflineSdkStatus(Arc<OfflineSdkStatus>);

    #[async_trait::async_trait]
    impl RpcSender for SharedOfflineSdkStatus {
        async fn send(
            &self,
            request: RpcRequest,
            params: serde_json::Value,
        ) -> solana_client::client_error::Result<serde_json::Value> {
            assert_eq!(request, RpcRequest::GetSignatureStatuses);
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            *self.0.requested_signature.lock().unwrap() = params[0][0].as_str().map(str::to_string);
            Ok(self.0.response.clone())
        }

        fn get_transport_stats(&self) -> RpcTransportStats {
            RpcTransportStats::default()
        }

        fn url(&self) -> String {
            "offline-sdk-status".to_string()
        }
    }

    #[tokio::test]
    async fn sdk_readback_queries_only_the_reserved_signature_without_resending() {
        let signature = Keypair::new().sign_message(b"reserved sdk message");
        for (response, expected) in [
            (
                serde_json::json!({"context": {"slot": 42}, "value": [null]}),
                None,
            ),
            (
                serde_json::json!({
                    "context": {"slot": 42},
                    "value": [{
                        "slot": 42,
                        "confirmations": 1,
                        "err": null,
                        "confirmationStatus": "confirmed",
                        "status": {"Ok": null}
                    }]
                }),
                Some(true),
            ),
            (
                serde_json::json!({
                    "context": {"slot": 43},
                    "value": [{
                        "slot": 43,
                        "confirmations": 1,
                        "err": {"InstructionError": [0, {"Custom": 1}]},
                        "confirmationStatus": "confirmed",
                        "status": {"Err": {"InstructionError": [0, {"Custom": 1}]}}
                    }]
                }),
                Some(false),
            ),
        ] {
            let sender = Arc::new(OfflineSdkStatus {
                calls: AtomicUsize::new(0),
                requested_signature: std::sync::Mutex::new(None),
                response,
            });
            let rpc = solana_client::nonblocking::rpc_client::RpcClient::new_sender(
                SharedOfflineSdkStatus(sender.clone()),
                RpcClientConfig::with_commitment(CommitmentConfig::processed()),
            );
            assert_eq!(
                observe_sdk_signature_by_original(&rpc, signature, Duration::ZERO)
                    .await
                    .unwrap(),
                expected
            );
            assert_eq!(sender.calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                sender.requested_signature.lock().unwrap().as_deref(),
                Some(signature.to_string().as_str())
            );
        }
    }

    #[async_trait::async_trait]
    impl RpcSender for SharedOfflineSdkSend {
        async fn send(
            &self,
            request: RpcRequest,
            params: serde_json::Value,
        ) -> solana_client::client_error::Result<serde_json::Value> {
            assert_eq!(request, RpcRequest::SendTransaction);
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            *self.0.params.lock().unwrap() = Some(params);
            tokio::time::sleep(self.0.delay).await;
            Ok(serde_json::json!(self.0.signature.to_string()))
        }

        fn get_transport_stats(&self) -> RpcTransportStats {
            RpcTransportStats::default()
        }

        fn url(&self) -> String {
            "offline-sdk-send".to_string()
        }
    }

    #[tokio::test]
    async fn prepared_sdk_send_uses_one_rpc_request_and_preserves_local_signature_on_timeout() {
        let payer = Keypair::new();
        let instruction = solana_sdk::instruction::Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[],
            vec![solana_sdk::instruction::AccountMeta::new(
                payer.pubkey(),
                true,
            )],
        );
        let prepared = compile_sdk_two_hop_unsigned_transaction(
            &[instruction],
            &payer.pubkey(),
            Hash::new_unique(),
        )
        .unwrap();
        let transaction = prepared.into_signed_transaction(&payer).unwrap();
        let signature = transaction.signatures[0];
        for (delay, deadline, succeeds) in [
            (Duration::ZERO, Duration::from_secs(1), true),
            (Duration::from_millis(100), Duration::from_millis(5), false),
        ] {
            let sender = Arc::new(OfflineSdkSend {
                calls: AtomicUsize::new(0),
                params: std::sync::Mutex::new(None),
                delay,
                signature,
            });
            let rpc = solana_client::nonblocking::rpc_client::RpcClient::new_sender(
                SharedOfflineSdkSend(sender.clone()),
                RpcClientConfig::with_commitment(CommitmentConfig::processed()),
            );
            let result =
                send_signed_sdk_transaction_once(&rpc, payer.pubkey(), &transaction, 42, deadline)
                    .await;
            assert_eq!(result.is_ok(), succeeds);
            assert_eq!(sender.calls.load(Ordering::SeqCst), 1);
            let params = sender.params.lock().unwrap().clone().unwrap();
            assert_eq!(params[1]["maxRetries"], 0);
            assert_eq!(params[1]["minContextSlot"], 42);
            let wire = base64::engine::general_purpose::STANDARD
                .decode(params[0].as_str().unwrap())
                .unwrap();
            assert_eq!(wire, wincode::serialize(&transaction).unwrap());
            // A transport timeout cannot erase the signature held by the caller.
            assert_eq!(transaction.signatures[0], signature);
        }
        let wrong_wallet = Keypair::new();
        let sender = Arc::new(OfflineSdkSend {
            calls: AtomicUsize::new(0),
            params: std::sync::Mutex::new(None),
            delay: Duration::ZERO,
            signature,
        });
        let rpc = solana_client::nonblocking::rpc_client::RpcClient::new_sender(
            SharedOfflineSdkSend(sender.clone()),
            RpcClientConfig::with_commitment(CommitmentConfig::processed()),
        );
        assert!(send_signed_sdk_transaction_once(
            &rpc,
            wrong_wallet.pubkey(),
            &transaction,
            42,
            Duration::from_secs(1)
        )
        .await
        .is_err());
        assert_eq!(sender.calls.load(Ordering::SeqCst), 0);
    }

    #[async_trait::async_trait]
    impl RpcSender for CaptureRpcSender {
        async fn send(
            &self,
            request: RpcRequest,
            params: serde_json::Value,
        ) -> solana_client::client_error::Result<serde_json::Value> {
            assert_eq!(request, RpcRequest::SendTransaction);
            *self.0.lock().unwrap() = Some(params.clone());
            let wire = params[0].as_str().unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(wire)
                .unwrap();
            let transaction: VersionedTransaction = wincode::deserialize(&bytes).unwrap();
            Ok(serde_json::json!(transaction.signatures[0].to_string()))
        }

        fn get_transport_stats(&self) -> RpcTransportStats {
            RpcTransportStats::default()
        }

        fn url(&self) -> String {
            "offline-capture".to_string()
        }
    }

    #[tokio::test]
    async fn sdk_v1_send_encoder_matches_rpc_wire_without_network() {
        let payer = Arc::new(Keypair::new());
        let instruction = solana_sdk::instruction::Instruction {
            program_id: Pubkey::new_unique(),
            accounts: Vec::new(),
            data: vec![0; 1_300],
        };
        let transaction = sol_trade_sdk::trading::common::build_transaction_with_version(
            &payer,
            200_000,
            1,
            sol_trade_sdk::common::TradeTransactionVersion::V1,
            &[instruction],
            &[],
            Some(Hash::new_unique()),
            None,
            "v1-offline-test",
            true,
            false,
            &payer.pubkey(),
            0.0,
            None,
        )
        .unwrap();
        transaction.sanitize().unwrap();
        transaction.verify_and_hash_message().unwrap();
        let expected_wire = wincode::serialize(&transaction).unwrap();
        assert!(expected_wire.len() > 1_232);
        assert!(expected_wire.len() <= solana_sdk::message::v1::MAX_TRANSACTION_SIZE);
        let (encoded, signature) = sol_trade_sdk::swqos::common::serialize_transaction_and_encode(
            &transaction,
            UiTransactionEncoding::Base64,
        )
        .unwrap();
        let encoded_wire = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(encoded_wire, expected_wire);
        assert_eq!(signature, transaction.signatures[0]);

        let sender = Arc::new(CaptureRpcSender(std::sync::Mutex::new(None)));
        struct SharedSender(Arc<CaptureRpcSender>);
        #[async_trait::async_trait]
        impl RpcSender for SharedSender {
            async fn send(
                &self,
                request: RpcRequest,
                params: serde_json::Value,
            ) -> solana_client::client_error::Result<serde_json::Value> {
                self.0.send(request, params).await
            }

            fn get_transport_stats(&self) -> RpcTransportStats {
                self.0.get_transport_stats()
            }

            fn url(&self) -> String {
                self.0.url()
            }
        }
        let rpc = solana_client::nonblocking::rpc_client::RpcClient::new_sender(
            SharedSender(sender.clone()),
            RpcClientConfig::with_commitment(CommitmentConfig::processed()),
        );
        let returned_signature = rpc
            .send_transaction_with_config(
                &transaction,
                RpcSendTransactionConfig {
                    encoding: Some(UiTransactionEncoding::Base64),
                    ..RpcSendTransactionConfig::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(returned_signature, signature);
        let captured = sender.0.lock().unwrap().clone().unwrap();
        assert_eq!(
            captured[0].as_str().unwrap(),
            base64::engine::general_purpose::STANDARD.encode(expected_wire)
        );
    }

    #[test]
    fn lookup_table_rejects_same_slot_extensions_and_stale_metadata() {
        let addresses = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let mut table = AddressLookupTable {
            meta: LookupTableMeta {
                last_extended_slot: 42,
                last_extended_slot_start_index: 1,
                ..LookupTableMeta::default()
            },
            addresses: Cow::Owned(addresses),
        };
        assert!(validate_aggregator_lookup_table(&table, 42, &[0], &[]).is_ok());
        assert!(validate_aggregator_lookup_table(&table, 42, &[1], &[]).is_err());
        assert!(validate_aggregator_lookup_table(&table, 42, &[], &[1]).is_err());
        assert!(validate_aggregator_lookup_table(&table, 43, &[1], &[]).is_ok());
        assert!(validate_aggregator_lookup_table(&table, 43, &[2], &[]).is_err());
        assert!(validate_aggregator_lookup_table(&table, 41, &[0], &[]).is_err());

        table.meta.last_extended_slot_start_index = 3;
        assert!(validate_aggregator_lookup_table(&table, 43, &[0], &[]).is_err());
        table.meta.last_extended_slot_start_index = 1;
        table.meta.deactivation_slot = 40;
        assert!(validate_aggregator_lookup_table(&table, 43, &[0], &[]).is_err());
    }

    #[test]
    fn target_ata_rejects_close_and_outgoing_token_instructions() {
        let target = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let transfer = [vec![3], 1u64.to_le_bytes().to_vec()].concat();
        for program in [spl_token::ID, spl_token_2022::ID] {
            assert!(
                check_target_token_instruction(&program, &[9], &[target, other], &target).is_err()
            );
            assert!(check_target_token_instruction(
                &program,
                &transfer,
                &[target, other, other],
                &target
            )
            .is_err());
            assert!(check_target_token_instruction(
                &program,
                &transfer,
                &[other, target, other],
                &target
            )
            .is_ok());
            assert!(check_target_token_instruction(&program, &[22], &[target], &target).is_ok());
        }
    }

    #[test]
    fn inner_system_spend_counts_payer_and_marks_unknown_instructions() {
        let payer = Pubkey::new_unique();
        let wsol_ata = Pubkey::new_unique();
        let attacker = Pubkey::new_unique();
        let keys = [Pubkey::default(), payer, wsol_ata, attacker];
        let data = [vec![2, 0, 0, 0], 123u64.to_le_bytes().to_vec()].concat();
        let instruction = solana_transaction_status_client_types::UiCompiledInstruction {
            program_id_index: 0,
            accounts: vec![1, 2],
            data: bs58::encode(data).into_string(),
            stack_height: Some(2),
        };
        let group = UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Compiled(instruction.clone())],
        };
        assert_eq!(
            inner_payer_system_spend(&[group], &keys, &payer, &wsol_ata).unwrap(),
            Some(123)
        );
        let mut hidden_tip = instruction.clone();
        hidden_tip.accounts[1] = 3;
        assert!(inner_payer_system_spend(
            &[UiInnerInstructions {
                index: 0,
                instructions: vec![UiInstruction::Compiled(hidden_tip)],
            }],
            &keys,
            &payer,
            &wsol_ata,
        )
        .unwrap_err()
        .to_string()
        .contains("unexpected recipient"));
        let unknown = UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Compiled(
                solana_transaction_status_client_types::UiCompiledInstruction {
                    data: bs58::encode([0u8; 4]).into_string(),
                    ..instruction.clone()
                },
            )],
        };
        assert_eq!(
            inner_payer_system_spend(&[unknown.clone()], &keys, &payer, &wsol_ata).unwrap(),
            None
        );
        let mut hidden_tip = instruction;
        hidden_tip.accounts[1] = 3;
        let unknown_then_tip = UiInnerInstructions {
            index: 0,
            instructions: vec![
                unknown.instructions[0].clone(),
                UiInstruction::Compiled(hidden_tip),
            ],
        };
        assert!(inner_payer_system_spend(&[unknown_then_tip], &keys, &payer, &wsol_ata).is_err());
    }

    #[test]
    fn parsed_inner_system_transfer_rejects_hidden_tip() {
        let payer = Pubkey::new_unique();
        let wsol_ata = Pubkey::new_unique();
        let attacker = Pubkey::new_unique();
        let keys = [Pubkey::default(), payer, wsol_ata, attacker];
        let system_program = Pubkey::default().to_string();
        let data = [vec![2, 0, 0, 0], 1u64.to_le_bytes().to_vec()].concat();
        let partial = UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(
            solana_transaction_status_client_types::UiPartiallyDecodedInstruction {
                program_id: system_program.clone(),
                accounts: vec![payer.to_string(), attacker.to_string()],
                data: bs58::encode(data).into_string(),
                stack_height: Some(2),
            },
        ));
        let parsed = UiInstruction::Parsed(UiParsedInstruction::Parsed(
            solana_transaction_status_client_types::ParsedInstruction {
                program: "system".into(),
                program_id: system_program,
                parsed: serde_json::json!({
                    "type": "transfer",
                    "info": {
                        "source": payer.to_string(),
                        "destination": attacker.to_string(),
                        "lamports": 1
                    }
                }),
                stack_height: Some(2),
            },
        ));
        let mut safe_partial = partial.clone();
        if let UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix)) = &mut safe_partial
        {
            ix.accounts[1] = wsol_ata.to_string();
        }
        let mut safe_parsed = parsed.clone();
        if let UiInstruction::Parsed(UiParsedInstruction::Parsed(ix)) = &mut safe_parsed {
            ix.parsed["info"]["destination"] = wsol_ata.to_string().into();
        }
        for instruction in [safe_partial, safe_parsed] {
            assert_eq!(
                inner_payer_system_spend(
                    &[UiInnerInstructions {
                        index: 0,
                        instructions: vec![instruction],
                    }],
                    &keys,
                    &payer,
                    &wsol_ata,
                )
                .unwrap(),
                Some(1)
            );
        }
        for instruction in [partial, parsed] {
            assert!(inner_payer_system_spend(
                &[UiInnerInstructions {
                    index: 0,
                    instructions: vec![instruction],
                }],
                &keys,
                &payer,
                &wsol_ata,
            )
            .unwrap_err()
            .to_string()
            .contains("unexpected recipient"));
        }
    }

    #[test]
    fn inner_inventory_rejects_invalid_indices() {
        let program = Pubkey::new_unique();
        let account = Pubkey::new_unique();
        let instruction = solana_transaction_status_client_types::UiCompiledInstruction {
            program_id_index: 0,
            accounts: vec![1],
            data: String::new(),
            stack_height: Some(2),
        };
        let group = UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Compiled(instruction.clone())],
        };
        assert_eq!(
            audit_inner_instruction_inventory(&[group.clone()], &[program, account], 1).unwrap(),
            (1, vec![program.to_string()])
        );
        assert!(
            audit_inner_instruction_inventory(&[group.clone()], &[program, account], 0).is_err()
        );
        let invalid = UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Compiled(
                solana_transaction_status_client_types::UiCompiledInstruction {
                    accounts: vec![2],
                    ..instruction
                },
            )],
        };
        assert!(audit_inner_instruction_inventory(&[invalid], &[program, account], 1).is_err());
    }

    #[test]
    fn balance_deltas_require_matching_slots() {
        assert_eq!(
            same_slot_deltas(42, 42, 100, Some(80), 5, Some(12)),
            (Some(20), Some(7))
        );
        assert_eq!(
            same_slot_deltas(42, 43, 100, Some(80), 5, Some(12)),
            (None, None)
        );
        assert_eq!(
            same_slot_deltas(42, 42, 100, Some(120), 5, Some(12)),
            (None, Some(7))
        );
    }

    #[test]
    fn simulated_target_account_requires_matching_mint_owner_and_program() {
        let mint = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let mut data = vec![0u8; 165];
        data[..32].copy_from_slice(mint.as_ref());
        data[32..64].copy_from_slice(wallet.as_ref());
        data[64..72].copy_from_slice(&42u64.to_le_bytes());
        data[108] = 1;
        assert_eq!(
            target_account_amount_from_data(&spl_token::ID, &data, &spl_token::ID, &mint, &wallet)
                .unwrap(),
            42
        );
        assert_eq!(
            target_account_amount_from_data(
                &spl_token_2022::ID,
                &data,
                &spl_token_2022::ID,
                &mint,
                &wallet
            )
            .unwrap(),
            42
        );
        let mut uninitialized = data.clone();
        uninitialized[108] = 0;
        assert!(target_account_amount_from_data(
            &spl_token::ID,
            &uninitialized,
            &spl_token::ID,
            &mint,
            &wallet
        )
        .is_err());
        assert!(target_account_amount_from_data(
            &spl_token_2022::ID,
            &uninitialized,
            &spl_token_2022::ID,
            &mint,
            &wallet
        )
        .is_err());
        let mut malformed = data.clone();
        malformed[72] = 9;
        assert!(target_account_amount_from_data(
            &spl_token::ID,
            &malformed,
            &spl_token::ID,
            &mint,
            &wallet
        )
        .is_err());
        assert!(target_account_amount_from_data(
            &spl_token_2022::ID,
            &malformed,
            &spl_token_2022::ID,
            &mint,
            &wallet
        )
        .is_err());
        let account: solana_account_decoder::UiAccount =
            serde_json::from_value(serde_json::json!({
                "lamports": 2039280,
                "data": [base64::engine::general_purpose::STANDARD.encode(data), "base64"],
                "owner": spl_token::ID.to_string(),
                "executable": false,
                "rentEpoch": 0
            }))
            .unwrap();
        assert_eq!(
            simulated_target_account_amount(&account, &spl_token::ID, &mint, &wallet).unwrap(),
            42
        );
        assert!(
            simulated_target_account_amount(&account, &spl_token_2022::ID, &mint, &wallet).is_err()
        );
        assert!(simulated_target_account_amount(
            &account,
            &spl_token::ID,
            &Pubkey::new_unique(),
            &wallet
        )
        .is_err());
        assert!(simulated_target_account_amount(
            &account,
            &spl_token::ID,
            &mint,
            &Pubkey::new_unique()
        )
        .is_err());
    }

    #[test]
    fn target_token_amount_requires_unique_matching_index_and_mint() {
        let mint = Pubkey::new_unique();
        let entry = serde_json::json!({
            "accountIndex": 3,
            "mint": mint.to_string(),
            "uiTokenAmount": { "amount": "123", "decimals": 6, "uiAmount": 0.000123, "uiAmountString": "0.000123" }
        });
        let balance: solana_transaction_status_client_types::UiTransactionTokenBalance =
            serde_json::from_value(entry).unwrap();
        assert_eq!(target_token_amount(&[balance.clone()], 3, &mint), Some(123));
        assert_eq!(target_token_amount(&[balance.clone()], 2, &mint), None);
        assert_eq!(
            target_token_amount(&[balance.clone(), balance], 3, &mint),
            None
        );
    }
}

impl SdkTradeClient {
    pub fn conversion_signer(&self) -> Arc<Keypair> {
        self.lanes[0].client.payer.clone()
    }

    pub async fn send_aggregator_transaction(
        &self,
        transaction: &VersionedTransaction,
    ) -> anyhow::Result<Signature> {
        let payer = self.conversion_signer();
        let expected_payer = transaction
            .message
            .static_account_keys()
            .first()
            .context("aggregator transaction has no payer")?;
        if *expected_payer != payer.pubkey() {
            bail!("aggregator transaction payer does not match wallet");
        }
        if transaction.message.header().num_required_signatures != 1
            || transaction.signatures.len() != 1
            || transaction.signatures[0] == Signature::default()
        {
            bail!("aggregator transaction must contain exactly one wallet signature");
        }
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no default RPC lane available for aggregator submission")?;
        tokio::time::timeout(
            Duration::from_secs(12),
            lane.client.get_rpc().send_transaction(transaction),
        )
        .await
        .context("aggregator transaction submission timed out")?
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))
    }

    /// Submit the previously signed SDK message through exactly one Default
    /// RPC request. The caller must durably reserve its local signature first.
    pub(crate) async fn send_prepared_sdk_transaction_once(
        &self,
        transaction: &VersionedTransaction,
        minimum_context_slot: u64,
    ) -> anyhow::Result<Signature> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for prepared SDK submission")?;
        send_signed_sdk_transaction_once(
            lane.client.get_rpc(),
            self.wallet(),
            transaction,
            minimum_context_slot,
            Duration::from_secs(12),
        )
        .await
    }

    /// Read only the original signature after the one permitted send attempt.
    /// `None` means its landing status remains unknown; this never resubmits.
    pub(crate) async fn observe_prepared_sdk_signature(
        &self,
        signature: Signature,
        verify_ms: u64,
    ) -> anyhow::Result<Option<bool>> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for SDK signature readback")?;
        observe_sdk_signature_by_original(
            lane.client.get_rpc(),
            signature,
            Duration::from_millis(verify_ms.max(250)),
        )
        .await
    }

    pub async fn aggregator_target_ata(&self, mint: &Pubkey) -> anyhow::Result<(Pubkey, Pubkey)> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no default RPC lane available for aggregator preflight")?;
        let account = tokio::time::timeout(
            Duration::from_secs(5),
            lane.client.get_rpc().get_account(mint),
        )
        .await
        .context("target mint RPC lookup timed out")?
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
        if account.owner != spl_token::ID && account.owner != spl_token_2022::ID {
            bail!("aggregator target mint is not owned by a supported token program");
        }
        let ata =
            get_associated_token_address_with_program_id_fast(&self.wallet(), mint, &account.owner);
        Ok((ata, account.owner))
    }

    pub async fn resolve_aggregator_lookup_table(
        &self,
        address: &Pubkey,
        writable_indexes: &[u8],
        readonly_indexes: &[u8],
    ) -> anyhow::Result<Vec<Pubkey>> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no default RPC lane available for aggregator preflight")?;
        let account = tokio::time::timeout(
            Duration::from_secs(5),
            lane.client
                .get_rpc()
                .get_account_with_commitment(address, CommitmentConfig::processed()),
        )
        .await
        .context("aggregator lookup table RPC timed out")?
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
        let account_context_slot = account.context.slot;
        let account = account
            .value
            .context("aggregator lookup table is missing")?;
        if account.owner != lookup_program::id() {
            bail!("aggregator lookup table has invalid owner");
        }
        let table = AddressLookupTable::deserialize(&account.data)
            .map_err(|_| anyhow!("invalid aggregator lookup table data"))?;
        validate_aggregator_lookup_table(
            &table,
            account_context_slot,
            writable_indexes,
            readonly_indexes,
        )?;
        Ok(table.addresses.into_owned())
    }

    pub async fn simulate_unsigned_aggregator_transaction(
        &self,
        transaction: &VersionedTransaction,
        target_account_index: u8,
        target_mint: &Pubkey,
        target_ata: &Pubkey,
        target_token_program: &Pubkey,
        resolved_keys: &[Pubkey],
    ) -> anyhow::Result<AggregatorSimulation> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no default RPC lane available for aggregator preflight")?;
        let baseline = tokio::time::timeout(
            Duration::from_secs(5),
            lane.client.get_rpc().get_multiple_accounts_with_commitment(
                &[self.wallet(), *target_ata],
                CommitmentConfig {
                    commitment: solana_commitment_config::CommitmentLevel::Processed,
                },
            ),
        )
        .await
        .context("aggregator balance baseline timed out")?
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
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
                    &self.wallet(),
                )
            })
            .transpose()?
            .unwrap_or(0);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            lane.client.get_rpc().simulate_transaction_with_config(
                transaction,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: false,
                    commitment: Some(CommitmentConfig {
                        commitment: solana_commitment_config::CommitmentLevel::Processed,
                    }),
                    encoding: Some(UiTransactionEncoding::Base64),
                    accounts: Some(RpcSimulateTransactionAccountsConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        addresses: vec![self.wallet().to_string(), target_ata.to_string()],
                    }),
                    min_context_slot: Some(baseline.context.slot),
                    inner_instructions: true,
                },
            ),
        )
        .await
        .context("local aggregator simulation timed out")?
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
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
        let wsol_mint = Pubkey::from_str(WSOL_MINT).context("invalid WSOL mint")?;
        let wsol_ata = get_associated_token_address_with_program_id_fast(
            &self.wallet(),
            &wsol_mint,
            &spl_token::ID,
        );
        let inner_payer_system_spend_lamports = value
            .inner_instructions
            .as_ref()
            .map(|groups| {
                inner_payer_system_spend(groups, resolved_keys, &self.wallet(), &wsol_ata)
            })
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
        let target_token_delta_units = value
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
                simulated_target_account_amount(
                    account,
                    target_token_program,
                    target_mint,
                    &self.wallet(),
                )
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
            baseline_payer_lamports: baseline_payer.lamports,
            simulated_pre_payer_lamports: value
                .pre_balances
                .as_ref()
                .and_then(|balances| balances.first())
                .copied(),
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

    pub async fn resolve_wsol_quote_cpmm_hop(&self, quote_mint: &Pubkey) -> anyhow::Result<Pubkey> {
        // Serialize discovery so repeated panel opens do not multiply expensive GPA calls.
        let mut cache = self.sol_hop_cache.lock().await;
        if let Some((pool_id, expires_at)) = cache.get(quote_mint) {
            if *expires_at > Instant::now() {
                return Ok(*pool_id);
            }
        }
        let wsol = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
        if quote_mint == &wsol {
            bail!("quote mint is already WSOL");
        }
        let mut last_error = None;
        let mut complete_empty_scans = 0;
        for lane in self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
        {
            let started = Instant::now();
            let rpc = lane.client.get_rpc();
            let found = tokio::time::timeout(Duration::from_secs(5), async {
                use sol_trade_sdk::instruction::utils::raydium_cpmm::{accounts, fetch_pool_state};
                use sol_trade_sdk::instruction::utils::raydium_cpmm_types::POOL_STATE_DISCRIMINATOR;
                for (quote_offset, wsol_offset) in [(168usize, 200usize), (200, 168)] {
                    let config = RpcProgramAccountsConfig {
                        filters: Some(vec![
                            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                0,
                                &POOL_STATE_DISCRIMINATOR,
                            )),
                            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                quote_offset,
                                quote_mint.as_ref(),
                            )),
                            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                wsol_offset,
                                wsol.as_ref(),
                            )),
                        ]),
                        account_config: RpcAccountInfoConfig {
                            encoding: Some(UiAccountEncoding::Base64),
                            data_slice: None,
                            commitment: None,
                            min_context_slot: None,
                        },
                        with_context: None,
                        sort_results: None,
                    };
                    #[allow(deprecated)]
                    let pools = rpc
                        .get_program_ui_accounts_with_config(&accounts::RAYDIUM_CPMM, config)
                        .await?;
                    for (pool_id, _) in pools {
                        let state = fetch_pool_state(rpc, &pool_id).await?;
                        if [state.token0_mint, state.token1_mint].contains(&wsol)
                            && [state.token0_mint, state.token1_mint].contains(quote_mint)
                        {
                            let params =
                                RaydiumCpmmParams::from_pool_address_by_rpc(rpc, &pool_id).await?;
                            let mints = [params.base_mint, params.quote_mint];
                            ensure_pool_contains_quote("WSOL/Quote CPMM hop", quote_mint, &mints)?;
                            ensure_pool_contains_quote("WSOL/Quote CPMM hop", &wsol, &mints)?;
                            let (base_balance, quote_balance) = tokio::try_join!(
                                rpc.get_token_account_balance(&params.base_vault),
                                rpc.get_token_account_balance(&params.quote_vault),
                            )?;
                            if base_balance.amount.parse::<u64>()? == 0
                                || quote_balance.amount.parse::<u64>()? == 0
                            {
                                continue;
                            }
                            return Ok::<Option<Pubkey>, anyhow::Error>(Some(pool_id));
                        }
                    }
                }
                Ok::<Option<Pubkey>, anyhow::Error>(None)
            })
            .await;
            match found {
                Ok(Ok(Some(pool_id))) => {
                    cache.insert(
                        *quote_mint,
                        (pool_id, Instant::now() + Duration::from_secs(30)),
                    );
                    return Ok(pool_id);
                }
                Ok(Ok(None)) => {
                    complete_empty_scans += 1;
                    info!(
                        elapsed_ms = started.elapsed().as_millis(),
                        "SOL hop RPC scan found no CPMM pool"
                    );
                    if complete_empty_scans >= 2 {
                        bail!("no WSOL/Quote Raydium CPMM pool found on two RPC lanes");
                    }
                }
                Ok(Err(error)) => {
                    last_error = Some(redact_urls(&error.to_string()));
                    warn!(
                        elapsed_ms = started.elapsed().as_millis(),
                        "SOL hop RPC lane failed"
                    );
                }
                Err(_) => {
                    last_error = Some("SOL hop discovery timed out".to_string());
                    warn!(
                        elapsed_ms = started.elapsed().as_millis(),
                        "SOL hop RPC lane timed out"
                    );
                }
            }
        }
        bail!(
            "no verified WSOL/Quote Raydium CPMM pool: {}",
            last_error.unwrap_or_else(|| "no Default RPC lane configured".to_string())
        )
    }
    pub async fn new(
        rpc_urls: Vec<String>,
        private_key_base58: String,
        mev: MevConfig,
        commitment: CommitmentConfig,
        enable_sdk_log: bool,
    ) -> anyhow::Result<Self> {
        if rpc_urls.is_empty() {
            bail!("at least one RPC URL is required");
        }
        let rpc_count = rpc_urls.len();
        let mut lanes = Vec::new();
        let payer = Arc::new(
            Keypair::try_from_base58_string(&private_key_base58)
                .context("PRIVATE_KEY_BASE58 must be a valid base58 Solana keypair secret")?,
        );
        let swqos_configs = build_swqos_configs(&mev)?;
        if mev.require_mev_submit && swqos_configs.is_empty() {
            bail!("REQUIRE_MEV_SUBMIT=true but no MEV/SWQoS provider is enabled");
        }
        let mut swqos_lanes = swqos_configs
            .iter()
            .map(|config| config.swqos_type().as_str().to_string())
            .collect::<Vec<_>>();
        swqos_lanes.push("Default".to_string());
        for (rpc_index, rpc_url) in rpc_urls.into_iter().enumerate() {
            let is_helius = is_helius_rpc(&rpc_url);
            let use_mev_on_rpc = rpc_index == 0 || mev.use_backup_rpcs;
            if use_mev_on_rpc {
                for swqos_config in &swqos_configs {
                    let swqos_type = swqos_config.swqos_type();
                    let trade_config = TradeConfig::builder(
                        rpc_url.clone(),
                        vec![swqos_config.clone()],
                        commitment.clone(),
                    )
                    .create_wsol_ata_on_startup(false)
                    .use_seed_optimize(false)
                    .log_enabled(enable_sdk_log)
                    .mev_protection(mev.enable_mev_protection)
                    .build();
                    let client = TradingClient::new(payer.clone(), trade_config).await;
                    lanes.push(SdkLane {
                        swqos_type,
                        is_helius,
                        client,
                    });
                }
            }

            let trade_config = TradeConfig::builder(
                rpc_url.clone(),
                vec![SwqosConfig::Default(rpc_url.clone())],
                commitment.clone(),
            )
            .create_wsol_ata_on_startup(false)
            .use_seed_optimize(false)
            .log_enabled(enable_sdk_log)
            .mev_protection(mev.enable_mev_protection)
            .build();
            let client = TradingClient::new(payer.clone(), trade_config).await;
            lanes.push(SdkLane {
                swqos_type: SwqosType::Default,
                is_helius,
                client,
            });
        }

        if mev.require_mev_submit
            && !lanes
                .iter()
                .any(|lane| lane.swqos_type != SwqosType::Default)
        {
            bail!("REQUIRE_MEV_SUBMIT=true but no MEV/SWQoS execution lane is available");
        }

        Ok(Self {
            lanes,
            sol_hop_cache: Mutex::new(HashMap::new()),
            swqos_lanes,
            mev_protection: mev.enable_mev_protection,
            require_mev_submit: mev.require_mev_submit,
            rpc_count,
        })
    }

    pub fn wallet(&self) -> Pubkey {
        self.lanes[0].client.payer.pubkey()
    }

    pub fn rpcs(&self) -> Vec<Arc<sol_trade_sdk::common::SolanaRpcClient>> {
        self.lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
            .map(|lane| lane.client.get_rpc().clone())
            .collect()
    }

    pub fn rpc_count(&self) -> usize {
        self.rpc_count
    }

    pub fn swqos_lanes(&self) -> Vec<String> {
        self.swqos_lanes.clone()
    }

    pub fn active_swqos_lanes(&self) -> Vec<String> {
        let mut lanes =
            if self.require_mev_submit && self.swqos_lanes.iter().any(|lane| lane != "Default") {
                self.swqos_lanes
                    .iter()
                    .filter(|lane| lane.as_str() != "Default")
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                self.swqos_lanes.clone()
            };
        lanes.dedup();
        lanes
    }

    pub fn mev_protection(&self) -> bool {
        self.mev_protection
    }

    pub fn require_mev_submit(&self) -> bool {
        self.require_mev_submit
    }

    pub async fn ping_rpc(&self) -> bool {
        for lane in self.execution_lanes() {
            if lane.client.get_rpc().get_latest_blockhash().await.is_ok() {
                return true;
            }
        }
        false
    }

    pub async fn wallet_balance(&self) -> anyhow::Result<u64> {
        let wallet = self.wallet();
        let mut last_error = None;
        for lane in self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
        {
            match tokio::time::timeout(
                Duration::from_secs(4),
                lane.client.get_rpc().get_balance(&wallet),
            )
            .await
            {
                Ok(Ok(balance)) => return Ok(balance),
                Ok(Err(error)) => last_error = Some(redact_urls(&error.to_string())),
                Err(_) => last_error = Some("RPC balance query timed out".to_string()),
            }
        }
        bail!(
            "wallet balance unavailable: {}",
            last_error.unwrap_or_else(|| "no Default RPC lane configured".to_string())
        )
    }

    pub async fn quote_balance(&self, mint: &Pubkey) -> anyhow::Result<(u8, u64, u64)> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured")?;
        let rpc = lane.client.get_rpc();
        let wallet = self.wallet();
        let supply = rpc.get_token_supply(mint).await?;
        let balance = self.token_balance(&lane.client, mint).await?;
        let sol_balance = rpc.get_balance(&wallet).await?;
        Ok((supply.decimals, balance, sol_balance))
    }

    pub async fn wallet_balances(&self, wallets: &[Pubkey]) -> anyhow::Result<Vec<u64>> {
        let mut last_error = None;
        for lane in self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
        {
            match tokio::time::timeout(
                Duration::from_secs(4),
                lane.client.get_rpc().get_multiple_accounts(wallets),
            )
            .await
            {
                Ok(Ok(accounts)) => {
                    return Ok(accounts
                        .into_iter()
                        .map(|account| account.map(|value| value.lamports).unwrap_or(0))
                        .collect())
                }
                Ok(Err(error)) => last_error = Some(redact_urls(&error.to_string())),
                Err(_) => last_error = Some("RPC balance query timed out".to_string()),
            }
        }
        bail!(
            "wallet balances unavailable: {}",
            last_error.unwrap_or_else(|| "no Default RPC lane configured".to_string())
        )
    }

    pub async fn transfer_sol(
        &self,
        destination: &Pubkey,
        fixed_lamports: Option<u64>,
        percent: Option<u64>,
    ) -> anyhow::Result<SolTransferResult> {
        let payer = self
            .lanes
            .first()
            .context("wallet has no execution lane")?
            .client
            .payer
            .clone();
        if payer.pubkey() == *destination {
            bail!("source and destination wallets must be different");
        }

        let mut prepared = None;
        let mut last_error = None;
        for lane in self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
        {
            let rpc = lane.client.get_rpc().clone();
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let balance = rpc.get_balance(&payer.pubkey()).await?;
                let blockhash = rpc.get_latest_blockhash().await?;
                let estimate_instruction =
                    system_instruction::transfer(&payer.pubkey(), destination, 1);
                let mut message = Message::new(&[estimate_instruction], Some(&payer.pubkey()));
                message.recent_blockhash = blockhash;
                let fee = rpc.get_fee_for_message(&message).await?;
                let rent_exempt_minimum = rpc.get_minimum_balance_for_rent_exemption(0).await?;
                Ok::<_, solana_client::client_error::ClientError>((
                    rpc,
                    balance,
                    blockhash,
                    fee,
                    rent_exempt_minimum,
                ))
            })
            .await;
            match result {
                Ok(Ok(value)) => {
                    prepared = Some(value);
                    break;
                }
                Ok(Err(error)) => last_error = Some(redact_urls(&error.to_string())),
                Err(_) => last_error = Some("RPC transfer preparation timed out".to_string()),
            }
        }
        let (rpc, balance, blockhash, fee, rent_exempt_minimum) = prepared.with_context(|| {
            format!(
                "unable to prepare SOL transfer: {}",
                last_error.unwrap_or_else(|| "no Default RPC lane configured".to_string())
            )
        })?;
        let (amount, reserve) = calculate_sol_transfer_amount(
            balance,
            fee,
            rent_exempt_minimum,
            fixed_lamports,
            percent,
        )?;

        let instruction = system_instruction::transfer(&payer.pubkey(), destination, amount);
        let transaction = Transaction::new_signed_with_payer(
            &[instruction],
            Some(&payer.pubkey()),
            &[payer.as_ref()],
            blockhash,
        );
        let signature = tokio::time::timeout(
            Duration::from_secs(12),
            rpc.send_transaction(&transaction),
        )
        .await
        .context("SOL transfer submission timed out")?
        .with_context(|| {
            format!(
                "SOL transfer submission failed: balance={balance}, amount={amount}, fee={fee}, reserve={reserve} lamports"
            )
        })?;
        let status = match tokio::time::timeout(
            Duration::from_secs(15),
            rpc.confirm_transaction(&signature),
        )
        .await
        {
            Ok(Ok(true)) => "confirmed",
            _ => "submitted",
        };
        Ok(SolTransferResult {
            status: status.to_string(),
            signature: signature.to_string(),
            amount_lamports: amount,
            fee_lamports: fee,
            balance_before_lamports: balance,
        })
    }

    pub async fn pumpapi_trade(&self, trade: PumpApiTrade) -> anyhow::Result<ExecuteResult> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for PumpAPI execution")?;
        pumpapi_client::execute(&lane.client, trade).await
    }

    pub async fn warmup_params(
        &self,
        pool: Pool,
        mint: &Pubkey,
        quote_mint: &Pubkey,
        pool_id: Option<&Pubkey>,
    ) -> anyhow::Result<()> {
        let mut last_error = None;
        for lane in self.execution_lanes() {
            let result: anyhow::Result<()> = async {
                match pool {
                    Pool::Pump => {
                        let params =
                            PumpFunParams::from_mint_by_rpc(lane.client.get_rpc(), mint).await?;
                        if params.bonding_curve.complete {
                            bail!("pump bonding curve is complete");
                        }
                        Ok(())
                    }
                    Pool::PumpAmm => {
                        let params = if let Some(pool_id) = pool_id {
                            PumpSwapParams::from_pool_address_by_rpc(lane.client.get_rpc(), pool_id)
                                .await?
                        } else {
                            PumpSwapParams::from_mint_by_rpc(lane.client.get_rpc(), mint).await?
                        };
                        ensure_pool_contains_quote(
                            "pump-amm",
                            quote_mint,
                            &[params.base_mint, params.quote_mint],
                        )?;
                        Ok(())
                    }
                    Pool::Bonk => BonkParams::from_mint_by_rpc(
                        lane.client.get_rpc(),
                        mint,
                        is_usd1_quote(quote_mint),
                    )
                    .await
                    .map(|_| ()),
                    Pool::StonkFun => {
                        let pool_id = pool_id.context("pool_id is required for stonkfun")?;
                        load_stonkfun_params(lane.client.get_rpc(), pool_id, mint, quote_mint)
                            .await?;
                        Ok(())
                    }
                    Pool::StonkFunSwap => {
                        let pool_id = pool_id.context("pool_id is required for stonkfun-swap")?;
                        let params = RaydiumCpmmParams::from_pool_address_by_rpc(
                            lane.client.get_rpc(),
                            pool_id,
                        )
                        .await?;
                        ensure_pool_contains_mint(
                            "stonkfun-swap",
                            mint,
                            &[params.base_mint, params.quote_mint],
                        )?;
                        ensure_pool_contains_quote(
                            "stonkfun-swap",
                            quote_mint,
                            &[params.base_mint, params.quote_mint],
                        )?;
                        Ok(())
                    }
                    Pool::MeteoraDammV2 => {
                        let pool_id = pool_id.context("pool_id is required for meteora-damm-v2")?;
                        let params = MeteoraDammV2Params::from_pool_address_by_rpc(
                            lane.client.get_rpc(),
                            pool_id,
                        )
                        .await?;
                        ensure_pool_contains_quote(
                            "meteora-damm-v2",
                            quote_mint,
                            &[params.token_a_mint, params.token_b_mint],
                        )?;
                        Ok(())
                    }
                    Pool::MeteoraDbc => {
                        bail!("meteora-dbc requires the legacy DBC backend")
                    }
                    Pool::RaydiumAmmV4 => {
                        let pool_id = pool_id.context("pool_id is required for raydium-amm-v4")?;
                        let params = RaydiumAmmV4Params::from_amm_address_by_rpc(
                            lane.client.get_rpc(),
                            *pool_id,
                        )
                        .await?;
                        ensure_pool_contains_quote(
                            "raydium-amm-v4",
                            quote_mint,
                            &[params.coin_mint, params.pc_mint],
                        )?;
                        Ok(())
                    }
                    Pool::RaydiumCpmm => {
                        let pool_id = pool_id.context("pool_id is required for raydium-cpmm")?;
                        let params = RaydiumCpmmParams::from_pool_address_by_rpc(
                            lane.client.get_rpc(),
                            pool_id,
                        )
                        .await?;
                        ensure_pool_contains_quote(
                            "raydium-cpmm",
                            quote_mint,
                            &[params.base_mint, params.quote_mint],
                        )?;
                        Ok(())
                    }
                }
            }
            .await;
            match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    let error = redact_urls(&error.to_string());
                    last_error = Some(anyhow!("{}: {}", lane.swqos_type.as_str(), error));
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("no RPC lanes configured")))
    }

    pub async fn buy(&self, trade: ExecuteTrade) -> anyhow::Result<ExecuteResult> {
        ensure_direct_sdk_buildability(
            trade.pool,
            Side::Buy,
            &trade.quote_mint,
            trade.pool_id.as_ref(),
            trade.sol_hop_pool,
            trade.sol_hop_pool_id.as_ref(),
        )?;
        let mut last_error = None;
        let mut last_submitted = None;
        let mut fallback_notes = Vec::new();
        let lanes = self.trade_execution_lanes(&trade)?;
        // A live SDK call may have sent a transaction even if it returns an
        // error without a visible signature. Never rebuild this order on a
        // second lane; simulated requests can still try another lane.
        for lane in lanes
            .into_iter()
            .take(if trade.simulate { usize::MAX } else { 1 })
        {
            match self.buy_on_lane(lane, trade.clone()).await {
                Ok(mut result) if result.success => {
                    if trade.simulate {
                        info!(
                            swqos_lane = lane.swqos_type.as_str(),
                            "buy lane simulation succeeded"
                        );
                    } else {
                        info!(
                            swqos_lane = lane.swqos_type.as_str(),
                            signature = ?result.signature,
                            "buy lane submitted successfully"
                        );
                    }
                    if let Some(verified) = self
                        .verify_lane_submission(lane, &mut result, &trade)
                        .await?
                    {
                        if verified {
                            attach_fallback_notes(&mut result, &fallback_notes);
                            return Ok(result);
                        }
                        remember_fallback_note(lane, result.error.as_deref(), &mut fallback_notes);
                        last_submitted = Some(result);
                        continue;
                    }
                    attach_fallback_notes(&mut result, &fallback_notes);
                    return Ok(result);
                }
                Ok(result) => {
                    let error_text = result.error.as_deref().map(redact_urls);
                    warn!(
                        swqos_lane = lane.swqos_type.as_str(),
                        signature = ?result.signature,
                        error = ?error_text,
                        "buy lane returned non-success"
                    );
                    if result.signature.is_some() {
                        if is_pumpswap_buy_slippage_error(result.error.as_deref()) {
                            return Ok(result);
                        }
                        remember_fallback_note(lane, result.error.as_deref(), &mut fallback_notes);
                        last_submitted = Some(result);
                        continue;
                    }
                    let error_text = result.error.unwrap_or_else(|| "buy failed".to_string());
                    remember_fallback_note(lane, Some(&error_text), &mut fallback_notes);
                    last_error = Some(anyhow!("{}: {}", lane.swqos_type.as_str(), error_text))
                }
                Err(error) => {
                    let error_text = redact_urls(&error.to_string());
                    warn!(
                        swqos_lane = lane.swqos_type.as_str(),
                        error = %error_text,
                        "buy lane failed"
                    );
                    remember_fallback_note(lane, Some(&error_text), &mut fallback_notes);
                    last_error = Some(anyhow!("{}: {}", lane.swqos_type.as_str(), error_text));
                }
            }
        }
        if let Some(mut result) = last_submitted {
            attach_fallback_notes(&mut result, &fallback_notes);
            return Ok(result);
        }
        Err(last_error.unwrap_or_else(|| anyhow!("no RPC lanes configured")))
    }

    async fn buy_on_lane(
        &self,
        lane: &SdkLane,
        trade: ExecuteTrade,
    ) -> anyhow::Result<ExecuteResult> {
        let params = self
            .dex_params(
                &lane.client,
                trade.pool,
                &trade.mint,
                &trade.quote_mint,
                trade.pool_id.as_ref(),
                trade.sol_hop_pool,
                trade.sol_hop_pool_id.as_ref(),
            )
            .await?;
        let dex_type = dex_type(trade.pool)?;
        let input_token_type = if trade.sol_hop_pool_id.is_some() {
            TradeTokenType::SOL
        } else {
            trade_token_type_for_buy(trade.pool, &trade.quote_mint)?
        };
        let amount = trade.amount_lamports.context("amount_lamports missing")?;
        if matches!(
            trade.pool,
            Pool::PumpAmm | Pool::StonkFun | Pool::StonkFunSwap | Pool::RaydiumCpmm
        ) && !is_wsol_quote(&trade.quote_mint)
            && trade.sol_hop_pool_id.is_none()
        {
            let balance = self.token_balance(&lane.client, &trade.quote_mint).await?;
            if balance < amount {
                bail!("quote token balance insufficient: available {balance} units, requested {amount} units");
            }
        }
        let fixed_output_token_amount = match trade.pool {
            Pool::MeteoraDammV2 => Some(1),
            _ => None,
        };
        let mint_account_exists = self.token_account_exists(&lane.client, &trade.mint).await?;
        let buy_params = TradeBuyParams {
            dex_type,
            input_token_type,
            mint: trade.mint,
            input_token_amount: amount,
            slippage_basis_points: Some(trade.slippage_bps),
            recent_blockhash: Some(trade.recent_blockhash),
            extension_params: params,
            address_lookup_table_accounts: Vec::new(),
            wait_tx_confirmed: should_wait_lane_confirmation(
                lane.swqos_type,
                trade.wait_tx_confirmed,
            ),
            wait_for_all_submits: false,
            create_input_token_ata: true,
            close_input_token_ata: false,
            create_mint_ata: !mint_account_exists,
            durable_nonce: None,
            fixed_output_token_amount,
            gas_fee_strategy: default_fee_strategy(lane.swqos_type, trade.tip_sol),
            simulate: trade.simulate,
            use_exact_sol_amount: Some(true),
            grpc_recv_us: None,
        };
        map_sdk_result(lane.client.buy(buy_params).await, trade.simulate)
    }

    pub async fn sell(&self, trade: ExecuteTrade) -> anyhow::Result<ExecuteResult> {
        ensure_direct_sdk_buildability(
            trade.pool,
            Side::Sell,
            &trade.quote_mint,
            trade.pool_id.as_ref(),
            trade.sol_hop_pool,
            trade.sol_hop_pool_id.as_ref(),
        )?;
        let mut last_error = None;
        let mut last_submitted = None;
        let mut fallback_notes = Vec::new();
        let lanes = self.trade_execution_lanes(&trade)?;
        // A live SDK call may have sent a transaction even if it returns an
        // error without a visible signature. Never rebuild this order on a
        // second lane; simulated requests can still try another lane.
        for lane in lanes
            .into_iter()
            .take(if trade.simulate { usize::MAX } else { 1 })
        {
            match self.sell_on_lane(lane, trade.clone()).await {
                Ok(mut result) if result.success => {
                    if trade.simulate {
                        info!(
                            swqos_lane = lane.swqos_type.as_str(),
                            "sell lane simulation succeeded"
                        );
                    } else {
                        info!(
                            swqos_lane = lane.swqos_type.as_str(),
                            signature = ?result.signature,
                            "sell lane submitted successfully"
                        );
                    }
                    if let Some(verified) = self
                        .verify_lane_submission(lane, &mut result, &trade)
                        .await?
                    {
                        if verified {
                            attach_fallback_notes(&mut result, &fallback_notes);
                            return Ok(result);
                        }
                        remember_fallback_note(lane, result.error.as_deref(), &mut fallback_notes);
                        last_submitted = Some(result);
                        continue;
                    }
                    attach_fallback_notes(&mut result, &fallback_notes);
                    return Ok(result);
                }
                Ok(result) => {
                    let error_text = result.error.as_deref().map(redact_urls);
                    warn!(
                        swqos_lane = lane.swqos_type.as_str(),
                        signature = ?result.signature,
                        error = ?error_text,
                        "sell lane returned non-success"
                    );
                    if result.signature.is_some() {
                        remember_fallback_note(lane, result.error.as_deref(), &mut fallback_notes);
                        last_submitted = Some(result);
                        continue;
                    }
                    let error_text = result.error.unwrap_or_else(|| "sell failed".to_string());
                    remember_fallback_note(lane, Some(&error_text), &mut fallback_notes);
                    last_error = Some(anyhow!("{}: {}", lane.swqos_type.as_str(), error_text))
                }
                Err(error) => {
                    let error_text = redact_urls(&error.to_string());
                    warn!(
                        swqos_lane = lane.swqos_type.as_str(),
                        error = %error_text,
                        "sell lane failed"
                    );
                    remember_fallback_note(lane, Some(&error_text), &mut fallback_notes);
                    last_error = Some(anyhow!("{}: {}", lane.swqos_type.as_str(), error_text));
                }
            }
        }
        if let Some(mut result) = last_submitted {
            attach_fallback_notes(&mut result, &fallback_notes);
            return Ok(result);
        }
        Err(last_error.unwrap_or_else(|| anyhow!("no RPC lanes configured")))
    }

    async fn sell_on_lane(
        &self,
        lane: &SdkLane,
        trade: ExecuteTrade,
    ) -> anyhow::Result<ExecuteResult> {
        let percent = trade.sell_percent.context("sell_percent missing")?;
        let balance = self.token_balance(&lane.client, &trade.mint).await?;
        if balance == 0 {
            bail!("token balance is zero");
        }
        let mut params = self
            .dex_params(
                &lane.client,
                trade.pool,
                &trade.mint,
                &trade.quote_mint,
                trade.pool_id.as_ref(),
                trade.sol_hop_pool,
                trade.sol_hop_pool_id.as_ref(),
            )
            .await?;
        let fixed_output_token_amount = if trade.pool == Pool::MeteoraDammV2 {
            let DexParamEnum::MeteoraDammV2(ref mut damm) = params else {
                bail!("DAMM v2 protocol params missing");
            };
            // Exact-in prevents a partial-fill from satisfying a full-size quote with less input.
            damm.swap_mode = 0;
            let input_amount = balance
                .checked_mul(percent)
                .context("sell amount overflow")?
                / 100;
            if input_amount == 0 {
                bail!("sell input amount rounds to zero");
            }
            Some(
                self.simulate_damm_v2_sell_output(
                    lane,
                    damm,
                    trade.mint,
                    trade.quote_mint,
                    input_amount,
                    trade.recent_blockhash,
                )
                .await
                .and_then(|quoted| minimum_output(quoted, trade.slippage_bps))?,
            )
        } else {
            None
        };
        let sell_params = TradeSellParams {
            dex_type: dex_type(trade.pool)?,
            output_token_type: if trade.sol_hop_pool_id.is_some() {
                TradeTokenType::SOL
            } else {
                trade_token_type_for_sell(trade.pool, &trade.quote_mint)?
            },
            mint: trade.mint,
            input_token_amount: 0,
            slippage_basis_points: Some(trade.slippage_bps),
            recent_blockhash: Some(trade.recent_blockhash),
            with_tip: true,
            extension_params: params,
            address_lookup_table_accounts: Vec::new(),
            wait_tx_confirmed: should_wait_lane_confirmation(
                lane.swqos_type,
                trade.wait_tx_confirmed,
            ),
            wait_for_all_submits: false,
            create_output_token_ata: true,
            close_output_token_ata: false,
            close_mint_token_ata: percent == 100,
            durable_nonce: None,
            fixed_output_token_amount,
            gas_fee_strategy: if trade.simulate && trade.pool == Pool::MeteoraDammV2 {
                simulation_fee_strategy()
            } else {
                default_fee_strategy(lane.swqos_type, trade.tip_sol)
            },
            simulate: trade.simulate,
            grpc_recv_us: None,
        };
        if trade.pool == Pool::MeteoraDammV2 && !trade.simulate {
            let mut preflight = sell_params.clone();
            preflight.simulate = true;
            preflight.gas_fee_strategy = simulation_fee_strategy();
            let checked = map_sdk_result(
                lane.client
                    .sell_by_percent(preflight, balance, percent)
                    .await,
                true,
            )?;
            if !checked.success {
                bail!(
                    "DAMM v2 final transaction simulation failed: {}",
                    checked.error.unwrap_or_else(|| "unknown error".to_string())
                );
            }
        }
        map_sdk_result(
            lane.client
                .sell_by_percent(sell_params, balance, percent)
                .await,
            trade.simulate,
        )
    }

    async fn simulate_damm_v2_sell_output(
        &self,
        lane: &SdkLane,
        damm: &MeteoraDammV2Params,
        mint: Pubkey,
        quote_mint: Pubkey,
        input_amount: u64,
        recent_blockhash: Hash,
    ) -> anyhow::Result<u64> {
        let token_program = if quote_mint == damm.token_a_mint {
            damm.token_a_program
        } else if quote_mint == damm.token_b_mint {
            damm.token_b_program
        } else {
            bail!("DAMM v2 quote mint is not in the pool");
        };
        let owner = lane.client.payer.pubkey();
        let output_ata =
            get_associated_token_address_with_program_id_fast(&owner, &quote_mint, &token_program);
        let rpc = lane.client.get_rpc();
        let baseline = rpc
            .get_account_with_commitment(&output_ata, CommitmentConfig::processed())
            .await?;
        let before = baseline
            .value
            .as_ref()
            .map(|account| {
                target_account_amount_from_data(
                    &account.owner,
                    &account.data,
                    &token_program,
                    &quote_mint,
                    &owner,
                )
            })
            .transpose()?
            .unwrap_or(0);
        let params = SwapParams {
            rpc: Some(rpc.clone()),
            payer: lane.client.payer.clone(),
            trade_type: TradeType::Sell,
            input_mint: mint,
            input_token_program: None,
            output_mint: quote_mint,
            output_token_program: Some(token_program),
            input_amount: Some(input_amount),
            slippage_basis_points: None,
            address_lookup_table_accounts: Vec::new(),
            recent_blockhash: Some(recent_blockhash),
            wait_tx_confirmed: false,
            protocol_params: DexParamEnum::MeteoraDammV2(damm.clone()),
            open_seed_optimize: lane.client.use_seed_optimize,
            swqos_clients: lane.client.infrastructure.swqos_clients.clone(),
            middleware_manager: None,
            durable_nonce: None,
            with_tip: false,
            create_input_mint_ata: false,
            close_input_mint_ata: false,
            create_output_mint_ata: true,
            close_output_mint_ata: false,
            fixed_output_amount: Some(1),
            gas_fee_strategy: default_fee_strategy(lane.swqos_type, 0.0),
            simulate: true,
            log_enabled: false,
            wait_for_all_submits: false,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: lane.client.max_sender_concurrency,
            effective_core_ids: lane.client.effective_core_ids.clone(),
            check_min_tip: false,
            transaction_version: lane.client.transaction_version,
            grpc_recv_us: None,
            use_exact_sol_amount: None,
        };
        let instructions = MeteoraDammV2InstructionBuilder
            .build_sell_instructions(&params)
            .await?;
        let transaction = Transaction::new_signed_with_payer(
            &instructions,
            Some(&owner),
            &[lane.client.payer.as_ref()],
            recent_blockhash,
        );
        let result = rpc
            .simulate_transaction_with_config(
                &transaction,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: false,
                    commitment: Some(CommitmentConfig::processed()),
                    encoding: Some(UiTransactionEncoding::Base64),
                    accounts: Some(RpcSimulateTransactionAccountsConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        addresses: vec![output_ata.to_string()],
                    }),
                    min_context_slot: Some(baseline.context.slot),
                    inner_instructions: false,
                },
            )
            .await?;
        if let Some(error) = result.value.err {
            bail!("DAMM v2 quote simulation failed: {error:?}");
        }
        let after_account = result
            .value
            .accounts
            .as_ref()
            .and_then(|accounts| accounts.first())
            .and_then(Option::as_ref)
            .context("DAMM v2 simulation did not return output ATA")?;
        let after =
            simulated_target_account_amount(after_account, &token_program, &quote_mint, &owner)?;
        after
            .checked_sub(before)
            .filter(|amount| *amount > 0)
            .context("DAMM v2 simulation returned no positive output")
    }

    async fn dex_params(
        &self,
        client: &TradingClient,
        pool: Pool,
        mint: &Pubkey,
        quote_mint: &Pubkey,
        pool_id: Option<&Pubkey>,
        sol_hop_pool: Option<Pool>,
        sol_hop_pool_id: Option<&Pubkey>,
    ) -> anyhow::Result<DexParamEnum> {
        Ok(match pool {
            Pool::Pump => DexParamEnum::PumpFun(
                PumpFunParams::from_mint_by_rpc(client.get_rpc(), mint).await?,
            ),
            Pool::PumpAmm => {
                let params = if let Some(pool_id) = pool_id {
                    PumpSwapParams::from_pool_address_by_rpc(client.get_rpc(), pool_id).await?
                } else {
                    PumpSwapParams::from_mint_by_rpc(client.get_rpc(), mint).await?
                };
                ensure_pool_contains_mint(
                    "pump-amm",
                    mint,
                    &[params.base_mint, params.quote_mint],
                )?;
                ensure_pool_contains_quote(
                    "pump-amm",
                    quote_mint,
                    &[params.base_mint, params.quote_mint],
                )?;
                DexParamEnum::PumpSwap(params)
            }
            Pool::Bonk => DexParamEnum::Bonk(
                BonkParams::from_mint_by_rpc(client.get_rpc(), mint, is_usd1_quote(quote_mint))
                    .await
                    .map_err(|error| describe_bonk_param_error(mint, quote_mint, error))?,
            ),
            Pool::StonkFun => {
                let pool_id = pool_id.context("pool_id is required for stonkfun")?;
                let params =
                    load_stonkfun_params(client.get_rpc(), pool_id, mint, quote_mint).await?;
                if let Some(sol_hop_pool_id) = sol_hop_pool_id {
                    let hop =
                        load_stonkfun_sol_hop(client, sol_hop_pool, sol_hop_pool_id, quote_mint)
                            .await?;
                    DexParamEnum::StonkFunViaSol(StonkFunViaSolParams::curve_with_cpmm(params, hop))
                } else {
                    DexParamEnum::StonkFun(params)
                }
            }
            Pool::StonkFunSwap => {
                let pool_id = pool_id.context("pool_id is required for stonkfun-swap")?;
                let params =
                    RaydiumCpmmParams::from_pool_address_by_rpc(client.get_rpc(), pool_id).await?;
                ensure_pool_contains_mint(
                    "stonkfun-swap",
                    mint,
                    &[params.base_mint, params.quote_mint],
                )?;
                ensure_pool_contains_quote(
                    "stonkfun-swap",
                    quote_mint,
                    &[params.base_mint, params.quote_mint],
                )?;
                if let Some(sol_hop_pool_id) = sol_hop_pool_id {
                    let hop =
                        load_stonkfun_sol_hop(client, sol_hop_pool, sol_hop_pool_id, quote_mint)
                            .await?;
                    DexParamEnum::StonkFunViaSol(StonkFunViaSolParams::graduated_with_cpmm(
                        params, hop,
                    ))
                } else {
                    DexParamEnum::StonkFunSwap(params)
                }
            }
            Pool::MeteoraDammV2 => {
                let pool_id = pool_id.context("pool_id is required for meteora-damm-v2")?;
                let params =
                    MeteoraDammV2Params::from_pool_address_by_rpc(client.get_rpc(), pool_id)
                        .await?
                        .with_rate_limiter_sysvar(true);
                ensure_pool_contains_mint(
                    "meteora-damm-v2",
                    mint,
                    &[params.token_a_mint, params.token_b_mint],
                )?;
                ensure_pool_contains_quote(
                    "meteora-damm-v2",
                    quote_mint,
                    &[params.token_a_mint, params.token_b_mint],
                )?;
                DexParamEnum::MeteoraDammV2(params)
            }
            Pool::MeteoraDbc => {
                bail!("meteora-dbc requires the legacy DBC backend")
            }
            Pool::RaydiumAmmV4 => {
                let pool_id = pool_id.context("pool_id is required for raydium-amm-v4")?;
                let params =
                    RaydiumAmmV4Params::from_amm_address_by_rpc(client.get_rpc(), *pool_id).await?;
                ensure_pool_contains_mint(
                    "raydium-amm-v4",
                    mint,
                    &[params.coin_mint, params.pc_mint],
                )?;
                ensure_pool_contains_quote(
                    "raydium-amm-v4",
                    quote_mint,
                    &[params.coin_mint, params.pc_mint],
                )?;
                DexParamEnum::RaydiumAmmV4(params)
            }
            Pool::RaydiumCpmm => {
                let pool_id = pool_id.context("pool_id is required for raydium-cpmm")?;
                let params =
                    RaydiumCpmmParams::from_pool_address_by_rpc(client.get_rpc(), pool_id).await?;
                ensure_pool_contains_mint(
                    "raydium-cpmm",
                    mint,
                    &[params.base_mint, params.quote_mint],
                )?;
                ensure_pool_contains_quote(
                    "raydium-cpmm",
                    quote_mint,
                    &[params.base_mint, params.quote_mint],
                )?;
                DexParamEnum::RaydiumCpmm(params)
            }
        })
    }

    async fn token_balance(&self, client: &TradingClient, mint: &Pubkey) -> anyhow::Result<u64> {
        let accounts = match client
            .get_rpc()
            .get_token_accounts_by_owner(&self.wallet(), TokenAccountsFilter::Mint(*mint))
            .await
        {
            Ok(accounts) => accounts,
            Err(error) => {
                warn!(
                    mint = %mint,
                    error = %redact_urls(&error.to_string()),
                    "token account scan failed, trying ATA balance fallback"
                );
                return self.token_balance_from_atas(client, mint).await;
            }
        };
        if accounts.is_empty() {
            return Ok(0);
        }
        let mut total = 0_u64;
        let mut parsed_count = 0_u64;
        for account in accounts {
            if let UiAccountData::Json(parsed) = account.account.data {
                if parsed.program != "spl-token" && parsed.program != "spl-token-2022" {
                    continue;
                }
                parsed_count += 1;
                let amount = parsed
                    .parsed
                    .get("info")
                    .and_then(|info| info.get("tokenAmount"))
                    .and_then(|token_amount| token_amount.get("amount"))
                    .and_then(|amount| amount.as_str())
                    .context("missing token amount in RPC response")?
                    .parse::<u64>()
                    .context("parse token amount")?;
                total = total.saturating_add(amount);
            }
        }
        if parsed_count == 0 {
            self.token_balance_from_atas(client, mint).await
        } else {
            Ok(total)
        }
    }

    async fn token_balance_from_atas(
        &self,
        client: &TradingClient,
        mint: &Pubkey,
    ) -> anyhow::Result<u64> {
        let wallet = self.wallet();
        let mut last_error = None;
        for token_program_id in [spl_token::ID, spl_token_2022::ID] {
            let ata =
                get_associated_token_address_with_program_id_fast(&wallet, mint, &token_program_id);
            match client.get_rpc().get_token_account_balance(&ata).await {
                Ok(balance) => {
                    let amount = balance
                        .amount
                        .parse::<u64>()
                        .context("parse ATA token amount")?;
                    if amount > 0 {
                        return Ok(amount);
                    }
                }
                Err(error) => {
                    last_error = Some(anyhow!("{}", redact_urls(&error.to_string())));
                }
            }
        }
        if let Some(error) = last_error {
            return Err(error.into());
        }
        Ok(0)
    }

    async fn token_account_exists(
        &self,
        client: &TradingClient,
        mint: &Pubkey,
    ) -> anyhow::Result<bool> {
        let accounts = client
            .get_rpc()
            .get_token_accounts_by_owner(&self.wallet(), TokenAccountsFilter::Mint(*mint))
            .await?;
        Ok(accounts
            .into_iter()
            .any(|account| match account.account.data {
                UiAccountData::Json(parsed) => {
                    parsed.program == "spl-token" || parsed.program == "spl-token-2022"
                }
                _ => false,
            }))
    }

    async fn verify_lane_submission(
        &self,
        lane: &SdkLane,
        result: &mut ExecuteResult,
        trade: &ExecuteTrade,
    ) -> anyhow::Result<Option<bool>> {
        if lane.swqos_type == SwqosType::Default
            || trade.wait_tx_confirmed
            || trade.simulate
            || trade.mev_submit_verify_ms == 0
        {
            return Ok(None);
        }

        let Some(signature_text) = result.signature.as_deref() else {
            return Ok(None);
        };
        let signature = match Signature::from_str(signature_text) {
            Ok(signature) => signature,
            Err(_) => {
                result.success = false;
                result.error =
                    Some("submitted signature is invalid; reconcile order status".into());
                return Ok(Some(false));
            }
        };
        let deadline = Instant::now() + Duration::from_millis(trade.mev_submit_verify_ms);

        loop {
            let statuses = match lane
                .client
                .get_rpc()
                .get_signature_statuses_with_history(&[signature])
                .await
            {
                Ok(statuses) => statuses,
                Err(error) => {
                    result.success = false;
                    result.error = Some(format!(
                        "{} signature status unavailable: {}; reconcile order status",
                        lane.swqos_type.as_str(),
                        redact_urls(&error.to_string())
                    ));
                    return Ok(Some(false));
                }
            };
            if let Some(status) = statuses.value.into_iter().next().flatten() {
                if let Some(error) = status.err {
                    result.success = false;
                    result.error = Some(format!(
                        "{} landed with transaction error: {error:?}",
                        lane.swqos_type.as_str()
                    ));
                    warn!(
                        swqos_lane = lane.swqos_type.as_str(),
                        signature = %signature,
                        error = ?error,
                        "lane signature landed with error"
                    );
                }
                return Ok(Some(true));
            }

            if Instant::now() >= deadline {
                result.success = false;
                result.error = Some(format!(
                    "{} submit signature not visible after {}ms; reconcile order status",
                    lane.swqos_type.as_str(),
                    trade.mev_submit_verify_ms
                ));
                warn!(
                    swqos_lane = lane.swqos_type.as_str(),
                    signature = %signature,
                    verify_ms = trade.mev_submit_verify_ms,
                    "lane signature not visible; awaiting order reconciliation"
                );
                return Ok(Some(false));
            }

            sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn resolve_pool(&self, mint: &Pubkey) -> anyhow::Result<ResolvePoolResponse> {
        self.resolve_pump_family_pool(mint, None).await
    }

    /// Read the quote mint from a caller-selected PumpSwap pool. The pool ID is
    /// a hint only; the returned route is verified separately against chain
    /// owners and mint accounts before it can enter the route store.
    pub async fn resolve_pumpswap_pool_by_id(
        &self,
        mint: &Pubkey,
        pool_id: &Pubkey,
    ) -> anyhow::Result<ResolvePoolResponse> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for PumpSwap pool resolution")?;
        let params =
            PumpSwapParams::from_pool_address_by_rpc(lane.client.get_rpc(), pool_id).await?;
        anyhow::ensure!(
            params.pool == *pool_id && params.base_mint == *mint,
            "selected PumpSwap pool does not match target mint"
        );
        Ok(ResolvePoolResponse {
            mint: mint.to_string(),
            pool: Pool::PumpAmm,
            pool_id: Some(pool_id.to_string()),
            quote_mint: params.quote_mint.to_string(),
        })
    }

    /// Discover the opposite mint of a selected Raydium CPMM pool using a
    /// current, swap-enabled chain snapshot. The caller's pool ID and mint
    /// remain hints until the backend verifies the returned route again.
    pub async fn resolve_cpmm_pool_by_id(
        &self,
        mint: &Pubkey,
        pool_id: &Pubkey,
    ) -> anyhow::Result<ResolvePoolResponse> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for CPMM pool resolution")?;
        let params = load_open_cpmm_params(lane.client.get_rpc(), pool_id).await?;
        let quote_mint = other_pool_mint(mint, &params.base_mint, &params.quote_mint)?;
        Ok(ResolvePoolResponse {
            mint: mint.to_string(),
            pool: Pool::RaydiumCpmm,
            pool_id: Some(pool_id.to_string()),
            quote_mint: quote_mint.to_string(),
        })
    }

    /// Discover the opposite mint of a selected AMM v4 pool after checking
    /// its owner and open status. Route verification independently rechecks
    /// the mints and token programs before an observation is stored.
    pub async fn resolve_amm_v4_pool_by_id(
        &self,
        mint: &Pubkey,
        pool_id: &Pubkey,
    ) -> anyhow::Result<ResolvePoolResponse> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for AMM v4 pool resolution")?;
        let rpc = lane.client.get_rpc();
        let pool_account = rpc.get_account(pool_id).await?;
        anyhow::ensure!(
            pool_account.owner
                == sol_trade_sdk::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4,
            "AMM v4 pool owner does not match program"
        );
        let state = sol_trade_sdk::instruction::utils::raydium_amm_v4_types::amm_info_decode(
            &pool_account.data,
        )
        .context("AMM v4 pool state is invalid")?;
        let now_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_secs();
        anyhow::ensure!(
            amm_v4_pool_open(state.status, state.out_put.pool_open_time, now_seconds),
            "AMM v4 pool is not open for swaps"
        );
        let params = RaydiumAmmV4Params::from_amm_address_by_rpc(rpc, *pool_id).await?;
        anyhow::ensure!(
            params.amm == *pool_id
                && params.coin_mint == state.coin_mint
                && params.pc_mint == state.pc_mint,
            "AMM v4 pool identity changed while discovering quote mint"
        );
        let quote_mint = other_pool_mint(mint, &params.coin_mint, &params.pc_mint)?;
        Ok(ResolvePoolResponse {
            mint: mint.to_string(),
            pool: Pool::RaydiumAmmV4,
            pool_id: Some(pool_id.to_string()),
            quote_mint: quote_mint.to_string(),
        })
    }

    /// Resolve only the requested Pump-family protocol when a caller selected
    /// one. A mint can have an active curve and a PumpSwap pool at the same
    /// time, so the generic curve-first search cannot satisfy a PumpSwap hint.
    pub async fn resolve_pump_family_pool(
        &self,
        mint: &Pubkey,
        requested_pool: Option<Pool>,
    ) -> anyhow::Result<ResolvePoolResponse> {
        anyhow::ensure!(
            matches!(requested_pool, None | Some(Pool::Pump | Pool::PumpAmm)),
            "mint-only resolver supports Pump and PumpSwap hints only"
        );
        let default_lanes = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
            .collect::<Vec<_>>();
        if default_lanes.is_empty() {
            bail!("no Default RPC lane configured for pool resolution");
        }

        let helius_lanes = default_lanes
            .iter()
            .map(|lane| lane.is_helius)
            .collect::<Vec<_>>();
        let mut random_byte = [0u8; 1];
        if helius_lanes.iter().filter(|&&is_helius| is_helius).count() >= 2 {
            SystemRandom::new()
                .fill(&mut random_byte)
                .map_err(|_| anyhow!("could not choose Helius RPC lane"))?;
        }
        let lane_order = pool_resolution_lane_order(&helius_lanes, random_byte[0] & 1 != 0);
        let mut failures = Vec::with_capacity(default_lanes.len());
        for lane_index in lane_order {
            let lane = default_lanes[lane_index];
            let rpc = lane.client.get_rpc();
            // Keep the total resolver budget usable for the remaining serial fallback lanes.
            let lane_result = tokio::time::timeout(Duration::from_secs(3), async {
                let pump_result = if requested_pool == Some(Pool::PumpAmm) {
                    None
                } else {
                    Some(PumpFunParams::from_mint_by_rpc(rpc, mint).await)
                };
                if let Some(Ok(params)) = &pump_result {
                    if !params.bonding_curve.complete {
                        return Ok(ResolvePoolResponse {
                            mint: mint.to_string(),
                            pool: Pool::Pump,
                            pool_id: None,
                            quote_mint: params.quote_mint.to_string(),
                        });
                    }
                }
                if requested_pool == Some(Pool::Pump) {
                    return Err(match pump_result {
                        Some(Ok(_)) => "requested Pump bonding curve is complete".to_string(),
                        Some(Err(error)) => format!("requested Pump bonding curve unavailable: {error}"),
                        None => unreachable!("Pump hint always probes the curve"),
                    });
                }
                let dbc_error = "DBC backend is disabled in SDK11 candidate";

                let pump_amm_result = match find_pool_by_mint(rpc, mint, DexType::PumpSwap).await {
                    Ok(pool_address) => {
                        PumpSwapParams::from_pool_address_by_rpc(rpc, &pool_address).await
                    }
                    Err(error) => Err(error),
                };
                match pump_amm_result {
                    Ok(params) => Ok(ResolvePoolResponse {
                        mint: mint.to_string(),
                        pool: Pool::PumpAmm,
                        pool_id: Some(params.pool.to_string()),
                        quote_mint: params.quote_mint.to_string(),
                    }),
                    Err(pump_amm_error) => match pump_result {
                        Some(Ok(_)) => Err(format!(
                            "pump bonding curve is complete but migrated pools are unavailable: pump-amm={pump_amm_error}; meteora-dbc={dbc_error}"
                        )),
                        Some(Err(pump_error)) => Err(format!(
                            "no supported pool found for mint; pump={pump_error}; pump-amm={pump_amm_error}; meteora-dbc={dbc_error}"
                        )),
                        None => Err(format!("requested PumpSwap pool unavailable: {pump_amm_error}")),
                    },
                }
            })
            .await;
            let failure = match lane_result {
                Ok(Ok(resolved)) => return Ok(resolved),
                Ok(Err(failure)) => failure,
                Err(_) => "pool resolution timed out".to_string(),
            };
            let redacted = redact_urls(&failure);
            warn!(
                mint = %mint,
                rpc_lane = lane_index + 1,
                error = %redacted,
                "pool resolution RPC lane failed"
            );
            failures.push(pool_resolution_failure_kind(&redacted));
        }

        bail!(
            "pool resolution failed across {} RPC lanes: {}",
            failures.len(),
            failures.join(", ")
        )
    }

    /// Reload the resolved pool and both mint accounts at one RPC snapshot.
    /// SDK parameter constructors perform layout checks; the explicit owner
    /// reads here prevent a route from being accepted solely because an address
    /// was observed in an event or supplied by a caller.
    pub async fn verify_resolved_route(
        &self,
        response: &ResolvePoolResponse,
    ) -> anyhow::Result<VerifiedRouteMetadata> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for route verification")?;
        let rpc = lane.client.get_rpc();
        let mint = Pubkey::from_str(&response.mint).context("resolved mint is invalid")?;
        let quote_mint =
            Pubkey::from_str(&response.quote_mint).context("resolved quote mint is invalid")?;
        let slot = rpc.get_slot().await?;
        let mint_account = rpc.get_account(&mint).await?;
        let supported_token_owner = |owner: Pubkey| {
            owner == sol_trade_sdk::common::spl_token::ID
                || owner == sol_trade_sdk::common::spl_token_2022::ID
        };
        anyhow::ensure!(
            supported_token_owner(mint_account.owner),
            "resolved mint is not owned by SPL Token or Token-2022"
        );

        let native_sol_quote = quote_mint == sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT;
        let (quote_token_program, quote_mint_owner_verified) = if native_sol_quote {
            (sol_trade_sdk::common::spl_token::ID, true)
        } else {
            let account = rpc.get_account(&quote_mint).await?;
            anyhow::ensure!(
                supported_token_owner(account.owner),
                "resolved quote mint is not owned by SPL Token or Token-2022"
            );
            (account.owner, true)
        };

        match response.pool {
            Pool::Pump => {
                let params = PumpFunParams::from_mint_by_rpc(rpc, &mint).await?;
                anyhow::ensure!(
                    params.token_program == mint_account.owner,
                    "pump mint owner does not match decoded token program"
                );
                anyhow::ensure!(
                    !params.bonding_curve.complete,
                    "pump bonding curve is complete"
                );
                anyhow::ensure!(
                    params.quote_mint == quote_mint
                        || (quote_mint == sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT
                            && params.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT),
                    "resolved pump quote mint does not match bonding curve"
                );
                let pool_account = rpc.get_account(&params.bonding_curve.account).await?;
                anyhow::ensure!(
                    pool_account.owner
                        == sol_trade_sdk::instruction::utils::pumpfun::accounts::PUMPFUN,
                    "pump bonding curve owner does not match Pump program"
                );
                let validated_slot = if !native_sol_quote {
                    // Re-read the curve, fee accounts, mints and vaults within
                    // one RPC context slot. This is still an observation, not
                    // a selected fee tier or protected trade quote.
                    let quote_snapshot =
                        crate::pumpfun_quote_snapshot::load_pumpfun_quote_accounts(
                            rpc,
                            mint,
                            quote_mint,
                            mint_account.owner,
                            quote_token_program,
                        )
                        .await?;
                    anyhow::ensure!(
                        quote_snapshot.curve.account == params.bonding_curve.account,
                        "Pump.fun curve identity changed during route verification"
                    );
                    tracing::debug!(
                        slot = quote_snapshot.slot,
                        epoch = quote_snapshot.epoch,
                        global_supply = quote_snapshot.global.token_total_supply,
                        global_protocol_bps = quote_snapshot.global.fallback_protocol_bps,
                        global_creator_bps = quote_snapshot.global.fallback_creator_bps,
                        whitelisted_quote_mint = %quote_snapshot.global.whitelisted_quote_mint,
                        creator_fee_configurable = quote_snapshot.global.creator_fee_configurable,
                        max_creator_bps = quote_snapshot.global.max_configurable_creator_fee_bps,
                        max_curve_depth = ?quote_snapshot.global.max_curve_depth,
                        fee_tiers = quote_snapshot.config.fee_tiers.len(),
                        stable_fee_tiers = quote_snapshot.config.stable_fee_tiers.len(),
                        base_supply = quote_snapshot.base_supply,
                        quote_supply = quote_snapshot.quote_supply,
                        base_vault = quote_snapshot.base_vault_amount,
                        quote_vault = quote_snapshot.quote_vault_amount,
                        base_transfer_fee_bps = quote_snapshot.base_transfer_fee.basis_points,
                        quote_transfer_fee_bps = quote_snapshot.quote_transfer_fee.basis_points,
                        "Pump.fun token-quote account snapshot verified"
                    );
                    quote_snapshot.slot
                } else {
                    slot
                };
                Ok(VerifiedRouteMetadata {
                    program_id: sol_trade_sdk::instruction::utils::pumpfun::accounts::PUMPFUN,
                    pool_id: None,
                    input_mint: Some(quote_mint),
                    output_mint: mint,
                    quote_mint,
                    token_program: params.token_program,
                    quote_token_program,
                    validated_slot,
                    pool_owner_verified: true,
                    mint_owner_verified: true,
                    quote_mint_owner_verified,
                })
            }
            Pool::PumpAmm => {
                let pool_id = response
                    .pool_id
                    .as_deref()
                    .context("PumpSwap resolution did not include pool_id")?
                    .parse::<Pubkey>()
                    .context("PumpSwap pool_id is invalid")?;
                let params = PumpSwapParams::from_pool_address_by_rpc(rpc, &pool_id).await?;
                params.validate_accounts()?;
                anyhow::ensure!(
                    params.base_mint == mint && params.quote_mint == quote_mint,
                    "PumpSwap pool mints do not match resolved route"
                );
                anyhow::ensure!(
                    params.base_token_program == mint_account.owner,
                    "PumpSwap base mint owner does not match pool token program"
                );
                if !native_sol_quote {
                    let quote_account = rpc.get_account(&quote_mint).await?;
                    anyhow::ensure!(
                        params.quote_token_program == quote_account.owner,
                        "PumpSwap quote mint owner does not match pool token program"
                    );
                }
                let pool_account = rpc.get_account(&pool_id).await?;
                anyhow::ensure!(
                    pool_account.owner
                        == sol_trade_sdk::instruction::utils::pumpswap::accounts::AMM_PROGRAM,
                    "PumpSwap pool owner does not match AMM program"
                );
                Ok(VerifiedRouteMetadata {
                    program_id: sol_trade_sdk::instruction::utils::pumpswap::accounts::AMM_PROGRAM,
                    pool_id: Some(pool_id),
                    input_mint: Some(quote_mint),
                    output_mint: mint,
                    quote_mint,
                    token_program: params.base_token_program,
                    quote_token_program: params.quote_token_program,
                    validated_slot: slot,
                    pool_owner_verified: true,
                    mint_owner_verified: true,
                    quote_mint_owner_verified,
                })
            }
            Pool::Bonk => {
                use sol_trade_sdk::instruction::utils::bonk::{
                    accounts, fetch_pool_state, get_pool_pda,
                };

                let pool_id = get_pool_pda(&mint, &quote_mint)
                    .context("could not derive Bonk LaunchLab pool")?;
                let state = fetch_pool_state(rpc, &pool_id).await?;
                validate_bonk_pool_identity(
                    &mint,
                    &quote_mint,
                    &state.base_mint,
                    &state.quote_mint,
                    &state.global_config,
                    &state.platform_config,
                )?;
                let params =
                    BonkParams::from_mint_by_rpc(rpc, &mint, is_usd1_quote(&quote_mint)).await?;
                anyhow::ensure!(
                    params.pool_state == pool_id && params.quote_mint == quote_mint,
                    "Bonk SDK parameters do not match verified pool"
                );
                anyhow::ensure!(
                    mint_account.owner == params.mint_token_program,
                    "Bonk target mint owner does not match SDK token program"
                );
                anyhow::ensure!(
                    quote_token_program == params.quote_token_program,
                    "Bonk quote mint owner does not match SDK token program"
                );
                Ok(VerifiedRouteMetadata {
                    program_id: accounts::BONK,
                    pool_id: Some(pool_id),
                    input_mint: Some(quote_mint),
                    output_mint: mint,
                    quote_mint,
                    token_program: params.mint_token_program,
                    quote_token_program: params.quote_token_program,
                    validated_slot: slot,
                    pool_owner_verified: true,
                    mint_owner_verified: true,
                    quote_mint_owner_verified: true,
                })
            }
            Pool::StonkFun => {
                let pool_id = response
                    .pool_id
                    .as_deref()
                    .context("StonkFun resolution requires pool_id")?
                    .parse::<Pubkey>()
                    .context("StonkFun pool_id is invalid")?;
                let params = load_stonkfun_params(rpc, &pool_id, &mint, &quote_mint).await?;
                anyhow::ensure!(
                    mint_account.owner == params.mint_token_program,
                    "StonkFun target mint owner does not match pool token program"
                );
                anyhow::ensure!(
                    quote_token_program == params.quote_token_program,
                    "StonkFun quote mint owner does not match pool token program"
                );
                Ok(VerifiedRouteMetadata {
                    program_id: sol_trade_sdk::instruction::utils::bonk::accounts::BONK,
                    pool_id: Some(pool_id),
                    input_mint: Some(quote_mint),
                    output_mint: mint,
                    quote_mint,
                    token_program: params.mint_token_program,
                    quote_token_program: params.quote_token_program,
                    validated_slot: slot,
                    pool_owner_verified: true,
                    mint_owner_verified: true,
                    quote_mint_owner_verified: true,
                })
            }
            Pool::MeteoraDammV2 => {
                let pool_id = response
                    .pool_id
                    .as_deref()
                    .context("Meteora DAMM v2 route resolution requires pool_id")?
                    .parse::<Pubkey>()
                    .context("Meteora DAMM v2 pool_id is invalid")?;
                let params = MeteoraDammV2Params::from_pool_address_by_rpc(rpc, &pool_id).await?;
                let (token_program, route_quote_program) = pool_route_token_programs(
                    "Meteora DAMM v2",
                    &mint,
                    &quote_mint,
                    &params.token_a_mint,
                    &params.token_b_mint,
                    &params.token_a_program,
                    &params.token_b_program,
                )?;
                anyhow::ensure!(
                    mint_account.owner == token_program,
                    "Meteora DAMM v2 target mint owner does not match pool token program"
                );
                anyhow::ensure!(
                    quote_token_program == route_quote_program,
                    "Meteora DAMM v2 quote mint owner does not match pool token program"
                );
                Ok(VerifiedRouteMetadata {
                    program_id:
                        sol_trade_sdk::instruction::utils::meteora_damm_v2::accounts::METEORA_DAMM_V2,
                    pool_id: Some(pool_id),
                    input_mint: Some(quote_mint),
                    output_mint: mint,
                    quote_mint,
                    token_program,
                    quote_token_program: route_quote_program,
                    validated_slot: slot,
                    pool_owner_verified: true,
                    mint_owner_verified: true,
                    quote_mint_owner_verified: true,
                })
            }
            Pool::RaydiumAmmV4 => {
                let pool_id = response
                    .pool_id
                    .as_deref()
                    .context("Raydium AMM v4 route resolution requires pool_id")?
                    .parse::<Pubkey>()
                    .context("Raydium AMM v4 pool_id is invalid")?;
                let pool_account = rpc.get_account(&pool_id).await?;
                anyhow::ensure!(
                    pool_account.owner
                        == sol_trade_sdk::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4,
                    "Raydium AMM v4 pool owner does not match program"
                );
                let params = RaydiumAmmV4Params::from_amm_address_by_rpc(rpc, pool_id).await?;
                validate_amm_v4_route_mints(
                    &mint,
                    &quote_mint,
                    &params.coin_mint,
                    &params.pc_mint,
                )?;
                anyhow::ensure!(
                    params.amm == pool_id
                        && mint_account.owner == spl_token::ID
                        && quote_token_program == spl_token::ID,
                    "Raydium AMM v4 requires matching classic SPL Token mints"
                );
                Ok(VerifiedRouteMetadata {
                    program_id:
                        sol_trade_sdk::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4,
                    pool_id: Some(pool_id),
                    input_mint: Some(quote_mint),
                    output_mint: mint,
                    quote_mint,
                    token_program: spl_token::ID,
                    quote_token_program: spl_token::ID,
                    validated_slot: slot,
                    pool_owner_verified: true,
                    mint_owner_verified: true,
                    quote_mint_owner_verified: true,
                })
            }
            Pool::RaydiumCpmm | Pool::StonkFunSwap => {
                let pool_id = response
                    .pool_id
                    .as_deref()
                    .context("Raydium CPMM route resolution requires pool_id")?
                    .parse::<Pubkey>()
                    .context("Raydium CPMM pool_id is invalid")?;
                // The SDK constructor decodes the pool, checks its owner and
                // checks both mint owners against the pool token programs.
                let params = RaydiumCpmmParams::from_pool_address_by_rpc(rpc, &pool_id).await?;
                let (token_program, route_quote_program) = cpmm_route_token_programs(
                    &mint,
                    &quote_mint,
                    &params.base_mint,
                    &params.quote_mint,
                    &params.base_token_program,
                    &params.quote_token_program,
                )?;
                anyhow::ensure!(
                    mint_account.owner == token_program,
                    "Raydium CPMM target mint owner does not match pool token program"
                );
                anyhow::ensure!(
                    quote_token_program == route_quote_program,
                    "Raydium CPMM quote mint owner does not match pool token program"
                );
                Ok(VerifiedRouteMetadata {
                    program_id:
                        sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM,
                    pool_id: Some(pool_id),
                    input_mint: Some(quote_mint),
                    output_mint: mint,
                    quote_mint,
                    token_program,
                    quote_token_program: route_quote_program,
                    validated_slot: slot,
                    pool_owner_verified: true,
                    mint_owner_verified: true,
                    quote_mint_owner_verified: true,
                })
            }
            pool => anyhow::bail!("route verification is not implemented for {pool:?}"),
        }
    }

    /// Quote one exact-input Raydium CPMM leg from a freshly loaded pool
    /// snapshot. The SDK calculator includes protocol/creator/fund fees and
    /// Token-2022 transfer fees; unsupported protocols stay explicit rather
    /// than receiving a fabricated constant-product estimate.
    pub async fn quote_raydium_cpmm(
        &self,
        pool_id: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        amount_in: u64,
    ) -> anyhow::Result<(u64, u64)> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for quote")?;
        let params = load_open_cpmm_params(lane.client.get_rpc(), pool_id).await?;
        let input_mint = if *input_mint == sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT {
            sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        } else {
            *input_mint
        };
        let output_mint = if *output_mint == sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT {
            sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        } else {
            *output_mint
        };
        let is_base_in = if input_mint == params.base_mint && output_mint == params.quote_mint {
            true
        } else if input_mint == params.quote_mint && output_mint == params.base_mint {
            false
        } else {
            anyhow::bail!("Raydium CPMM quote mints do not match pool state")
        };
        let quote = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
            &params, is_base_in, amount_in, 0,
        )?;
        Ok((quote.amount_out, quote.fee))
    }

    /// Read-only exact-input quote for a verified Raydium AMM v4 pool. Use
    /// current on-chain fee and PnL-adjusted reserves from the SDK loader;
    /// this is not a SOL bridge or an executable route decision.
    pub async fn quote_raydium_amm_v4(
        &self,
        pool_id: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        amount_in: u64,
    ) -> anyhow::Result<(u64, u64)> {
        anyhow::ensure!(amount_in > 0, "AMM v4 quote input must be positive");
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for AMM v4 quote")?;
        let rpc = lane.client.get_rpc();
        let pool_account = rpc.get_account(pool_id).await?;
        anyhow::ensure!(
            pool_account.owner
                == sol_trade_sdk::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4,
            "AMM v4 pool owner does not match program"
        );
        let state = sol_trade_sdk::instruction::utils::raydium_amm_v4_types::amm_info_decode(
            &pool_account.data,
        )
        .context("AMM v4 pool state is invalid")?;
        let now_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_secs();
        anyhow::ensure!(
            amm_v4_pool_open(state.status, state.out_put.pool_open_time, now_seconds),
            "AMM v4 pool is not open for swaps"
        );
        let params = RaydiumAmmV4Params::from_amm_address_by_rpc(rpc, *pool_id).await?;
        anyhow::ensure!(
            params.amm == *pool_id
                && params.coin_mint == state.coin_mint
                && params.pc_mint == state.pc_mint
                && params.token_coin == state.token_coin
                && params.token_pc == state.token_pc
                && params.swap_fee_numerator == state.fees.swap_fee_numerator
                && params.swap_fee_denominator == state.fees.swap_fee_denominator,
            "AMM v4 pool identity or fee changed while quoting"
        );
        let is_coin_in =
            amm_v4_quote_direction(input_mint, output_mint, &params.coin_mint, &params.pc_mint)?;
        let quote = sol_trade_sdk::utils::calc::raydium_amm_v4::compute_swap_amount_for_pool(
            &params, is_coin_in, amount_in, 0,
        )?;
        anyhow::ensure!(quote.amount_out > 0, "AMM v4 quote output is zero");
        Ok((quote.amount_out, quote.fee))
    }

    /// Quote and assemble a CPMM -> AMM v4 SOL buy from fresh pool state.
    /// Optionally simulates the unsigned transaction. This never signs,
    /// submits, or authorizes an executable SDK or aggregator decision.
    pub async fn preview_amm_v4_via_cpmm(
        &self,
        target_pool_id: &Pubkey,
        bridge_pool_id: &Pubkey,
        target_mint: &Pubkey,
        quote_mint: &Pubkey,
        input_lamports: u64,
        slippage_bps: u64,
        simulate_transaction: bool,
    ) -> anyhow::Result<SdkTwoHopBuildPreview> {
        anyhow::ensure!(input_lamports > 0, "SDK two-hop input must be positive");
        anyhow::ensure!(slippage_bps < 10_000, "SDK two-hop slippage is invalid");
        anyhow::ensure!(
            target_mint != quote_mint,
            "AMM v4 target and quote mints match"
        );
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for SDK two-hop preview")?;
        let rpc = lane.client.get_rpc();
        let start_slot = rpc.get_slot().await?;
        let (bridge, target) = tokio::try_join!(
            load_open_cpmm_params(rpc, bridge_pool_id),
            RaydiumAmmV4Params::from_amm_address_by_rpc(rpc, *target_pool_id),
        )?;
        let (target_pool_account, target_mint_account, quote_mint_account) = tokio::try_join!(
            rpc.get_account(target_pool_id),
            rpc.get_account(target_mint),
            rpc.get_account(quote_mint),
        )?;
        let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
        anyhow::ensure!(
            bridge.pool_state == *bridge_pool_id
                && ((bridge.base_mint == wsol && bridge.quote_mint == *quote_mint)
                    || (bridge.quote_mint == wsol && bridge.base_mint == *quote_mint)),
            "CPMM two-hop bridge does not match verified pool and mints"
        );
        let bridge_is_base_in = bridge.base_mint == wsol;
        let bridge_output_program = if bridge_is_base_in {
            bridge.quote_token_program
        } else {
            bridge.base_token_program
        };
        let bridge_output_fee = if bridge_is_base_in {
            bridge.quote_transfer_fee
        } else {
            bridge.base_transfer_fee
        };
        anyhow::ensure!(
            bridge_output_program == spl_token::ID
                && bridge_output_fee.basis_points == 0
                && target_mint_account.owner == spl_token::ID
                && quote_mint_account.owner == spl_token::ID,
            "AMM v4 route requires classic SPL Token mints without bridge transfer fees"
        );
        anyhow::ensure!(
            target_pool_account.owner
                == sol_trade_sdk::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4,
            "AMM v4 target pool owner does not match program"
        );
        let state = sol_trade_sdk::instruction::utils::raydium_amm_v4_types::amm_info_decode(
            &target_pool_account.data,
        )
        .context("AMM v4 target pool state is invalid")?;
        let now_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_secs();
        anyhow::ensure!(
            amm_v4_pool_open(state.status, state.out_put.pool_open_time, now_seconds),
            "AMM v4 target pool is not open for swaps"
        );
        anyhow::ensure!(
            target.amm == *target_pool_id
                && target.coin_mint == state.coin_mint
                && target.pc_mint == state.pc_mint
                && target.token_coin == state.token_coin
                && target.token_pc == state.token_pc
                && target.swap_fee_numerator == state.fees.swap_fee_numerator
                && target.swap_fee_denominator == state.fees.swap_fee_denominator,
            "AMM v4 target pool identity or fee changed while quoting"
        );
        let target_is_coin_in =
            amm_v4_quote_direction(quote_mint, target_mint, &target.coin_mint, &target.pc_mint)?;
        let bridge_quote = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
            &bridge,
            bridge_is_base_in,
            input_lamports,
            slippage_bps,
        )?;
        let bridge_min = bridge_quote.min_amount_out;
        anyhow::ensure!(bridge_min > 0, "SDK two-hop bridge minimum is zero");
        let target_quote =
            sol_trade_sdk::utils::calc::raydium_amm_v4::compute_swap_amount_for_pool(
                &target,
                target_is_coin_in,
                bridge_min,
                slippage_bps,
            )?;
        let target_min = target_quote.min_amount_out;
        anyhow::ensure!(target_min > 0, "SDK two-hop target minimum is zero");
        let route = StonkFunQuoteRoute::buy(vec![
            StonkFunQuoteHop::exact_in(
                StonkFunQuoteVenue::RaydiumCpmm(bridge),
                wsol,
                *quote_mint,
                bridge_min,
            )
            .with_output_transfer_fee(bridge_output_fee),
            StonkFunQuoteHop::exact_in(
                StonkFunQuoteVenue::RaydiumAmmV4(target),
                *quote_mint,
                *target_mint,
                target_min,
            ),
        ]);
        let preview = route.preview(
            sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT,
            *target_mint,
            input_lamports,
            slippage_bps,
            true,
        )?;
        anyhow::ensure!(
            preview.hops.len() == 2 && preview.minimum_output_credit >= target_min,
            "SDK two-hop preview did not preserve final minimum output"
        );
        let params = two_hop_preview_swap_params(
            lane,
            DexParamEnum::StonkFunQuoteRoute(route),
            *target_mint,
            spl_token::ID,
            input_lamports,
            slippage_bps,
        );
        let instructions = StonkFunInstructionBuilder
            .build_buy_instructions(&params)
            .await?;
        anyhow::ensure!(
            !instructions.is_empty(),
            "SDK two-hop builder returned no instructions"
        );
        let cpmm_program = sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM;
        let target_program =
            sol_trade_sdk::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4;
        let swap_programs = instructions
            .iter()
            .map(|instruction| instruction.program_id)
            .filter(|program| *program == cpmm_program || *program == target_program)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            swap_programs == [cpmm_program, target_program],
            "SDK two-hop builder did not produce exactly CPMM then AMM v4"
        );
        let end_slot = rpc.get_slot().await?;
        let slot_span = end_slot
            .checked_sub(start_slot)
            .context("SDK two-hop RPC slot moved backwards")?;
        anyhow::ensure!(
            slot_span <= 64,
            "SDK two-hop pool and fee snapshots span {slot_span} slots (maximum 64)"
        );
        let mut program_ids = Vec::new();
        for instruction in &instructions {
            let program_id = instruction.program_id.to_string();
            if !program_ids.contains(&program_id) {
                program_ids.push(program_id);
            }
        }
        let simulation = if simulate_transaction {
            Some(
                simulate_two_hop_instructions(
                    lane,
                    &instructions,
                    target_mint,
                    &spl_token::ID,
                    target_min,
                    end_slot,
                )
                .await?,
            )
        } else {
            None
        };
        Ok(SdkTwoHopBuildPreview {
            builder: "sol-trade-sdk-11:stonkfun-quote-route",
            input_lamports,
            bridge_min_output_units: bridge_min,
            target_min_output_units: target_min,
            instruction_count: instructions.len(),
            program_ids,
            validated_slot: end_slot,
            snapshot_slot_span: slot_span,
            simulation,
        })
    }

    /// Quote and assemble a CPMM -> CPMM SOL buy from two fresh on-chain
    /// snapshots. A successful preview remains read-only until a separate
    /// unsigned simulation and a wallet-bound simulate-only decision pass.
    pub async fn preview_cpmm_via_cpmm(
        &self,
        target_pool_id: &Pubkey,
        bridge_pool_id: &Pubkey,
        target_mint: &Pubkey,
        quote_mint: &Pubkey,
        input_lamports: u64,
        slippage_bps: u64,
        simulate_transaction: bool,
    ) -> anyhow::Result<SdkTwoHopBuildPreview> {
        Ok(self
            .prepare_cpmm_buy_via_cpmm(
                target_pool_id,
                bridge_pool_id,
                target_mint,
                quote_mint,
                input_lamports,
                slippage_bps,
                simulate_transaction,
            )
            .await?
            .preview)
    }

    pub(crate) async fn prepare_cpmm_buy_via_cpmm(
        &self,
        target_pool_id: &Pubkey,
        bridge_pool_id: &Pubkey,
        target_mint: &Pubkey,
        quote_mint: &Pubkey,
        input_lamports: u64,
        slippage_bps: u64,
        simulate_transaction: bool,
    ) -> anyhow::Result<SdkTwoHopBuyPrepared> {
        anyhow::ensure!(input_lamports > 0, "SDK two-hop input must be positive");
        anyhow::ensure!(slippage_bps < 10_000, "SDK two-hop slippage is invalid");
        anyhow::ensure!(
            target_pool_id != bridge_pool_id,
            "SDK two-hop pools must differ"
        );
        let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
        anyhow::ensure!(
            *target_mint != *quote_mint && *quote_mint != wsol && *target_mint != wsol,
            "SDK two-hop mints must be distinct"
        );
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for SDK two-hop preview")?;
        let rpc = lane.client.get_rpc();
        let start_slot = rpc.get_slot().await?;
        let (bridge, target) = tokio::try_join!(
            load_open_cpmm_params(rpc, bridge_pool_id),
            load_open_cpmm_params(rpc, target_pool_id),
        )?;
        anyhow::ensure!(
            bridge.pool_state == *bridge_pool_id && target.pool_state == *target_pool_id,
            "SDK two-hop pool identity changed"
        );
        let (bridge_is_base_in, _, bridge_output_program, bridge_output_fee) =
            cpmm_hop_direction(&bridge, &wsol, quote_mint)?;
        let (target_is_base_in, target_input_program, target_output_program, target_output_fee) =
            cpmm_hop_direction(&target, quote_mint, target_mint)?;
        anyhow::ensure!(
            bridge_output_program == target_input_program,
            "SDK two-hop intermediate token program mismatch"
        );
        let bridge_quote = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
            &bridge,
            bridge_is_base_in,
            input_lamports,
            slippage_bps,
        )?;
        let bridge_min = bridge_quote.min_amount_out;
        anyhow::ensure!(bridge_min > 0, "SDK two-hop bridge minimum is zero");
        let target_quote = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
            &target,
            target_is_base_in,
            bridge_min,
            slippage_bps,
        )?;
        let target_min = target_quote.min_amount_out;
        anyhow::ensure!(target_min > 0, "SDK two-hop target minimum is zero");
        let route = StonkFunQuoteRoute::buy(vec![
            StonkFunQuoteHop::exact_in(
                StonkFunQuoteVenue::RaydiumCpmm(bridge),
                wsol,
                *quote_mint,
                bridge_min,
            )
            .with_output_transfer_fee(bridge_output_fee),
            StonkFunQuoteHop::exact_in(
                StonkFunQuoteVenue::RaydiumCpmm(target),
                *quote_mint,
                *target_mint,
                target_min,
            )
            .with_output_transfer_fee(target_output_fee),
        ]);
        let preview = route.preview(
            sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT,
            *target_mint,
            input_lamports,
            slippage_bps,
            true,
        )?;
        anyhow::ensure!(
            preview.hops.len() == 2 && preview.minimum_output_credit >= target_min,
            "SDK two-hop preview did not preserve final minimum output"
        );
        let params = two_hop_preview_swap_params(
            lane,
            DexParamEnum::StonkFunQuoteRoute(route),
            *target_mint,
            target_output_program,
            input_lamports,
            slippage_bps,
        );
        let instructions = StonkFunInstructionBuilder
            .build_buy_instructions(&params)
            .await?;
        let cpmm_program = sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM;
        let swap_count = instructions
            .iter()
            .filter(|instruction| instruction.program_id == cpmm_program)
            .count();
        anyhow::ensure!(
            swap_count == 2,
            "SDK two-hop builder did not produce two CPMM swaps"
        );
        let end_slot = rpc.get_slot().await?;
        let slot_span = end_slot
            .checked_sub(start_slot)
            .context("SDK two-hop RPC slot moved backwards")?;
        anyhow::ensure!(
            slot_span <= 64,
            "SDK two-hop pool and fee snapshots span {slot_span} slots (maximum 64)"
        );
        let mut program_ids = Vec::new();
        for instruction in &instructions {
            let program_id = instruction.program_id.to_string();
            if !program_ids.contains(&program_id) {
                program_ids.push(program_id);
            }
        }
        let (simulation, unsigned) = if simulate_transaction {
            let (simulation, unsigned) = simulate_two_hop_instructions_prepared(
                lane,
                &instructions,
                target_mint,
                &target_output_program,
                target_min,
                end_slot,
            )
            .await?;
            (Some(simulation), Some(unsigned))
        } else {
            (None, None)
        };
        Ok(SdkTwoHopBuyPrepared {
            preview: SdkTwoHopBuildPreview {
                builder: "sol-trade-sdk-11:stonkfun-quote-route",
                input_lamports,
                bridge_min_output_units: bridge_min,
                target_min_output_units: target_min,
                instruction_count: instructions.len(),
                program_ids,
                validated_slot: end_slot,
                snapshot_slot_span: slot_span,
                simulation,
            },
            unsigned,
        })
    }

    /// Assemble a protected CPMM target-token -> Quote -> WSOL sell without
    /// signing or submitting it. The quote is based on the selected wallet's
    /// target-token ATA; non-ATA balances are deliberately excluded.
    pub async fn preview_cpmm_sell_via_cpmm(
        &self,
        target_pool_id: &Pubkey,
        bridge_pool_id: &Pubkey,
        target_mint: &Pubkey,
        quote_mint: &Pubkey,
        sell_percent: u64,
        slippage_bps: u64,
        simulate_transaction: bool,
    ) -> anyhow::Result<SdkTwoHopSellBuildPreview> {
        Ok(self
            .prepare_cpmm_sell_via_cpmm(
                target_pool_id,
                bridge_pool_id,
                target_mint,
                quote_mint,
                sell_percent,
                slippage_bps,
                simulate_transaction,
            )
            .await?
            .preview)
    }

    pub(crate) async fn prepare_cpmm_sell_via_cpmm(
        &self,
        target_pool_id: &Pubkey,
        bridge_pool_id: &Pubkey,
        target_mint: &Pubkey,
        quote_mint: &Pubkey,
        sell_percent: u64,
        slippage_bps: u64,
        simulate_transaction: bool,
    ) -> anyhow::Result<SdkTwoHopSellPrepared> {
        anyhow::ensure!(
            (1..=100).contains(&sell_percent),
            "SDK two-hop sell percent is invalid"
        );
        anyhow::ensure!(
            slippage_bps < 10_000,
            "SDK two-hop sell slippage is invalid"
        );
        let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
        anyhow::ensure!(
            target_pool_id != bridge_pool_id
                && target_mint != quote_mint
                && *target_mint != wsol
                && *quote_mint != wsol,
            "SDK two-hop sell pools and mints must be distinct"
        );
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for SDK two-hop sell preview")?;
        let rpc = lane.client.get_rpc();
        let start_slot = rpc.get_slot().await?;
        let (target, bridge) = tokio::try_join!(
            load_open_cpmm_params(rpc, target_pool_id),
            load_open_cpmm_params(rpc, bridge_pool_id),
        )?;
        let (_, input_program, _, _) = cpmm_hop_direction(&target, target_mint, quote_mint)?;
        let input_ata = get_associated_token_address_with_program_id_fast(
            &lane.client.payer.pubkey(),
            target_mint,
            &input_program,
        );
        let balance = rpc
            .get_token_account_balance(&input_ata)
            .await?
            .amount
            .parse::<u64>()
            .context("invalid target token ATA balance")?;
        let input_units = u64::try_from(
            (balance as u128)
                .checked_mul(sell_percent as u128)
                .context("SDK two-hop sell amount overflow")?
                / 100,
        )?;
        anyhow::ensure!(input_units > 0, "SDK two-hop sell ATA balance is zero");
        let (route, target_min, bridge_min, route_input_program) = quote_cpmm_sell_via_cpmm(
            target,
            bridge,
            target_mint,
            quote_mint,
            input_units,
            slippage_bps,
        )?;
        anyhow::ensure!(
            route_input_program == input_program,
            "SDK two-hop sell input token program changed"
        );
        let mut params = two_hop_preview_swap_params(
            lane,
            DexParamEnum::StonkFunQuoteRoute(route),
            *target_mint,
            input_program,
            input_units,
            slippage_bps,
        );
        params.trade_type = TradeType::Sell;
        params.input_mint = *target_mint;
        params.input_token_program = Some(input_program);
        params.output_mint = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
        params.output_token_program = Some(spl_token::ID);
        params.create_input_mint_ata = false;
        params.close_input_mint_ata = false;
        params.create_output_mint_ata = true;
        params.close_output_mint_ata = false;
        params.use_exact_sol_amount = None;
        let instructions = StonkFunInstructionBuilder
            .build_sell_instructions(&params)
            .await?;
        let cpmm_program = sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM;
        anyhow::ensure!(
            instructions
                .iter()
                .filter(|instruction| instruction.program_id == cpmm_program)
                .count()
                == 2,
            "SDK two-hop sell builder did not produce two CPMM swaps"
        );
        let end_slot = rpc.get_slot().await?;
        let slot_span = end_slot
            .checked_sub(start_slot)
            .context("SDK two-hop sell RPC slot moved backwards")?;
        anyhow::ensure!(
            slot_span <= 64,
            "SDK two-hop sell snapshots span {slot_span} slots (maximum 64)"
        );
        let mut program_ids = Vec::new();
        for instruction in &instructions {
            let program_id = instruction.program_id.to_string();
            if !program_ids.contains(&program_id) {
                program_ids.push(program_id);
            }
        }
        let (simulation, unsigned) = if simulate_transaction {
            let (simulation, unsigned) = simulate_two_hop_sell_instructions(
                lane,
                &instructions,
                target_mint,
                &input_program,
                &input_ata,
                balance,
                input_units,
                bridge_min,
                end_slot,
            )
            .await?;
            (Some(simulation), Some(unsigned))
        } else {
            (None, None)
        };
        Ok(SdkTwoHopSellPrepared {
            preview: SdkTwoHopSellBuildPreview {
                builder: "sol-trade-sdk-11:stonkfun-quote-route-sell",
                input_token_units: input_units,
                input_token_ata: input_ata.to_string(),
                input_token_ata_balance_units: balance,
                target_min_quote_units: target_min,
                bridge_min_wsol_units: bridge_min,
                instruction_count: instructions.len(),
                program_ids,
                validated_slot: end_slot,
                snapshot_slot_span: slot_span,
                simulation,
                executable: false,
            },
            unsigned,
        })
    }

    /// Quote and assemble a CPMM -> Pump.fun V3 USDC route without signing.
    /// The caller receives only diagnostic instructions and optional unsigned
    /// simulation; this method does not issue a trade-bound preview credential.
    pub async fn preview_pumpfun_v3_usdc_via_cpmm(
        &self,
        bridge_pool_id: &Pubkey,
        target_mint: &Pubkey,
        quote_mint: &Pubkey,
        input_lamports: u64,
        slippage_bps: u64,
        simulate_transaction: bool,
    ) -> anyhow::Result<SdkTwoHopBuildPreview> {
        anyhow::ensure!(
            input_lamports > 0 && slippage_bps < 10_000,
            "invalid Pump.fun preview amount or slippage"
        );
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for Pump.fun preview")?;
        let rpc = lane.client.get_rpc();
        let start_slot = rpc.get_slot().await?;
        let (bridge, mut target) = tokio::try_join!(
            load_open_cpmm_params(rpc, bridge_pool_id),
            PumpFunParams::from_mint_by_rpc(rpc, target_mint),
        )?;
        let (base_mint_account, quote_mint_account) =
            tokio::try_join!(rpc.get_account(target_mint), rpc.get_account(quote_mint),)?;
        anyhow::ensure!(
            target.quote_mint == *quote_mint && target.token_program == base_mint_account.owner,
            "Pump.fun preview target mint or quote changed"
        );
        let snapshot = crate::pumpfun_quote_snapshot::load_pumpfun_quote_accounts(
            rpc,
            *target_mint,
            *quote_mint,
            base_mint_account.owner,
            quote_mint_account.owner,
        )
        .await?;
        anyhow::ensure!(
            target.bonding_curve.account == snapshot.curve.account
                && target.associated_bonding_curve
                    == get_associated_token_address_with_program_id_fast(
                        &snapshot.curve.account,
                        target_mint,
                        &base_mint_account.owner,
                    ),
            "Pump.fun preview curve or base vault changed"
        );
        target.bonding_curve = Arc::new(snapshot.curve.clone());
        let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
        anyhow::ensure!(
            bridge.pool_state == *bridge_pool_id
                && ((bridge.base_mint == wsol && bridge.quote_mint == *quote_mint)
                    || (bridge.quote_mint == wsol && bridge.base_mint == *quote_mint)),
            "Pump.fun preview CPMM bridge does not connect WSOL to target quote"
        );
        let bridge_is_base_in = bridge.base_mint == wsol;
        let bridge_output_program = if bridge_is_base_in {
            bridge.quote_token_program
        } else {
            bridge.base_token_program
        };
        let bridge_output_fee = if bridge_is_base_in {
            bridge.quote_transfer_fee
        } else {
            bridge.base_transfer_fee
        };
        anyhow::ensure!(
            bridge_output_program == quote_mint_account.owner
                && bridge_output_fee.basis_points == 0,
            "Pump.fun preview bridge output program or transfer fee is unsupported"
        );
        let bridge_quote = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
            &bridge,
            bridge_is_base_in,
            input_lamports,
            slippage_bps,
        )?;
        let bridge_min = bridge_quote.min_amount_out;
        anyhow::ensure!(bridge_min > 0, "Pump.fun preview bridge minimum is zero");
        let curve_quote = snapshot.quote_usdc_v3_exact_in(*quote_mint, bridge_min, slippage_bps)?;
        let target_min = curve_quote.minimum_base_out;
        let base_program = base_mint_account.owner;
        let route = PumpFunQuoteRoute::buy(vec![
            PumpFunQuoteHop::exact_in(
                PumpFunQuoteVenue::RaydiumCpmm(bridge),
                wsol,
                *quote_mint,
                bridge_min,
            )
            .with_output_transfer_fee(bridge_output_fee),
            PumpFunQuoteHop::exact_in(
                PumpFunQuoteVenue::PumpFunV3 {
                    mint: *target_mint,
                    params: PumpFunV3Params {
                        curve: target,
                        quote_token_program: quote_mint_account.owner,
                        minimum_output_amount: target_min,
                        base_transfer_fee: snapshot.base_transfer_fee,
                        quote_transfer_fee: snapshot.quote_transfer_fee,
                    },
                },
                *quote_mint,
                *target_mint,
                target_min,
            )
            .with_output_transfer_fee(snapshot.base_transfer_fee),
        ]);
        let route_preview = route.preview(
            sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT,
            *target_mint,
            input_lamports,
            slippage_bps,
            true,
        )?;
        anyhow::ensure!(
            route_preview.hops.len() == 2 && route_preview.minimum_output_credit >= target_min,
            "Pump.fun preview did not preserve final minimum output"
        );
        let params = two_hop_preview_swap_params(
            lane,
            DexParamEnum::PumpFunQuoteRoute(route),
            *target_mint,
            base_program,
            input_lamports,
            slippage_bps,
        );
        let instructions = PumpFunInstructionBuilder
            .build_buy_instructions(&params)
            .await?;
        let cpmm_program = sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM;
        let pump_program = sol_trade_sdk::instruction::utils::pumpfun::accounts::PUMPFUN;
        let swap_programs = instructions
            .iter()
            .map(|ix| ix.program_id)
            .filter(|program| *program == cpmm_program || *program == pump_program)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            swap_programs == [cpmm_program, pump_program],
            "Pump.fun preview did not produce exactly CPMM then curve swap"
        );
        let end_slot = rpc.get_slot().await?;
        let slot_span = end_slot
            .checked_sub(start_slot)
            .context("Pump.fun preview RPC slot moved backwards")?;
        anyhow::ensure!(
            slot_span <= 64 && end_slot >= snapshot.slot,
            "Pump.fun preview account snapshot is stale"
        );
        let mut program_ids = Vec::new();
        for instruction in &instructions {
            let program_id = instruction.program_id.to_string();
            if !program_ids.contains(&program_id) {
                program_ids.push(program_id);
            }
        }
        let simulation = if simulate_transaction {
            Some(
                simulate_two_hop_instructions(
                    lane,
                    &instructions,
                    target_mint,
                    &base_program,
                    target_min,
                    end_slot,
                )
                .await?,
            )
        } else {
            None
        };
        Ok(SdkTwoHopBuildPreview {
            builder: "sol-trade-sdk-11:pumpfun-v3-quote-route",
            input_lamports,
            bridge_min_output_units: bridge_min,
            target_min_output_units: target_min,
            instruction_count: instructions.len(),
            program_ids,
            validated_slot: end_slot,
            snapshot_slot_span: slot_span,
            simulation,
        })
    }

    /// Quote a PumpSwap quote-token-in buy from a refreshed Pool/vault snapshot.
    /// Missing current fee configuration, cashback rates or nonzero transfer
    /// fees must not become an optimistic quote. This is read-only and does not
    /// establish that the caller has a buildable transaction.
    pub async fn quote_pumpswap_buy(
        &self,
        pool_id: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        amount_in: u64,
    ) -> anyhow::Result<(u64, u64)> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for PumpSwap quote")?;
        let rpc = lane.client.get_rpc();
        let (params, base_fee, quote_fee) = load_pumpswap_pool_strict(rpc, pool_id).await?;
        quote_pumpswap_buy_with_params(
            &params,
            input_mint,
            output_mint,
            amount_in,
            base_fee,
            quote_fee,
        )
    }

    /// Verify current SDK 11 parameters, quote both exact-input legs and
    /// assemble instructions without signing or submitting a transaction.
    /// A successful preview is not a simulation or an execution decision.
    pub async fn preview_pumpswap_via_cpmm(
        &self,
        target_pool_id: &Pubkey,
        bridge_pool_id: &Pubkey,
        target_mint: &Pubkey,
        quote_mint: &Pubkey,
        input_lamports: u64,
        slippage_bps: u64,
        simulate_transaction: bool,
    ) -> anyhow::Result<SdkTwoHopBuildPreview> {
        anyhow::ensure!(input_lamports > 0, "SDK two-hop input must be positive");
        anyhow::ensure!(slippage_bps < 10_000, "SDK two-hop slippage is invalid");
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for SDK two-hop preview")?;
        let rpc = lane.client.get_rpc();
        let start_slot = rpc.get_slot().await?;
        let (bridge, (target, base_fee, quote_fee)) = tokio::try_join!(
            load_open_cpmm_params(rpc, bridge_pool_id),
            load_pumpswap_pool_strict(rpc, target_pool_id),
        )?;
        let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
        anyhow::ensure!(
            target.pool == *target_pool_id
                && target.base_mint == *target_mint
                && target.quote_mint == *quote_mint,
            "PumpSwap two-hop target does not match verified pool and mints"
        );
        target.validate_accounts()?;
        anyhow::ensure!(
            bridge.pool_state == *bridge_pool_id
                && ((bridge.base_mint == wsol && bridge.quote_mint == *quote_mint)
                    || (bridge.quote_mint == wsol && bridge.base_mint == *quote_mint)),
            "CPMM two-hop bridge does not match verified pool and mints"
        );
        let bridge_is_base_in = bridge.base_mint == wsol;
        let bridge_output_program = if bridge_is_base_in {
            bridge.quote_token_program
        } else {
            bridge.base_token_program
        };
        let bridge_output_fee = if bridge_is_base_in {
            bridge.quote_transfer_fee
        } else {
            bridge.base_transfer_fee
        };
        anyhow::ensure!(
            bridge_output_program == target.quote_token_program,
            "SDK two-hop intermediate token program mismatch"
        );
        anyhow::ensure!(
            base_fee.basis_points == 0 && quote_fee.basis_points == 0,
            "PumpSwap quote route requires zero transfer fees"
        );
        let base_program = target.base_token_program;
        let bridge_quote = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
            &bridge,
            bridge_is_base_in,
            input_lamports,
            slippage_bps,
        )?;
        let bridge_min = bridge_quote.min_amount_out;
        anyhow::ensure!(bridge_min > 0, "SDK two-hop bridge minimum is zero");
        let target_quote =
            sol_trade_sdk::utils::calc::pumpswap::buy_quote_input_internal_with_fees(
                bridge_min,
                0,
                target.pool_base_token_reserves,
                target.pool_quote_token_reserves,
                target.virtual_quote_reserves,
                &target.fee_basis_points,
            )
            .map_err(anyhow::Error::msg)?;
        let target_min = (target_quote.base as u128)
            .checked_mul((10_000 - slippage_bps) as u128)
            .context("SDK two-hop minimum output overflow")?
            / 10_000;
        let target_min = u64::try_from(target_min)?;
        anyhow::ensure!(target_min > 0, "SDK two-hop target minimum is zero");

        let route = PumpSwapQuoteRoute::buy(vec![
            PumpSwapQuoteHop::exact_in(
                PumpSwapQuoteVenue::RaydiumCpmm(bridge),
                wsol,
                *quote_mint,
                bridge_min,
            )
            .with_output_transfer_fee(bridge_output_fee),
            PumpSwapQuoteHop::exact_in(
                PumpSwapQuoteVenue::PumpSwap {
                    pool: target,
                    base_transfer_fee: base_fee,
                    quote_transfer_fee: quote_fee,
                },
                *quote_mint,
                *target_mint,
                target_min,
            )
            .with_output_transfer_fee(base_fee),
        ]);
        let preview = route.preview(
            sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT,
            *target_mint,
            input_lamports,
            slippage_bps,
            true,
        )?;
        anyhow::ensure!(
            preview.hops.len() == 2 && preview.minimum_output_credit >= target_min,
            "SDK two-hop preview did not preserve final minimum output"
        );
        let params = two_hop_preview_swap_params(
            lane,
            DexParamEnum::PumpSwapQuoteRoute(route),
            *target_mint,
            base_program,
            input_lamports,
            slippage_bps,
        );
        let instructions = PumpSwapInstructionBuilder
            .build_buy_instructions(&params)
            .await?;
        anyhow::ensure!(
            !instructions.is_empty(),
            "SDK two-hop builder returned no instructions"
        );
        let cpmm_program = sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM;
        let pumpswap_program = sol_trade_sdk::instruction::utils::pumpswap::accounts::AMM_PROGRAM;
        let swap_programs = instructions
            .iter()
            .map(|instruction| instruction.program_id)
            .filter(|program| *program == cpmm_program || *program == pumpswap_program)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            swap_programs == [cpmm_program, pumpswap_program],
            "SDK two-hop builder did not produce exactly CPMM then PumpSwap"
        );
        let end_slot = rpc.get_slot().await?;
        let slot_span = end_slot
            .checked_sub(start_slot)
            .context("SDK two-hop RPC slot moved backwards")?;
        anyhow::ensure!(
            slot_span <= 64,
            "SDK two-hop pool and fee snapshots span {slot_span} slots (maximum 64)"
        );
        let mut program_ids = Vec::new();
        for instruction in &instructions {
            let program_id = instruction.program_id.to_string();
            if !program_ids.contains(&program_id) {
                program_ids.push(program_id);
            }
        }
        let simulation = if simulate_transaction {
            Some(
                simulate_two_hop_instructions(
                    lane,
                    &instructions,
                    target_mint,
                    &base_program,
                    target_min,
                    end_slot,
                )
                .await?,
            )
        } else {
            None
        };
        Ok(SdkTwoHopBuildPreview {
            builder: "sol-trade-sdk-11:pumpswap-quote-route",
            input_lamports,
            bridge_min_output_units: bridge_min,
            target_min_output_units: target_min,
            instruction_count: instructions.len(),
            program_ids,
            validated_slot: end_slot,
            snapshot_slot_span: slot_span,
            simulation,
        })
    }

    /// Build and simulate a buy using the same SDK path as execution. The SDK
    /// receives `simulate=true`, so it only calls `simulateTransaction`; this
    /// method never invokes a submit path and intentionally returns no
    /// signature to callers.
    pub async fn simulate_buy(
        &self,
        pool: Pool,
        mint: Pubkey,
        quote_mint: Pubkey,
        pool_id: Option<Pubkey>,
        amount_in: u64,
        slippage_bps: u64,
    ) -> anyhow::Result<ExecuteResult> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for simulation")?;
        let recent_blockhash = lane.client.get_rpc().get_latest_blockhash().await?;
        self.buy(ExecuteTrade {
            pool,
            mint,
            quote_mint,
            pool_id,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: Some(amount_in),
            sell_percent: None,
            slippage_bps,
            tip_sol: 0.0,
            recent_blockhash,
            wait_tx_confirmed: false,
            mev_submit_verify_ms: 0,
            simulate: true,
            use_mev: Some(false),
            mev_provider: Some(MevProvider::Default),
        })
        .await
    }

    fn execution_lanes(&self) -> Vec<&SdkLane> {
        let mev_lanes = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type != SwqosType::Default)
            .collect::<Vec<_>>();
        if self.require_mev_submit && !mev_lanes.is_empty() {
            return mev_lanes;
        }
        let default_lanes = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default);
        mev_lanes.into_iter().chain(default_lanes).collect()
    }

    fn trade_execution_lanes(&self, trade: &ExecuteTrade) -> anyhow::Result<Vec<&SdkLane>> {
        let mut mev_lanes = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type != SwqosType::Default)
            .collect::<Vec<_>>();
        if let Some(provider_type) = trade.mev_provider.and_then(provider_swqos_type) {
            mev_lanes.retain(|lane| lane.swqos_type == provider_type);
        }

        let default_lanes = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
            .collect::<Vec<_>>();

        let lanes = match (trade.use_mev, trade.mev_provider) {
            (Some(false), _) | (_, Some(MevProvider::Default)) => default_lanes,
            (Some(true), _) => mev_lanes,
            (_, Some(MevProvider::Auto)) => {
                if mev_lanes.is_empty() {
                    default_lanes
                } else {
                    mev_lanes
                }
            }
            (_, Some(_)) => mev_lanes,
            (None, None) => self.execution_lanes(),
        };

        if lanes.is_empty() {
            let target = trade
                .mev_provider
                .map(provider_name)
                .unwrap_or("requested execution mode");
            bail!("{target} has no enabled execution lane");
        }
        Ok(lanes)
    }
}

async fn load_pumpswap_pool_strict(
    rpc: &sol_trade_sdk::common::SolanaRpcClient,
    pool_id: &Pubkey,
) -> anyhow::Result<(PumpSwapParams, TokenTransferFee, TokenTransferFee)> {
    let pool_hint = sol_trade_sdk::instruction::utils::pumpswap::fetch_pool(rpc, pool_id).await?;
    let (pool_snapshot, fee_config, global_config, base_mint, quote_mint) = tokio::join!(
        sol_trade_sdk::instruction::utils::pumpswap::get_pool_rpc_snapshot_with_address(
            pool_id, &pool_hint, rpc,
        ),
        sol_trade_sdk::instruction::utils::pumpswap::fetch_fee_config(rpc),
        sol_trade_sdk::instruction::utils::pumpswap::fetch_global_config(rpc),
        TokenTransferFee::from_mint_by_rpc(rpc, &pool_hint.base_mint),
        TokenTransferFee::from_mint_by_rpc(rpc, &pool_hint.quote_mint),
    );
    let (pool, snapshot) = pool_snapshot?;
    let fee_config = fee_config.context("PumpSwap fee config is unavailable")?;
    let global_config = global_config.context("PumpSwap global config is unavailable")?;
    let (base_program, base_fee) = base_mint?;
    let (quote_program, quote_fee) = quote_mint?;
    let params = PumpSwapParams::from_cached_pool_snapshot_strict(
        pool_id,
        &pool,
        &snapshot,
        &fee_config,
        &global_config,
    )?;
    anyhow::ensure!(
        base_program == params.base_token_program && quote_program == params.quote_token_program,
        "PumpSwap mint owner does not match the pool vault token program"
    );
    anyhow::ensure!(
        !params.is_cashback_coin,
        "PumpSwap cashback fee rate is unavailable from an RPC-only pool snapshot"
    );
    params.validate_accounts()?;
    Ok((params, base_fee, quote_fee))
}

fn quote_pumpswap_buy_with_params(
    params: &PumpSwapParams,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    amount_in: u64,
    base_fee: TokenTransferFee,
    quote_fee: TokenTransferFee,
) -> anyhow::Result<(u64, u64)> {
    anyhow::ensure!(amount_in > 0, "PumpSwap quote input must be positive");
    anyhow::ensure!(
        *input_mint == params.quote_mint && *output_mint == params.base_mint,
        "PumpSwap quote mints do not match the pool buy direction"
    );
    params.validate_accounts()?;
    anyhow::ensure!(
        !params.is_cashback_coin,
        "PumpSwap cashback fee rate is unavailable from an RPC-only pool snapshot"
    );
    anyhow::ensure!(
        base_fee.basis_points == 0 && quote_fee.basis_points == 0,
        "PumpSwap exact-in quote requires zero token transfer fees"
    );
    let quote = sol_trade_sdk::utils::calc::pumpswap::buy_quote_input_internal_with_fees(
        amount_in,
        0,
        params.pool_base_token_reserves,
        params.pool_quote_token_reserves,
        params.virtual_quote_reserves,
        &params.fee_basis_points,
    )
    .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(quote.base > 0, "PumpSwap quote output is zero");
    let fee_units = amount_in
        .checked_sub(quote.internal_quote_without_fees)
        .context("PumpSwap quote fee exceeds input")?;
    Ok((quote.base, fee_units))
}

const SOL_TRANSFER_SAFETY_MARGIN_LAMPORTS: u64 = 10_000;

fn calculate_sol_transfer_amount(
    balance: u64,
    fee: u64,
    rent_exempt_minimum: u64,
    fixed_lamports: Option<u64>,
    percent: Option<u64>,
) -> anyhow::Result<(u64, u64)> {
    if fixed_lamports.is_none() && percent == Some(100) {
        let amount = balance
            .checked_sub(fee)
            .context("insufficient SOL balance for network fee")?;
        if amount == 0 {
            bail!("insufficient SOL balance after network fee");
        }
        return Ok((amount, 0));
    }

    let reserve = rent_exempt_minimum.saturating_add(SOL_TRANSFER_SAFETY_MARGIN_LAMPORTS);
    let spendable = balance
        .checked_sub(fee.saturating_add(reserve))
        .with_context(|| {
            format!(
                "insufficient SOL balance for fee and rent reserve: balance={balance}, fee={fee}, reserve={reserve} lamports"
            )
        })?;
    let amount = match (fixed_lamports, percent) {
        (Some(amount), None) if amount > 0 => amount,
        (None, Some(50)) => spendable / 2,
        _ => bail!("provide either a positive fixed amount or percent 50/100"),
    };
    if amount == 0 || amount > spendable {
        bail!(
            "insufficient SOL balance: spendable={} lamports, requested={} lamports",
            spendable,
            amount
        );
    }
    Ok((amount, reserve))
}

fn provider_swqos_type(provider: MevProvider) -> Option<SwqosType> {
    match provider {
        MevProvider::Auto | MevProvider::Default => None,
        MevProvider::Jito => Some(SwqosType::Jito),
        MevProvider::Temporal => Some(SwqosType::Temporal),
        MevProvider::FlashBlock => Some(SwqosType::FlashBlock),
        MevProvider::BlockRazor => Some(SwqosType::BlockRazor),
        MevProvider::Astralane => Some(SwqosType::Astralane),
        MevProvider::Speedlanding => Some(SwqosType::Speedlanding),
    }
}

fn minimum_output(quoted: u64, slippage_bps: u64) -> anyhow::Result<u64> {
    if quoted == 0 || slippage_bps >= 10_000 {
        bail!("invalid DAMM v2 quote or slippage");
    }
    let minimum = (quoted as u128)
        .checked_mul((10_000 - slippage_bps) as u128)
        .context("DAMM v2 minimum output overflow")?
        / 10_000;
    u64::try_from(minimum)
        .ok()
        .filter(|value| *value > 0)
        .context("DAMM v2 minimum output rounds to zero")
}

fn provider_name(provider: MevProvider) -> &'static str {
    match provider {
        MevProvider::Auto => "auto MEV",
        MevProvider::Default => "default RPC",
        MevProvider::Jito => "jito",
        MevProvider::Temporal => "temporal",
        MevProvider::FlashBlock => "flashblock",
        MevProvider::BlockRazor => "blockrazor",
        MevProvider::Astralane => "astralane",
        MevProvider::Speedlanding => "speedlanding",
    }
}

fn cpmm_route_token_programs(
    mint: &Pubkey,
    quote_mint: &Pubkey,
    base_mint: &Pubkey,
    pool_quote_mint: &Pubkey,
    base_program: &Pubkey,
    pool_quote_program: &Pubkey,
) -> anyhow::Result<(Pubkey, Pubkey)> {
    pool_route_token_programs(
        "Raydium CPMM",
        mint,
        quote_mint,
        base_mint,
        pool_quote_mint,
        base_program,
        pool_quote_program,
    )
}

fn pool_route_token_programs(
    pool: &str,
    mint: &Pubkey,
    quote_mint: &Pubkey,
    base_mint: &Pubkey,
    pool_quote_mint: &Pubkey,
    base_program: &Pubkey,
    pool_quote_program: &Pubkey,
) -> anyhow::Result<(Pubkey, Pubkey)> {
    anyhow::ensure!(mint != quote_mint, "{pool} route mints must differ");
    let supported = |program: &Pubkey| *program == spl_token::ID || *program == spl_token_2022::ID;
    anyhow::ensure!(
        supported(base_program) && supported(pool_quote_program),
        "{pool} pool uses unsupported token program"
    );
    if mint == base_mint && quote_mint == pool_quote_mint {
        Ok((*base_program, *pool_quote_program))
    } else if mint == pool_quote_mint && quote_mint == base_mint {
        Ok((*pool_quote_program, *base_program))
    } else {
        bail!("{pool} route mints do not match pool state")
    }
}

fn validate_amm_v4_route_mints(
    mint: &Pubkey,
    quote_mint: &Pubkey,
    coin_mint: &Pubkey,
    pc_mint: &Pubkey,
) -> anyhow::Result<()> {
    anyhow::ensure!(mint != quote_mint, "Raydium AMM v4 route mints must differ");
    anyhow::ensure!(
        (mint == coin_mint && quote_mint == pc_mint)
            || (mint == pc_mint && quote_mint == coin_mint),
        "Raydium AMM v4 route mints do not match pool state"
    );
    Ok(())
}

fn amm_v4_quote_direction(
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    coin_mint: &Pubkey,
    pc_mint: &Pubkey,
) -> anyhow::Result<bool> {
    validate_amm_v4_route_mints(output_mint, input_mint, coin_mint, pc_mint)?;
    Ok(input_mint == coin_mint)
}

fn amm_v4_pool_open(status: u64, open_time: u64, now_seconds: u64) -> bool {
    status == 6 || (status == 7 && now_seconds >= open_time)
}

fn cpmm_pool_open(status: u8, open_time: u64, now_seconds: u64) -> bool {
    status & 4 == 0 && now_seconds >= open_time
}

fn other_pool_mint(target: &Pubkey, first: &Pubkey, second: &Pubkey) -> anyhow::Result<Pubkey> {
    anyhow::ensure!(first != second, "selected pool mints must differ");
    if target == first {
        Ok(*second)
    } else if target == second {
        Ok(*first)
    } else {
        anyhow::bail!("selected pool does not contain target mint")
    }
}

fn cpmm_hop_direction(
    params: &RaydiumCpmmParams,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
) -> anyhow::Result<(bool, Pubkey, Pubkey, TokenTransferFee)> {
    let (output_program, input_program) = cpmm_route_token_programs(
        output_mint,
        input_mint,
        &params.base_mint,
        &params.quote_mint,
        &params.base_token_program,
        &params.quote_token_program,
    )?;
    let is_base_in = *input_mint == params.base_mint;
    let output_fee = if is_base_in {
        params.quote_transfer_fee
    } else {
        params.base_transfer_fee
    };
    Ok((is_base_in, input_program, output_program, output_fee))
}

/// Prepare an independently quoted reverse route for a token sell. The target
/// pool is spent first (token -> quote), then the CPMM bridge (quote -> WSOL).
/// No buy-side quote, instruction, or minimum is reused for this direction.
fn quote_cpmm_sell_via_cpmm(
    target: RaydiumCpmmParams,
    bridge: RaydiumCpmmParams,
    target_mint: &Pubkey,
    quote_mint: &Pubkey,
    input_token_units: u64,
    slippage_bps: u64,
) -> anyhow::Result<(StonkFunQuoteRoute, u64, u64, Pubkey)> {
    anyhow::ensure!(
        input_token_units > 0,
        "SDK two-hop sell input must be positive"
    );
    anyhow::ensure!(
        slippage_bps < 10_000,
        "SDK two-hop sell slippage is invalid"
    );
    let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
    anyhow::ensure!(
        target.pool_state != bridge.pool_state
            && *target_mint != *quote_mint
            && *target_mint != wsol
            && *quote_mint != wsol,
        "SDK two-hop sell pools and mints must be distinct"
    );
    let (target_is_base_in, target_input_program, target_output_program, target_output_fee) =
        cpmm_hop_direction(&target, target_mint, quote_mint)?;
    let (bridge_is_base_in, bridge_input_program, bridge_output_program, bridge_output_fee) =
        cpmm_hop_direction(&bridge, quote_mint, &wsol)?;
    anyhow::ensure!(
        target_output_program == bridge_input_program,
        "SDK two-hop sell intermediate token program mismatch"
    );
    anyhow::ensure!(
        bridge_output_program == spl_token::ID && bridge_output_fee.basis_points == 0,
        "SDK two-hop sell WSOL output must use classic SPL Token without transfer fee"
    );
    let target_quote = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
        &target,
        target_is_base_in,
        input_token_units,
        slippage_bps,
    )?;
    let target_min = target_quote.min_amount_out;
    anyhow::ensure!(target_min > 0, "SDK two-hop sell target minimum is zero");
    let bridge_quote = sol_trade_sdk::utils::calc::raydium_cpmm::compute_swap_amount_for_pool(
        &bridge,
        bridge_is_base_in,
        target_min,
        slippage_bps,
    )?;
    let bridge_min = bridge_quote.min_amount_out;
    anyhow::ensure!(bridge_min > 0, "SDK two-hop sell bridge minimum is zero");
    let route = StonkFunQuoteRoute::sell(vec![
        StonkFunQuoteHop::exact_in(
            StonkFunQuoteVenue::RaydiumCpmm(target),
            *target_mint,
            *quote_mint,
            target_min,
        )
        .with_output_transfer_fee(target_output_fee),
        StonkFunQuoteHop::exact_in(
            StonkFunQuoteVenue::RaydiumCpmm(bridge),
            *quote_mint,
            wsol,
            bridge_min,
        )
        .with_output_transfer_fee(bridge_output_fee),
    ]);
    let preview = route.preview(*target_mint, wsol, input_token_units, slippage_bps, false)?;
    anyhow::ensure!(
        preview.hops.len() == 2 && preview.minimum_output_credit >= bridge_min,
        "SDK two-hop sell preview did not preserve final minimum output"
    );
    Ok((route, target_min, bridge_min, target_input_program))
}

async fn load_open_cpmm_params(
    rpc: &sol_trade_sdk::common::SolanaRpcClient,
    pool_id: &Pubkey,
) -> anyhow::Result<RaydiumCpmmParams> {
    let (state, params) = tokio::try_join!(
        sol_trade_sdk::instruction::utils::raydium_cpmm::fetch_pool_state(rpc, pool_id),
        RaydiumCpmmParams::from_pool_address_by_rpc(rpc, pool_id),
    )?;
    let now_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    anyhow::ensure!(
        cpmm_pool_open(state.status, state.open_time, now_seconds),
        "Raydium CPMM pool is not open for swaps"
    );
    anyhow::ensure!(
        params.pool_state == *pool_id
            && params.amm_config == state.amm_config
            && params.base_mint == state.token0_mint
            && params.quote_mint == state.token1_mint
            && params.base_vault == state.token0_vault
            && params.quote_vault == state.token1_vault
            && params.base_token_program == state.token0_program
            && params.quote_token_program == state.token1_program
            && params.observation_state == state.observation_key
            && params.creator_fee_on == state.creator_fee_on
            && params.enable_creator_fee == state.enable_creator_fee,
        "Raydium CPMM pool identity changed while loading"
    );
    Ok(params)
}

fn validate_bonk_pool_identity(
    mint: &Pubkey,
    quote_mint: &Pubkey,
    pool_base_mint: &Pubkey,
    pool_quote_mint: &Pubkey,
    global_config: &Pubkey,
    platform_config: &Pubkey,
) -> anyhow::Result<()> {
    use sol_trade_sdk::instruction::utils::bonk::accounts;

    let expected_global = if is_wsol_quote(quote_mint) {
        accounts::GLOBAL_CONFIG
    } else if is_usd1_quote(quote_mint) {
        accounts::USD1_GLOBAL_CONFIG
    } else {
        bail!("Bonk route only supports WSOL or USD1 quote");
    };
    anyhow::ensure!(
        pool_base_mint == mint && pool_quote_mint == quote_mint,
        "Bonk pool mints do not match requested route"
    );
    anyhow::ensure!(
        *global_config == expected_global,
        "Bonk pool has an unexpected global config"
    );
    anyhow::ensure!(
        *platform_config != accounts::STONKFUN_STANDARD_PLATFORM_CONFIG
            && *platform_config != accounts::STONKFUN_REWARD_PLATFORM_CONFIG,
        "StonkFun pool cannot be labeled as Bonk"
    );
    Ok(())
}

fn ensure_pool_contains_mint(
    pool: &str,
    mint: &Pubkey,
    pool_mints: &[Pubkey],
) -> anyhow::Result<()> {
    if pool_mints.iter().any(|pool_mint| pool_mint == mint) {
        return Ok(());
    }
    bail!(
        "{pool} pool does not contain requested mint {mint}; pool mints: {}",
        pool_mints
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn ensure_pool_contains_quote(
    pool: &str,
    quote_mint: &Pubkey,
    pool_mints: &[Pubkey],
) -> anyhow::Result<()> {
    if pool_mints.iter().any(|pool_mint| pool_mint == quote_mint) {
        return Ok(());
    }
    bail!(
        "{pool} pool does not contain requested quote_mint {quote_mint}; pool mints: {}",
        pool_mints
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

async fn load_stonkfun_params(
    rpc: &sol_trade_sdk::common::SolanaRpcClient,
    pool_id: &Pubkey,
    mint: &Pubkey,
    quote_mint: &Pubkey,
) -> anyhow::Result<StonkFunParams> {
    use sol_trade_sdk::instruction::utils::bonk::{accounts, fetch_pool_state};

    let state = fetch_pool_state(rpc, pool_id).await?;
    ensure_pool_contains_mint("stonkfun", mint, &[state.base_mint])?;
    ensure_pool_contains_quote("stonkfun", quote_mint, &[state.quote_mint])?;
    if state.platform_config != accounts::STONKFUN_STANDARD_PLATFORM_CONFIG
        && state.platform_config != accounts::STONKFUN_REWARD_PLATFORM_CONFIG
    {
        bail!(
            "launchlab pool {} is not a recognized StonkFun platform: {}",
            pool_id,
            state.platform_config
        );
    }
    StonkFunParams::from_pool_by_rpc(rpc, pool_id).await
}

async fn load_stonkfun_sol_hop(
    client: &TradingClient,
    pool: Option<Pool>,
    pool_id: &Pubkey,
    quote_mint: &Pubkey,
) -> anyhow::Result<RaydiumCpmmParams> {
    if pool != Some(Pool::RaydiumCpmm) {
        bail!("stonkfun Via-SOL only supports a Raydium CPMM SOL hop");
    }
    let params = RaydiumCpmmParams::from_pool_address_by_rpc(client.get_rpc(), pool_id).await?;
    let mints = [params.base_mint, params.quote_mint];
    ensure_pool_contains_quote("stonkfun Via-SOL hop", quote_mint, &mints)?;
    let wsol = Pubkey::from_str(WSOL_MINT).expect("known WSOL mint must be valid");
    ensure_pool_contains_quote("stonkfun Via-SOL hop", &wsol, &mints)?;
    if quote_mint == &wsol {
        bail!("stonkfun Via-SOL hop requires a non-WSOL quote mint");
    }
    Ok(params)
}

/// Static capability of the direct `/v1/trades` builder in this service.
/// `Unavailable` does not prove the sol-trade-sdk package cannot build the
/// route through another API (for example, explicit Pump or quote routes).
/// It must never by itself authorize Jupiter fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectSdkBuildability {
    Available,
    Unavailable(&'static str),
    Incomplete(&'static str),
}

/// Package-level route family and the adapter actually wired into this
/// service. A named SDK family is not proof that the current pool snapshot can
/// build, simulate, or send. No value here authorizes aggregator fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BridgeSdkCapability {
    pub package_builder_family: &'static str,
    pub service_adapter: &'static str,
}

pub(crate) fn bridge_sdk_capability(pool: Pool) -> BridgeSdkCapability {
    match pool {
        Pool::Pump => BridgeSdkCapability {
            package_builder_family: "PumpFunQuoteRoute",
            service_adapter: "not_integrated",
        },
        Pool::PumpAmm => BridgeSdkCapability {
            package_builder_family: "PumpSwapQuoteRoute",
            service_adapter: "simulate_only_bridge_decision",
        },
        Pool::StonkFun | Pool::StonkFunSwap => BridgeSdkCapability {
            package_builder_family: "StonkFunQuoteRoute",
            service_adapter: "explicit_via_sol_request",
        },
        Pool::RaydiumAmmV4 => BridgeSdkCapability {
            package_builder_family: "StonkFunQuoteRoute",
            service_adapter: "cpmm_amm_v4_simulate_only",
        },
        Pool::RaydiumCpmm => BridgeSdkCapability {
            package_builder_family: "StonkFunQuoteRoute",
            service_adapter: "cpmm_cpmm_simulate_only",
        },
        _ => BridgeSdkCapability {
            package_builder_family: "not_fully_evaluated",
            service_adapter: "not_integrated",
        },
    }
}

/// Builder selected by the service's verified direct trade decision. This
/// describes the existing SDK path, not package-wide route coverage or an
/// aggregator fallback decision.
pub(crate) fn direct_sdk_capability(pool: Pool, via_sol: bool) -> BridgeSdkCapability {
    let package_builder_family = match (pool, via_sol) {
        (Pool::Pump, false) => "PumpFunParams",
        (Pool::PumpAmm, false) => "PumpSwapParams",
        (Pool::Bonk, false) => "BonkParams",
        (Pool::StonkFun | Pool::StonkFunSwap, true) => "StonkFunViaSolParams",
        (Pool::StonkFun | Pool::StonkFunSwap, false) => "StonkFunParams",
        (Pool::MeteoraDammV2, false) => "MeteoraDammV2Params",
        (Pool::RaydiumAmmV4, false) => "RaydiumAmmV4Params",
        (Pool::RaydiumCpmm, false) => "RaydiumCpmmParams",
        (Pool::RaydiumCpmm, true) => "StonkFunQuoteRoute",
        _ => "not_integrated",
    };
    BridgeSdkCapability {
        package_builder_family,
        service_adapter: if via_sol {
            "verified_via_sol_sdk"
        } else {
            "verified_direct_sdk"
        },
    }
}

pub(crate) fn direct_sdk_buildability(
    pool: Pool,
    side: Side,
    quote_mint: &Pubkey,
    pool_id: Option<&Pubkey>,
    sol_hop_pool: Option<Pool>,
    sol_hop_pool_id: Option<&Pubkey>,
) -> DirectSdkBuildability {
    if pool == Pool::MeteoraDbc {
        return DirectSdkBuildability::Unavailable("dbc_legacy_backend_required");
    }
    if matches!(
        pool,
        Pool::StonkFun
            | Pool::StonkFunSwap
            | Pool::MeteoraDammV2
            | Pool::RaydiumAmmV4
            | Pool::RaydiumCpmm
    ) && pool_id.is_none()
    {
        return DirectSdkBuildability::Incomplete("pool_id_required");
    }
    match (sol_hop_pool, sol_hop_pool_id) {
        (None, None) => {
            let token_type = match side {
                Side::Buy => trade_token_type_for_buy(pool, quote_mint),
                Side::Sell => trade_token_type_for_sell(pool, quote_mint),
            };
            if token_type.is_err() {
                DirectSdkBuildability::Unavailable("quote_asset_not_supported_by_direct_builder")
            } else {
                DirectSdkBuildability::Available
            }
        }
        (Some(Pool::RaydiumCpmm), Some(_))
            if pool == Pool::RaydiumCpmm && !is_wsol_quote(quote_mint) =>
        {
            DirectSdkBuildability::Available
        }
        (Some(Pool::RaydiumCpmm), Some(_))
            if matches!(pool, Pool::StonkFun | Pool::StonkFunSwap)
                && !is_wsol_quote(quote_mint) =>
        {
            DirectSdkBuildability::Available
        }
        (Some(Pool::RaydiumCpmm), Some(_))
            if matches!(pool, Pool::StonkFun | Pool::StonkFunSwap) =>
        {
            DirectSdkBuildability::Incomplete("via_sol_requires_non_wsol_quote")
        }
        (Some(_), Some(_)) => DirectSdkBuildability::Unavailable("multi_hop_not_in_direct_builder"),
        _ => DirectSdkBuildability::Incomplete("sol_hop_hint_incomplete"),
    }
}

fn ensure_direct_sdk_buildability(
    pool: Pool,
    side: Side,
    quote_mint: &Pubkey,
    pool_id: Option<&Pubkey>,
    sol_hop_pool: Option<Pool>,
    sol_hop_pool_id: Option<&Pubkey>,
) -> anyhow::Result<()> {
    match direct_sdk_buildability(
        pool,
        side,
        quote_mint,
        pool_id,
        sol_hop_pool,
        sol_hop_pool_id,
    ) {
        DirectSdkBuildability::Available => Ok(()),
        DirectSdkBuildability::Unavailable(reason) => {
            bail!("direct SDK builder is unavailable for route: {reason}")
        }
        DirectSdkBuildability::Incomplete(reason) => {
            bail!("SDK route evidence is incomplete: {reason}")
        }
    }
}

fn trade_token_type_for_buy(pool: Pool, quote_mint: &Pubkey) -> anyhow::Result<TradeTokenType> {
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

fn trade_token_type_for_sell(pool: Pool, quote_mint: &Pubkey) -> anyhow::Result<TradeTokenType> {
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

fn is_wsol_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(WSOL_MINT)
}

fn is_usdc_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(USDC_MINT)
}

fn is_usd1_quote(quote_mint: &Pubkey) -> bool {
    *quote_mint == parse_known_pubkey(USD1_MINT)
}

fn parse_known_pubkey(value: &str) -> Pubkey {
    Pubkey::from_str(value).expect("known mint pubkey must be valid")
}

fn describe_bonk_param_error(
    mint: &Pubkey,
    quote_mint: &Pubkey,
    error: anyhow::Error,
) -> anyhow::Error {
    let error_text = redact_urls(&error.to_string());
    if error_text.contains("AccountNotFound") {
        let quote = if is_usd1_quote(quote_mint) {
            "USD1"
        } else {
            "WSOL"
        };
        anyhow!(
            "bonk {quote} pool not found for mint {mint}; this mint may not have an active Bonk {quote} launchpad pool, may use another quote route, or may already be migrated. SDK error: {error_text}"
        )
    } else {
        error
    }
}

fn pool_resolution_failure_kind(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("429") || lower.contains("too many requests") {
        return "rate limited".to_string();
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return "timeout".to_string();
    }
    if lower.contains("accountnotfound")
        || lower.contains("account not found")
        || lower.contains("no pool found")
    {
        return "pool not found".to_string();
    }
    "RPC unavailable".to_string()
}

fn remember_fallback_note(lane: &SdkLane, error: Option<&str>, notes: &mut Vec<String>) {
    if lane.swqos_type == SwqosType::Default {
        return;
    }
    let error = redact_urls(error.unwrap_or("unknown error"));
    notes.push(format!(
        "{} lane skipped: {error}",
        lane.swqos_type.as_str()
    ));
}

fn attach_fallback_notes(result: &mut ExecuteResult, notes: &[String]) {
    if notes.is_empty() {
        return;
    }
    let note = format!("fallback_notes=[{}]", notes.join("; "));
    result.error = match result.error.take() {
        Some(error) => Some(format!("{error}; {note}")),
        None => Some(note),
    };
}

fn is_pumpswap_buy_slippage_error(error: Option<&str>) -> bool {
    let Some(error) = error else {
        return false;
    };
    error.contains("BuySlippageBelowMinBaseAmountOut")
        || error.contains("Custom(6040)")
        || error.contains("Custom( 6040 )")
}

fn build_swqos_configs(mev: &MevConfig) -> anyhow::Result<Vec<SwqosConfig>> {
    let mut configs = Vec::new();
    push_jito_provider(&mut configs, &mev.jito)?;
    push_provider(
        &mut configs,
        "TEMPORAL",
        &mev.temporal,
        |token, region, url| SwqosConfig::Temporal(token, region, url, None),
    )?;
    push_provider(
        &mut configs,
        "FLASHBLOCK",
        &mev.flashblock,
        |token, region, url| SwqosConfig::FlashBlock(token, region, url, None),
    )?;

    if mev.blockrazor.provider.enabled {
        let token = provider_token("BLOCKRAZOR", &mev.blockrazor.provider)?;
        configs.push(SwqosConfig::BlockRazor(
            token,
            parse_region(&mev.blockrazor.provider.region)?,
            mev.blockrazor.provider.custom_url.clone(),
            Some(parse_swqos_transport(&mev.blockrazor.transport)?),
            None,
        ));
    }

    if mev.astralane.provider.enabled {
        let token = provider_token("ASTRALANE", &mev.astralane.provider)?;
        configs.push(SwqosConfig::Astralane(
            token,
            parse_region(&mev.astralane.provider.region)?,
            mev.astralane.provider.custom_url.clone(),
            Some(parse_astralane_transport(&mev.astralane.transport)?),
            None,
        ));
    }

    push_provider(
        &mut configs,
        "SPEEDLANDING",
        &mev.speedlanding,
        |token, region, url| SwqosConfig::Speedlanding(token, region, url, None),
    )?;

    Ok(configs)
}

fn push_jito_provider(
    configs: &mut Vec<SwqosConfig>,
    provider: &ProviderConfig,
) -> anyhow::Result<()> {
    if !provider.enabled {
        return Ok(());
    }
    configs.push(SwqosConfig::Jito(
        provider.auth_token.clone().unwrap_or_default(),
        parse_region(&provider.region)?,
        provider.custom_url.clone(),
        None,
    ));
    Ok(())
}

fn push_provider<F>(
    configs: &mut Vec<SwqosConfig>,
    name: &str,
    provider: &ProviderConfig,
    build: F,
) -> anyhow::Result<()>
where
    F: FnOnce(String, SwqosRegion, Option<String>) -> SwqosConfig,
{
    if !provider.enabled {
        return Ok(());
    }
    configs.push(build(
        provider_token(name, provider)?,
        parse_region(&provider.region)?,
        provider.custom_url.clone(),
    ));
    Ok(())
}

fn provider_token(name: &str, provider: &ProviderConfig) -> anyhow::Result<String> {
    provider
        .auth_token
        .clone()
        .with_context(|| format!("{name} is enabled but auth token/api key/uuid is missing"))
}

fn parse_region(value: &str) -> anyhow::Result<SwqosRegion> {
    Ok(match normalize(value).as_str() {
        "newyork" | "ny" | "nyc" => SwqosRegion::NewYork,
        "frankfurt" | "fra" => SwqosRegion::Frankfurt,
        "amsterdam" | "ams" => SwqosRegion::Amsterdam,
        "dublin" | "dub" => SwqosRegion::Dublin,
        "slc" | "saltlakecity" => SwqosRegion::SLC,
        "tokyo" | "tyo" => SwqosRegion::Tokyo,
        "singapore" | "sg" | "sin" => SwqosRegion::Singapore,
        "london" | "lon" => SwqosRegion::London,
        "losangeles" | "la" | "lax" => SwqosRegion::LosAngeles,
        "default" | "" => SwqosRegion::Default,
        _ => bail!("unsupported SWQoS region: {value}"),
    })
}

fn parse_swqos_transport(value: &str) -> anyhow::Result<SwqosTransport> {
    Ok(match normalize(value).as_str() {
        "http" | "" => SwqosTransport::Http,
        "grpc" => SwqosTransport::Grpc,
        "quic" => SwqosTransport::Quic,
        _ => bail!("unsupported SWQoS transport: {value}"),
    })
}

fn parse_astralane_transport(value: &str) -> anyhow::Result<AstralaneTransport> {
    Ok(match normalize(value).as_str() {
        "binary" | "" => AstralaneTransport::Binary,
        "plain" | "http" => AstralaneTransport::Plain,
        "quic" => AstralaneTransport::Quic,
        _ => bail!("unsupported Astralane transport: {value}"),
    })
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "")
}

fn dex_type(pool: Pool) -> anyhow::Result<DexType> {
    match pool {
        Pool::Pump => Ok(DexType::PumpFun),
        Pool::PumpAmm => Ok(DexType::PumpSwap),
        Pool::Bonk => Ok(DexType::Bonk),
        Pool::StonkFun | Pool::StonkFunSwap => Ok(DexType::StonkFun),
        Pool::MeteoraDammV2 => Ok(DexType::MeteoraDammV2),
        Pool::MeteoraDbc => bail!("meteora-dbc requires the legacy DBC backend"),
        Pool::RaydiumAmmV4 => Ok(DexType::RaydiumAmmV4),
        Pool::RaydiumCpmm => Ok(DexType::RaydiumCpmm),
    }
}

fn default_fee_strategy(swqos_type: SwqosType, tip_sol: f64) -> GasFeeStrategy {
    let strategy = GasFeeStrategy::new();
    let set_strategy = |strategy: &GasFeeStrategy, lane: SwqosType, tip: f64| {
        strategy.set(
            lane,
            TradeType::Buy,
            GasFeeStrategyType::Normal,
            300_000,
            1_000,
            tip,
        );
        strategy.set(
            lane,
            TradeType::Sell,
            GasFeeStrategyType::Normal,
            300_000,
            1_000,
            tip,
        );
    };

    // SDK 6.x always reads the Default strategy while simulating, even when
    // the transaction is being prepared for a specific SWQoS lane.
    set_strategy(&strategy, SwqosType::Default, 0.0);
    if swqos_type != SwqosType::Default {
        set_strategy(&strategy, swqos_type, tip_sol);
    }
    strategy
}

/// Keep packet/version selection identical for two-hop buy and sell. The
/// returned transaction is unsigned and cannot be sent by this helper.
#[derive(Clone)]
enum SdkTwoHopUnsignedTransaction {
    Legacy(Transaction, usize),
    V1(VersionedTransaction, usize),
}

impl SdkTwoHopUnsignedTransaction {
    /// Sign precisely the message selected for unsigned simulation. The
    /// caller must persist the resulting signature before any send attempt.
    fn into_signed_transaction(self, signer: &Keypair) -> anyhow::Result<VersionedTransaction> {
        let message = match self {
            Self::Legacy(transaction, _) => VersionedMessage::Legacy(transaction.message),
            Self::V1(transaction, _) => transaction.message,
        };
        anyhow::ensure!(
            message.header().num_required_signatures == 1
                && message.static_account_keys().first() == Some(&signer.pubkey()),
            "SDK two-hop signer does not match prepared wallet"
        );
        let signed = VersionedTransaction::try_new(message, &[signer])?;
        anyhow::ensure!(
            signed.signatures.len() == 1
                && signed.signatures[0] != Signature::default()
                && signed.signatures[0]
                    .verify(signer.pubkey().as_ref(), &signed.message.serialize()),
            "SDK two-hop prepared signature is invalid"
        );
        Ok(signed)
    }
}

async fn send_signed_sdk_transaction_once(
    rpc: &solana_client::nonblocking::rpc_client::RpcClient,
    wallet: Pubkey,
    transaction: &VersionedTransaction,
    minimum_context_slot: u64,
    deadline: Duration,
) -> anyhow::Result<Signature> {
    anyhow::ensure!(minimum_context_slot > 0, "SDK send context slot is missing");
    anyhow::ensure!(
        transaction.message.header().num_required_signatures == 1
            && transaction.message.static_account_keys().first() == Some(&wallet)
            && transaction.signatures.len() == 1
            && transaction.signatures[0] != Signature::default()
            && transaction.signatures[0].verify(wallet.as_ref(), &transaction.message.serialize()),
        "prepared SDK transaction does not have the selected wallet's valid signature"
    );
    transaction
        .sanitize()
        .context("prepared SDK transaction is invalid")?;
    let packet_bytes = wincode::serialize(transaction)?.len();
    anyhow::ensure!(
        packet_bytes <= solana_sdk::message::v1::MAX_TRANSACTION_SIZE,
        "prepared SDK transaction exceeds packet limit"
    );
    let local_signature = transaction.signatures[0];
    let returned_signature = tokio::time::timeout(
        deadline,
        rpc.send_transaction_with_config(
            transaction,
            RpcSendTransactionConfig {
                skip_preflight: false,
                max_retries: Some(0),
                min_context_slot: Some(minimum_context_slot),
                encoding: Some(UiTransactionEncoding::Base64),
                ..RpcSendTransactionConfig::default()
            },
        ),
    )
    .await
    .context("prepared SDK transaction submission timed out")?
    .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
    anyhow::ensure!(
        returned_signature == local_signature,
        "prepared SDK RPC returned a different signature"
    );
    Ok(local_signature)
}

async fn observe_sdk_signature_by_original(
    rpc: &solana_client::nonblocking::rpc_client::RpcClient,
    signature: Signature,
    deadline_duration: Duration,
) -> anyhow::Result<Option<bool>> {
    let deadline = Instant::now() + deadline_duration;
    loop {
        let statuses = tokio::time::timeout(
            Duration::from_secs(3),
            rpc.get_signature_statuses_with_history(&[signature]),
        )
        .await
        .context("SDK signature status read timed out")?
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
        if let Some(status) = statuses.value.into_iter().next().flatten() {
            return Ok(Some(status.err.is_none()));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn compile_sdk_two_hop_unsigned_transaction(
    instructions: &[solana_sdk::instruction::Instruction],
    payer: &Pubkey,
    blockhash: Hash,
) -> anyhow::Result<SdkTwoHopUnsignedTransaction> {
    let mut message = Message::new(instructions, Some(payer));
    message.recent_blockhash = blockhash;
    anyhow::ensure!(
        message.header.num_required_signatures == 1 && message.account_keys.first() == Some(payer),
        "SDK two-hop transaction requires exactly the selected wallet as signer"
    );
    let legacy = Transaction::new_unsigned(message);
    let legacy_bytes = bincode::serialize(&legacy)?.len();
    if legacy_bytes <= 1232 {
        return Ok(SdkTwoHopUnsignedTransaction::Legacy(legacy, legacy_bytes));
    }

    let config = solana_sdk::message::v1::TransactionConfig::empty()
        .with_compute_unit_limit(1_400_000)
        .with_loaded_accounts_data_size_limit(64 * 1024 * 1024);
    let versioned_message = sol_trade_sdk::trading::core::transaction_pool::compile_message(
        payer,
        instructions,
        &[],
        blockhash,
        sol_trade_sdk::common::TradeTransactionVersion::V1,
        config,
    )?;
    let transaction = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: versioned_message,
    };
    anyhow::ensure!(
        transaction.message.header().num_required_signatures == 1
            && transaction.message.static_account_keys().first() == Some(payer),
        "SDK V1 two-hop transaction requires exactly the selected wallet as signer"
    );
    // V1 has its own wire format; bincode does not measure its packet size.
    let transaction_bytes = wincode::serialize(&transaction)?.len();
    Ok(SdkTwoHopUnsignedTransaction::V1(
        transaction,
        transaction_bytes,
    ))
}

async fn simulate_two_hop_instructions(
    lane: &SdkLane,
    instructions: &[solana_sdk::instruction::Instruction],
    target_mint: &Pubkey,
    target_program: &Pubkey,
    target_min: u64,
    end_slot: u64,
) -> anyhow::Result<SdkTwoHopSimulation> {
    Ok(simulate_two_hop_instructions_prepared(
        lane,
        instructions,
        target_mint,
        target_program,
        target_min,
        end_slot,
    )
    .await?
    .0)
}

async fn simulate_two_hop_instructions_prepared(
    lane: &SdkLane,
    instructions: &[solana_sdk::instruction::Instruction],
    target_mint: &Pubkey,
    target_program: &Pubkey,
    target_min: u64,
    end_slot: u64,
) -> anyhow::Result<(SdkTwoHopSimulation, SdkTwoHopUnsignedTransaction)> {
    let rpc = lane.client.get_rpc();
    let base_program = *target_program;
    let blockhash = rpc.get_latest_blockhash().await?;
    let target_ata = get_associated_token_address_with_program_id_fast(
        &lane.client.payer.pubkey(),
        target_mint,
        &base_program,
    );
    // With sig_verify=false the RPC simulates an unsigned message.
    // No private-key signing or sendTransaction occurs here.
    let compiled = compile_sdk_two_hop_unsigned_transaction(
        instructions,
        &lane.client.payer.pubkey(),
        blockhash,
    )?;
    let simulation: anyhow::Result<SdkTwoHopSimulation> = match compiled.clone() {
        SdkTwoHopUnsignedTransaction::V1(v1_transaction, v1_bytes) => {
            if v1_bytes > solana_sdk::message::v1::MAX_TRANSACTION_SIZE {
                Ok(SdkTwoHopSimulation {
                    attempted: false,
                    rpc_accepted: false,
                    ok: false,
                    error: Some("v1_transaction_packet_too_large".to_string()),
                    slot: end_slot,
                    units_consumed: None,
                    fee_lamports: None,
                    transaction_bytes: v1_bytes,
                    transaction_version: "v1",
                    payer_debit_lamports: None,
                    target_token_post_units: None,
                    target_token_delta_units: None,
                    minimum_output_verified: false,
                })
            } else {
                let result = tokio::time::timeout(
                    Duration::from_secs(12),
                    rpc.simulate_transaction_with_config(
                        &v1_transaction,
                        RpcSimulateTransactionConfig {
                            sig_verify: false,
                            replace_recent_blockhash: false,
                            commitment: Some(CommitmentConfig::processed()),
                            encoding: Some(UiTransactionEncoding::Base64),
                            accounts: Some(RpcSimulateTransactionAccountsConfig {
                                encoding: Some(UiAccountEncoding::Base64),
                                addresses: vec![target_ata.to_string()],
                            }),
                            min_context_slot: Some(end_slot),
                            inner_instructions: false,
                        },
                    ),
                )
                .await;
                Ok(match result {
                    Ok(Ok(response)) => {
                        let target_index = v1_transaction
                            .message
                            .static_account_keys()
                            .iter()
                            .position(|key| key == &target_ata)
                            .context("target ATA missing from SDK V1 message")?;
                        sdk_two_hop_simulation_result(
                            response,
                            target_index,
                            target_mint,
                            &base_program,
                            &lane.client.payer.pubkey(),
                            target_min,
                            v1_bytes,
                            "v1",
                        )?
                    }
                    Ok(Err(error)) => SdkTwoHopSimulation {
                        attempted: true,
                        rpc_accepted: false,
                        ok: false,
                        error: Some(redact_urls(&error.to_string())),
                        slot: end_slot,
                        units_consumed: None,
                        fee_lamports: None,
                        transaction_bytes: v1_bytes,
                        transaction_version: "v1",
                        payer_debit_lamports: None,
                        target_token_post_units: None,
                        target_token_delta_units: None,
                        minimum_output_verified: false,
                    },
                    Err(_) => SdkTwoHopSimulation {
                        attempted: true,
                        rpc_accepted: false,
                        ok: false,
                        error: Some("v1_simulation_timed_out".to_string()),
                        slot: end_slot,
                        units_consumed: None,
                        fee_lamports: None,
                        transaction_bytes: v1_bytes,
                        transaction_version: "v1",
                        payer_debit_lamports: None,
                        target_token_post_units: None,
                        target_token_delta_units: None,
                        minimum_output_verified: false,
                    },
                })
            }
        }
        SdkTwoHopUnsignedTransaction::Legacy(transaction, transaction_bytes) => {
            let result = tokio::time::timeout(
                Duration::from_secs(12),
                rpc.simulate_transaction_with_config(
                    &transaction,
                    RpcSimulateTransactionConfig {
                        sig_verify: false,
                        replace_recent_blockhash: false,
                        commitment: Some(CommitmentConfig::processed()),
                        encoding: Some(UiTransactionEncoding::Base64),
                        accounts: Some(RpcSimulateTransactionAccountsConfig {
                            encoding: Some(UiAccountEncoding::Base64),
                            addresses: vec![target_ata.to_string()],
                        }),
                        min_context_slot: Some(end_slot),
                        inner_instructions: false,
                    },
                ),
            )
            .await
            .context("SDK two-hop simulation timed out")?
            .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
            let target_index = transaction
                .message
                .account_keys
                .iter()
                .position(|key| key == &target_ata)
                .context("target ATA missing from SDK legacy message")?;
            Ok(sdk_two_hop_simulation_result(
                result,
                target_index,
                target_mint,
                &base_program,
                &lane.client.payer.pubkey(),
                target_min,
                transaction_bytes,
                "legacy",
            )?)
        }
    };
    let simulation = simulation?;
    Ok((simulation, compiled))
}

fn sdk_two_hop_sell_simulation_result(
    response: solana_client::rpc_response::Response<
        solana_client::rpc_response::RpcSimulateTransactionResult,
    >,
    input_index: usize,
    wsol_index: usize,
    input_mint: &Pubkey,
    input_program: &Pubkey,
    payer: &Pubkey,
    expected_input_balance: u64,
    input_units: u64,
    minimum_wsol_units: u64,
    transaction_bytes: usize,
    transaction_version: &'static str,
) -> anyhow::Result<SdkTwoHopSellSimulation> {
    let value = response.value;
    let input_index = u8::try_from(input_index).context("sell input ATA index exceeds u8")?;
    let wsol_index = u8::try_from(wsol_index).context("sell WSOL ATA index exceeds u8")?;
    let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
    let pre_input = value
        .pre_token_balances
        .as_ref()
        .and_then(|balances| target_token_amount(balances, input_index, input_mint));
    let post_input = value
        .post_token_balances
        .as_ref()
        .and_then(|balances| target_token_amount(balances, input_index, input_mint));
    let pre_wsol = value
        .pre_token_balances
        .as_ref()
        .and_then(|balances| target_token_amount(balances, wsol_index, &wsol))
        .or_else(|| {
            (value
                .pre_balances
                .as_ref()
                .and_then(|balances| balances.get(wsol_index as usize))
                == Some(&0))
            .then_some(0)
        });
    let post_wsol = value
        .post_token_balances
        .as_ref()
        .and_then(|balances| target_token_amount(balances, wsol_index, &wsol));
    let input_token_debit_units = pre_input
        .zip(post_input)
        .and_then(|(before, after)| before.checked_sub(after));
    let wsol_credit_units = pre_wsol
        .zip(post_wsol)
        .and_then(|(before, after)| after.checked_sub(before));
    let post_input_account = value
        .accounts
        .as_ref()
        .and_then(|accounts| accounts.first())
        .and_then(Option::as_ref)
        .map(|account| simulated_target_account_amount(account, input_program, input_mint, payer))
        .transpose()?;
    let post_wsol_account = value
        .accounts
        .as_ref()
        .and_then(|accounts| accounts.get(1))
        .and_then(Option::as_ref)
        .map(|account| simulated_target_account_amount(account, &spl_token::ID, &wsol, payer))
        .transpose()?;
    let balance_and_minimum_verified = sell_balance_and_minimum_verified(
        value.err.is_none(),
        pre_input,
        post_input,
        post_input_account,
        pre_wsol,
        post_wsol,
        post_wsol_account,
        expected_input_balance,
        input_units,
        minimum_wsol_units,
    );
    Ok(SdkTwoHopSellSimulation {
        attempted: true,
        rpc_accepted: true,
        ok: value.err.is_none(),
        error: value.err.map(|error| redact_urls(&error.to_string())),
        slot: response.context.slot,
        units_consumed: value.units_consumed,
        transaction_bytes,
        transaction_version,
        input_token_debit_units,
        wsol_credit_units,
        balance_and_minimum_verified,
    })
}

async fn simulate_two_hop_sell_instructions(
    lane: &SdkLane,
    instructions: &[solana_sdk::instruction::Instruction],
    input_mint: &Pubkey,
    input_program: &Pubkey,
    input_ata: &Pubkey,
    expected_input_balance: u64,
    input_units: u64,
    minimum_wsol_units: u64,
    end_slot: u64,
) -> anyhow::Result<(SdkTwoHopSellSimulation, SdkTwoHopUnsignedTransaction)> {
    let rpc = lane.client.get_rpc();
    let payer = lane.client.payer.pubkey();
    let wsol_ata = get_associated_token_address_with_program_id_fast(
        &payer,
        &sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT,
        &spl_token::ID,
    );
    let blockhash = rpc.get_latest_blockhash().await?;
    let compiled = compile_sdk_two_hop_unsigned_transaction(instructions, &payer, blockhash)?;
    let addresses = vec![input_ata.to_string(), wsol_ata.to_string()];
    if let SdkTwoHopUnsignedTransaction::Legacy(legacy, legacy_bytes) = &compiled {
        let simulation_config = RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: false,
            commitment: Some(CommitmentConfig::processed()),
            encoding: Some(UiTransactionEncoding::Base64),
            accounts: Some(RpcSimulateTransactionAccountsConfig {
                encoding: Some(UiAccountEncoding::Base64),
                addresses,
            }),
            min_context_slot: Some(end_slot),
            inner_instructions: false,
        };
        let mut context_retries = 0;
        let result = loop {
            let response = tokio::time::timeout(
                Duration::from_secs(12),
                rpc.simulate_transaction_with_config(legacy, simulation_config.clone()),
            )
            .await
            .context("SDK sell simulation timed out")?;
            match response {
                Ok(result) => break result,
                Err(error)
                    if context_retries < 3
                        && error
                            .to_string()
                            .contains("Minimum context slot has not been reached") =>
                {
                    context_retries += 1;
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    continue;
                }
                Err(error) => return Err(anyhow::anyhow!("{}", redact_urls(&error.to_string()))),
            }
        };
        let input_index = legacy
            .message
            .account_keys
            .iter()
            .position(|key| key == input_ata)
            .context("sell input ATA missing from legacy message")?;
        let wsol_index = legacy
            .message
            .account_keys
            .iter()
            .position(|key| key == &wsol_ata)
            .context("sell WSOL ATA missing from legacy message")?;
        let simulation = sdk_two_hop_sell_simulation_result(
            result,
            input_index,
            wsol_index,
            input_mint,
            input_program,
            &payer,
            expected_input_balance,
            input_units,
            minimum_wsol_units,
            *legacy_bytes,
            "legacy",
        )?;
        return Ok((simulation, compiled));
    }
    let SdkTwoHopUnsignedTransaction::V1(transaction, transaction_bytes) = &compiled else {
        unreachable!("legacy transaction returned above")
    };
    if *transaction_bytes > solana_sdk::message::v1::MAX_TRANSACTION_SIZE {
        return Ok((
            SdkTwoHopSellSimulation {
                attempted: false,
                rpc_accepted: false,
                ok: false,
                error: Some("v1_transaction_packet_too_large".to_string()),
                slot: end_slot,
                units_consumed: None,
                transaction_bytes: *transaction_bytes,
                transaction_version: "v1",
                input_token_debit_units: None,
                wsol_credit_units: None,
                balance_and_minimum_verified: false,
            },
            compiled,
        ));
    }
    let simulation_config = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: false,
        commitment: Some(CommitmentConfig::processed()),
        encoding: Some(UiTransactionEncoding::Base64),
        accounts: Some(RpcSimulateTransactionAccountsConfig {
            encoding: Some(UiAccountEncoding::Base64),
            addresses,
        }),
        min_context_slot: Some(end_slot),
        inner_instructions: false,
    };
    let mut context_retries = 0;
    let result = loop {
        let response = tokio::time::timeout(
            Duration::from_secs(12),
            rpc.simulate_transaction_with_config(transaction, simulation_config.clone()),
        )
        .await
        .context("SDK V1 sell simulation timed out")?;
        match response {
            Ok(result) => break result,
            Err(error)
                if context_retries < 3
                    && error
                        .to_string()
                        .contains("Minimum context slot has not been reached") =>
            {
                context_retries += 1;
                tokio::time::sleep(Duration::from_millis(350)).await;
            }
            Err(error) => return Err(anyhow::anyhow!("{}", redact_urls(&error.to_string()))),
        }
    };
    let input_index = transaction
        .message
        .static_account_keys()
        .iter()
        .position(|key| key == input_ata)
        .context("sell input ATA missing from V1 message")?;
    let wsol_index = transaction
        .message
        .static_account_keys()
        .iter()
        .position(|key| key == &wsol_ata)
        .context("sell WSOL ATA missing from V1 message")?;
    let simulation = sdk_two_hop_sell_simulation_result(
        result,
        input_index,
        wsol_index,
        input_mint,
        input_program,
        &payer,
        expected_input_balance,
        input_units,
        minimum_wsol_units,
        *transaction_bytes,
        "v1",
    )?;
    Ok((simulation, compiled))
}

fn two_hop_preview_swap_params(
    lane: &SdkLane,
    protocol_params: DexParamEnum,
    target_mint: Pubkey,
    target_program: Pubkey,
    input_lamports: u64,
    slippage_bps: u64,
) -> SwapParams {
    SwapParams {
        rpc: Some(lane.client.get_rpc().clone()),
        payer: lane.client.payer.clone(),
        trade_type: TradeType::Buy,
        input_mint: sol_trade_sdk::constants::SOL_TOKEN_ACCOUNT,
        input_token_program: None,
        output_mint: target_mint,
        output_token_program: Some(target_program),
        input_amount: Some(input_lamports),
        slippage_basis_points: Some(slippage_bps),
        address_lookup_table_accounts: Vec::new(),
        recent_blockhash: None,
        wait_tx_confirmed: false,
        protocol_params,
        open_seed_optimize: false,
        swqos_clients: lane.client.infrastructure.swqos_clients.clone(),
        middleware_manager: None,
        durable_nonce: None,
        with_tip: false,
        create_input_mint_ata: true,
        close_input_mint_ata: false,
        create_output_mint_ata: true,
        close_output_mint_ata: false,
        fixed_output_amount: None,
        gas_fee_strategy: simulation_fee_strategy(),
        simulate: true,
        log_enabled: false,
        wait_for_all_submits: false,
        use_dedicated_sender_threads: false,
        sender_thread_cores: None,
        max_sender_concurrency: lane.client.max_sender_concurrency,
        effective_core_ids: lane.client.effective_core_ids.clone(),
        check_min_tip: false,
        transaction_version: lane.client.transaction_version,
        grpc_recv_us: None,
        use_exact_sol_amount: Some(true),
    }
}

fn simulation_fee_strategy() -> GasFeeStrategy {
    default_fee_strategy(SwqosType::Default, 0.0)
}

fn should_wait_lane_confirmation(swqos_type: SwqosType, wait_tx_confirmed: bool) -> bool {
    let _ = swqos_type;
    wait_tx_confirmed
}

fn map_sdk_result(
    result: anyhow::Result<(
        bool,
        Vec<Signature>,
        Option<sol_trade_sdk::swqos::common::TradeError>,
        Vec<(sol_trade_sdk::swqos::SwqosType, i64)>,
    )>,
    simulate: bool,
) -> anyhow::Result<ExecuteResult> {
    let (success, signatures, error, _) = result?;
    Ok(ExecuteResult {
        success,
        // SDK simulation returns a locally signed transaction fingerprint.
        // It is not a submitted signature and must not enter trade state.
        signature: if simulate {
            None
        } else {
            signatures.first().map(ToString::to_string)
        },
        error: error.map(|error| redact_urls(&format!("{error:?}"))),
        simulation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        amm_v4_pool_open, amm_v4_quote_direction, bridge_sdk_capability,
        calculate_sol_transfer_amount, compile_sdk_two_hop_unsigned_transaction,
        cpmm_hop_direction, cpmm_pool_open, cpmm_route_token_programs, default_fee_strategy,
        direct_sdk_buildability, is_helius_rpc, map_sdk_result, minimum_output, other_pool_mint,
        pool_resolution_failure_kind, pool_resolution_lane_order, pool_route_token_programs,
        quote_cpmm_sell_via_cpmm, quote_pumpswap_buy_with_params, simulation_fee_strategy,
        trade_token_type_for_buy, trade_token_type_for_sell, validate_amm_v4_route_mints,
        validate_bonk_pool_identity, DirectSdkBuildability, SdkTwoHopBuildPreview,
        SdkTwoHopBuyPrepared, SdkTwoHopSellBuildPreview, SdkTwoHopSellPrepared,
        SdkTwoHopSellSimulation, SdkTwoHopSimulation, SdkTwoHopUnsignedTransaction,
    };
    use crate::models::{Pool, Side, USD1_MINT, USDC_MINT, WSOL_MINT};
    use sol_trade_sdk::common::{spl_token, spl_token_2022};
    use sol_trade_sdk::instruction::utils::bonk::accounts as bonk_accounts;
    use sol_trade_sdk::instruction::utils::pumpswap::accounts::AMM_PROGRAM;
    use sol_trade_sdk::swqos::{SwqosType, TradeType};
    use sol_trade_sdk::trading::core::params::{
        PumpSwapParams, RaydiumCpmmParams, TokenTransferFee,
    };
    use sol_trade_sdk::TradeTokenType;
    use solana_sdk::hash::Hash;
    use solana_sdk::instruction::{AccountMeta, Instruction};
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::{Keypair, Signature, Signer};

    #[test]
    fn two_hop_unsigned_compiler_keeps_one_wallet_signer_and_selects_packet_version() {
        let payer = Pubkey::new_unique();
        let blockhash = Hash::new_unique();
        let instruction = Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[],
            vec![AccountMeta::new(payer, true)],
        );
        let small =
            compile_sdk_two_hop_unsigned_transaction(&[instruction], &payer, blockhash).unwrap();
        let SdkTwoHopUnsignedTransaction::Legacy(transaction, bytes) = small else {
            panic!("small two-hop message should use legacy")
        };
        assert!(bytes <= 1232);
        assert_eq!(transaction.message.header.num_required_signatures, 1);
        assert_eq!(transaction.message.account_keys.first(), Some(&payer));
        assert_eq!(transaction.message.recent_blockhash, blockhash);
        assert_eq!(transaction.signatures, vec![Signature::default()]);

        let accounts = std::iter::once(AccountMeta::new(payer, true))
            .chain((0..45).map(|_| AccountMeta::new_readonly(Pubkey::new_unique(), false)))
            .collect();
        let large = Instruction::new_with_bytes(Pubkey::new_unique(), &[], accounts);
        let compiled =
            compile_sdk_two_hop_unsigned_transaction(&[large], &payer, blockhash).unwrap();
        let SdkTwoHopUnsignedTransaction::V1(transaction, bytes) = compiled else {
            panic!("large two-hop message should select V1")
        };
        assert!(bytes > 1232);
        assert_eq!(transaction.message.header().num_required_signatures, 1);
        assert_eq!(
            transaction.message.static_account_keys().first(),
            Some(&payer)
        );
        assert_eq!(transaction.signatures, vec![Signature::default()]);
    }

    #[test]
    fn two_hop_unsigned_compiler_rejects_another_required_signer() {
        let payer = Pubkey::new_unique();
        let other_signer = Pubkey::new_unique();
        let instruction = Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[],
            vec![
                AccountMeta::new(payer, true),
                AccountMeta::new(other_signer, true),
            ],
        );
        assert!(compile_sdk_two_hop_unsigned_transaction(
            &[instruction],
            &payer,
            Hash::new_unique(),
        )
        .is_err());
    }

    #[test]
    fn two_hop_signed_transaction_preserves_the_prepared_message() {
        let payer = Keypair::new();
        let blockhash = Hash::new_unique();
        for extra_accounts in [0, 45] {
            let accounts = std::iter::once(AccountMeta::new(payer.pubkey(), true))
                .chain(
                    (0..extra_accounts)
                        .map(|_| AccountMeta::new_readonly(Pubkey::new_unique(), false)),
                )
                .collect();
            let instruction = Instruction::new_with_bytes(Pubkey::new_unique(), &[], accounts);
            let prepared = compile_sdk_two_hop_unsigned_transaction(
                &[instruction],
                &payer.pubkey(),
                blockhash,
            )
            .unwrap();
            let prepared_message = match &prepared {
                SdkTwoHopUnsignedTransaction::Legacy(transaction, _) => {
                    solana_sdk::message::VersionedMessage::Legacy(transaction.message.clone())
                }
                SdkTwoHopUnsignedTransaction::V1(transaction, _) => transaction.message.clone(),
            };
            let signed = prepared.into_signed_transaction(&payer).unwrap();
            assert_eq!(signed.message.serialize(), prepared_message.serialize());
            assert_eq!(signed.signatures.len(), 1);
            assert!(signed.signatures[0]
                .verify(payer.pubkey().as_ref(), &prepared_message.serialize(),));
        }
    }

    #[test]
    fn two_hop_signed_transaction_rejects_a_different_wallet() {
        let payer = Keypair::new();
        let other = Keypair::new();
        let instruction = Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[],
            vec![AccountMeta::new(payer.pubkey(), true)],
        );
        let prepared = compile_sdk_two_hop_unsigned_transaction(
            &[instruction],
            &payer.pubkey(),
            Hash::new_unique(),
        )
        .unwrap();
        assert!(prepared.into_signed_transaction(&other).is_err());
    }

    #[test]
    fn prepared_cpmm_sell_signs_only_the_simulated_message_and_verified_balances() {
        let payer = Keypair::new();
        let instruction = Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[],
            vec![AccountMeta::new(payer.pubkey(), true)],
        );
        let build = || {
            let unsigned = compile_sdk_two_hop_unsigned_transaction(
                std::slice::from_ref(&instruction),
                &payer.pubkey(),
                Hash::new_unique(),
            )
            .unwrap();
            let message = match &unsigned {
                SdkTwoHopUnsignedTransaction::Legacy(tx, _) => {
                    solana_sdk::message::VersionedMessage::Legacy(tx.message.clone())
                }
                SdkTwoHopUnsignedTransaction::V1(tx, _) => tx.message.clone(),
            };
            let preview = SdkTwoHopSellBuildPreview {
                builder: "test",
                input_token_units: 100,
                input_token_ata: Pubkey::new_unique().to_string(),
                input_token_ata_balance_units: 100,
                target_min_quote_units: 10,
                bridge_min_wsol_units: 20,
                instruction_count: 1,
                program_ids: vec![],
                validated_slot: 1,
                snapshot_slot_span: 0,
                simulation: Some(SdkTwoHopSellSimulation {
                    attempted: true,
                    rpc_accepted: true,
                    ok: true,
                    error: None,
                    slot: 1,
                    units_consumed: Some(1),
                    transaction_bytes: 200,
                    transaction_version: "legacy",
                    input_token_debit_units: Some(100),
                    wsol_credit_units: Some(20),
                    balance_and_minimum_verified: true,
                }),
                executable: false,
            };
            (
                SdkTwoHopSellPrepared {
                    preview,
                    unsigned: Some(unsigned),
                },
                message,
            )
        };
        let (prepared, original_message) = build();
        let signed = prepared.into_signed_transaction(&payer).unwrap();
        assert_eq!(signed.message.serialize(), original_message.serialize());

        let (mut missing_credit, _) = build();
        missing_credit
            .preview
            .simulation
            .as_mut()
            .unwrap()
            .wsol_credit_units = Some(19);
        assert!(missing_credit.into_signed_transaction(&payer).is_err());
        let (mut missing_debit, _) = build();
        missing_debit
            .preview
            .simulation
            .as_mut()
            .unwrap()
            .input_token_debit_units = None;
        assert!(missing_debit.into_signed_transaction(&payer).is_err());
        let (mut rpc_failed, _) = build();
        rpc_failed.preview.simulation.as_mut().unwrap().ok = false;
        assert!(rpc_failed.into_signed_transaction(&payer).is_err());
    }

    #[test]
    fn prepared_cpmm_buy_signs_only_the_simulated_message_and_verified_output() {
        let payer = Keypair::new();
        let instruction = Instruction::new_with_bytes(
            Pubkey::new_unique(),
            &[],
            vec![AccountMeta::new(payer.pubkey(), true)],
        );
        let build = || {
            let unsigned = compile_sdk_two_hop_unsigned_transaction(
                std::slice::from_ref(&instruction),
                &payer.pubkey(),
                Hash::new_unique(),
            )
            .unwrap();
            let message = match &unsigned {
                SdkTwoHopUnsignedTransaction::Legacy(tx, _) => {
                    solana_sdk::message::VersionedMessage::Legacy(tx.message.clone())
                }
                SdkTwoHopUnsignedTransaction::V1(tx, _) => tx.message.clone(),
            };
            let preview = SdkTwoHopBuildPreview {
                builder: "test",
                input_lamports: 100,
                bridge_min_output_units: 40,
                target_min_output_units: 20,
                instruction_count: 1,
                program_ids: vec![],
                validated_slot: 1,
                snapshot_slot_span: 0,
                simulation: Some(SdkTwoHopSimulation {
                    attempted: true,
                    rpc_accepted: true,
                    ok: true,
                    error: None,
                    slot: 1,
                    units_consumed: Some(1),
                    fee_lamports: Some(1),
                    transaction_bytes: 200,
                    transaction_version: "legacy",
                    payer_debit_lamports: Some(100),
                    target_token_post_units: Some(30),
                    target_token_delta_units: Some(20),
                    minimum_output_verified: true,
                }),
            };
            (
                SdkTwoHopBuyPrepared {
                    preview,
                    unsigned: Some(unsigned),
                },
                message,
            )
        };
        let (prepared, original_message) = build();
        let signed = prepared.into_signed_transaction(&payer).unwrap();
        assert_eq!(signed.message.serialize(), original_message.serialize());

        let (mut low_output, _) = build();
        low_output
            .preview
            .simulation
            .as_mut()
            .unwrap()
            .target_token_delta_units = Some(19);
        assert!(low_output.into_signed_transaction(&payer).is_err());
        let (mut low_debit, _) = build();
        low_debit
            .preview
            .simulation
            .as_mut()
            .unwrap()
            .payer_debit_lamports = Some(99);
        assert!(low_debit.into_signed_transaction(&payer).is_err());
        let (prepared, _) = build();
        assert!(prepared.into_signed_transaction(&Keypair::new()).is_err());
    }

    #[test]
    fn cpmm_swap_requires_open_time_and_enabled_status() {
        assert!(!cpmm_pool_open(0, 101, 100));
        assert!(cpmm_pool_open(0, 100, 100));
        assert!(cpmm_pool_open(1, 100, 101));
        assert!(!cpmm_pool_open(4, 100, 101));
        assert!(!cpmm_pool_open(5, 100, 101));
    }

    #[test]
    fn cpmm_two_hop_direction_checks_mints_programs_and_output_fee() {
        let wsol: Pubkey = WSOL_MINT.parse().unwrap();
        let usdc: Pubkey = USDC_MINT.parse().unwrap();
        let target = Pubkey::new_unique();
        let mut bridge = RaydiumCpmmParams::from_trade(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            wsol,
            usdc,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            spl_token::ID,
            spl_token_2022::ID,
            Pubkey::new_unique(),
            1_000_000,
            1_000_000,
        );
        bridge.quote_transfer_fee = TokenTransferFee {
            basis_points: 25,
            maximum_fee: 100,
        };
        let (forward, input_program, output_program, fee) =
            cpmm_hop_direction(&bridge, &wsol, &usdc).unwrap();
        assert!(forward);
        assert_eq!(input_program, spl_token::ID);
        assert_eq!(output_program, spl_token_2022::ID);
        assert_eq!(fee.basis_points, 25);
        let (reverse, input_program, output_program, fee) =
            cpmm_hop_direction(&bridge, &usdc, &wsol).unwrap();
        assert!(!reverse);
        assert_eq!(input_program, spl_token_2022::ID);
        assert_eq!(output_program, spl_token::ID);
        assert_eq!(fee.basis_points, 0);
        assert!(cpmm_hop_direction(&bridge, &usdc, &target).is_err());
        bridge.quote_token_program = Pubkey::new_unique();
        assert!(cpmm_hop_direction(&bridge, &wsol, &usdc).is_err());
    }

    #[test]
    fn cpmm_two_hop_sell_quotes_independent_reverse_order() {
        let wsol: Pubkey = WSOL_MINT.parse().unwrap();
        let usdc: Pubkey = USDC_MINT.parse().unwrap();
        let target_mint = Pubkey::new_unique();
        let target = RaydiumCpmmParams::from_trade(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            target_mint,
            usdc,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            spl_token::ID,
            spl_token::ID,
            Pubkey::new_unique(),
            5_000_000_000,
            5_000_000_000,
        );
        let bridge = RaydiumCpmmParams::from_trade(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            wsol,
            usdc,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            spl_token::ID,
            spl_token::ID,
            Pubkey::new_unique(),
            5_000_000_000,
            5_000_000_000,
        );
        let (route, quote_min, wsol_min, input_program) = quote_cpmm_sell_via_cpmm(
            target.clone(),
            bridge.clone(),
            &target_mint,
            &usdc,
            1_000_000,
            100,
        )
        .unwrap();
        assert!(route.buy_hops.is_empty());
        assert_eq!(route.sell_hops.len(), 2);
        assert_eq!(route.sell_hops[0].input_mint, target_mint);
        assert_eq!(route.sell_hops[0].output_mint, usdc);
        assert_eq!(route.sell_hops[1].input_mint, usdc);
        assert_eq!(route.sell_hops[1].output_mint, wsol);
        assert_eq!(route.sell_hops[0].minimum_amount_out, quote_min);
        assert_eq!(route.sell_hops[1].minimum_amount_out, wsol_min);
        assert!(quote_min > 0 && wsol_min > 0);
        assert_eq!(input_program, spl_token::ID);
        assert!(quote_cpmm_sell_via_cpmm(
            target.clone(),
            bridge.clone(),
            &target_mint,
            &usdc,
            0,
            100
        )
        .is_err());
        assert!(quote_cpmm_sell_via_cpmm(
            target.clone(),
            bridge.clone(),
            &target_mint,
            &usdc,
            1_000_000,
            10_000
        )
        .is_err());
        let mut mismatched_bridge = bridge;
        mismatched_bridge.quote_token_program = spl_token_2022::ID;
        assert!(quote_cpmm_sell_via_cpmm(
            target,
            mismatched_bridge,
            &target_mint,
            &usdc,
            1_000_000,
            100
        )
        .is_err());
    }

    #[test]
    fn selected_two_sided_pool_discovers_only_the_other_mint() {
        let target = Pubkey::new_unique();
        let quote = Pubkey::new_unique();
        let unrelated = Pubkey::new_unique();
        assert_eq!(other_pool_mint(&target, &target, &quote).unwrap(), quote);
        assert_eq!(other_pool_mint(&quote, &target, &quote).unwrap(), target);
        assert!(other_pool_mint(&unrelated, &target, &quote).is_err());
        assert!(other_pool_mint(&target, &target, &target).is_err());
    }

    #[test]
    fn pumpswap_quote_requires_matching_mints_and_known_zero_transfer_fees() {
        let base = Pubkey::new_unique();
        let quote = Pubkey::new_unique();
        let creator = Pubkey::default();
        let vault_authority =
            Pubkey::find_program_address(&[b"creator_vault", &creator.to_bytes()], &AMM_PROGRAM).0;
        let vault_ata =
            sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &vault_authority,
                &quote,
                &sol_trade_sdk::constants::TOKEN_PROGRAM,
            );
        let mut params = PumpSwapParams::new(
            Pubkey::new_unique(),
            base,
            quote,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            1_000_000_000,
            2_000_000_000,
            0,
            vault_ata,
            vault_authority,
            sol_trade_sdk::constants::TOKEN_PROGRAM,
            sol_trade_sdk::constants::TOKEN_PROGRAM,
            Pubkey::new_unique(),
            creator,
            false,
            0,
        );
        let zero_fee = TokenTransferFee::default();
        let (amount_out, fee_units) =
            quote_pumpswap_buy_with_params(&params, &quote, &base, 1_000_000, zero_fee, zero_fee)
                .unwrap();
        assert!(amount_out > 0 && amount_out < params.pool_base_token_reserves);
        assert!(fee_units > 0 && fee_units < 1_000_000);
        assert!(quote_pumpswap_buy_with_params(
            &params, &base, &quote, 1_000_000, zero_fee, zero_fee
        )
        .is_err());
        assert!(
            quote_pumpswap_buy_with_params(&params, &quote, &base, 0, zero_fee, zero_fee).is_err()
        );
        let charged = TokenTransferFee {
            basis_points: 100,
            maximum_fee: 1_000,
        };
        assert!(quote_pumpswap_buy_with_params(
            &params, &quote, &base, 1_000_000, zero_fee, charged
        )
        .is_err());
        params.is_cashback_coin = true;
        assert!(quote_pumpswap_buy_with_params(
            &params, &quote, &base, 1_000_000, zero_fee, zero_fee
        )
        .is_err());
    }

    #[test]
    fn sdk_simulation_signature_stays_inside_the_sdk_boundary() {
        let signature = Signature::new_unique();
        let simulated =
            map_sdk_result(Ok((true, vec![signature.clone()], None, Vec::new())), true).unwrap();
        assert!(simulated.signature.is_none());
        let submitted =
            map_sdk_result(Ok((true, vec![signature], None, Vec::new())), false).unwrap();
        let expected = signature.to_string();
        assert_eq!(submitted.signature.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn bridge_capability_keeps_package_builder_separate_from_service_adapter() {
        let pump_curve = bridge_sdk_capability(Pool::Pump);
        assert_eq!(pump_curve.package_builder_family, "PumpFunQuoteRoute");
        assert_eq!(pump_curve.service_adapter, "not_integrated");
        let pump = bridge_sdk_capability(Pool::PumpAmm);
        assert_eq!(pump.package_builder_family, "PumpSwapQuoteRoute");
        assert_eq!(pump.service_adapter, "simulate_only_bridge_decision");
        let stonk = bridge_sdk_capability(Pool::StonkFun);
        assert_eq!(stonk.package_builder_family, "StonkFunQuoteRoute");
        assert_eq!(stonk.service_adapter, "explicit_via_sol_request");
        let amm_v4 = bridge_sdk_capability(Pool::RaydiumAmmV4);
        assert_eq!(amm_v4.package_builder_family, "StonkFunQuoteRoute");
        assert_eq!(amm_v4.service_adapter, "cpmm_amm_v4_simulate_only");
        let cpmm = bridge_sdk_capability(Pool::RaydiumCpmm);
        assert_eq!(cpmm.package_builder_family, "StonkFunQuoteRoute");
        assert_eq!(cpmm.service_adapter, "cpmm_cpmm_simulate_only");
        let other = bridge_sdk_capability(Pool::MeteoraDammV2);
        assert_eq!(other.package_builder_family, "not_fully_evaluated");
    }

    #[test]
    fn direct_builder_covers_registered_single_pool_paths_in_both_directions() {
        let wsol: Pubkey = WSOL_MINT.parse().unwrap();
        let pool_id = Pubkey::new_unique();
        for pool in [
            Pool::Pump,
            Pool::PumpAmm,
            Pool::Bonk,
            Pool::StonkFun,
            Pool::StonkFunSwap,
            Pool::MeteoraDammV2,
            Pool::RaydiumAmmV4,
            Pool::RaydiumCpmm,
        ] {
            let id = if matches!(pool, Pool::Pump | Pool::PumpAmm | Pool::Bonk) {
                None
            } else {
                Some(&pool_id)
            };
            for side in [Side::Buy, Side::Sell] {
                assert_eq!(
                    direct_sdk_buildability(pool, side, &wsol, id, None, None),
                    DirectSdkBuildability::Available,
                    "{pool:?} {side:?}"
                );
            }
        }
    }

    #[test]
    fn direct_builder_distinguishes_current_integration_from_missing_route_evidence() {
        let wsol: Pubkey = WSOL_MINT.parse().unwrap();
        let usdc: Pubkey = USDC_MINT.parse().unwrap();
        let stock = Pubkey::new_unique();
        let target = Pubkey::new_unique();
        let hop = Pubkey::new_unique();
        for side in [Side::Buy, Side::Sell] {
            assert_eq!(
                direct_sdk_buildability(
                    Pool::StonkFun,
                    side,
                    &stock,
                    Some(&target),
                    Some(Pool::RaydiumCpmm),
                    Some(&hop),
                ),
                DirectSdkBuildability::Available
            );
            assert_eq!(
                direct_sdk_buildability(
                    Pool::RaydiumCpmm,
                    side,
                    &usdc,
                    Some(&target),
                    Some(Pool::RaydiumCpmm),
                    Some(&hop),
                ),
                DirectSdkBuildability::Available
            );
        }
        assert_eq!(
            direct_sdk_buildability(
                Pool::StonkFun,
                Side::Buy,
                &stock,
                None,
                Some(Pool::RaydiumCpmm),
                Some(&hop),
            ),
            DirectSdkBuildability::Incomplete("pool_id_required")
        );
        assert_eq!(
            direct_sdk_buildability(
                Pool::StonkFun,
                Side::Buy,
                &stock,
                Some(&target),
                Some(Pool::RaydiumCpmm),
                None,
            ),
            DirectSdkBuildability::Incomplete("sol_hop_hint_incomplete")
        );
        assert_eq!(
            direct_sdk_buildability(
                Pool::StonkFun,
                Side::Buy,
                &wsol,
                Some(&target),
                Some(Pool::RaydiumCpmm),
                Some(&hop),
            ),
            DirectSdkBuildability::Incomplete("via_sol_requires_non_wsol_quote")
        );
        // These are unavailable in this service's direct builder. They are
        // not evidence that the SDK package or another adapter lacks a route.
        assert_eq!(
            direct_sdk_buildability(Pool::Bonk, Side::Buy, &usdc, None, None, None),
            DirectSdkBuildability::Unavailable("quote_asset_not_supported_by_direct_builder")
        );
        assert_eq!(
            direct_sdk_buildability(
                Pool::PumpAmm,
                Side::Buy,
                &stock,
                None,
                Some(Pool::RaydiumCpmm),
                Some(&hop),
            ),
            DirectSdkBuildability::Unavailable("multi_hop_not_in_direct_builder")
        );
    }

    #[test]
    fn recognizes_only_helius_rpc_hosts() {
        assert!(is_helius_rpc(
            "https://mainnet.helius-rpc.com/?api-key=secret"
        ));
        assert!(!is_helius_rpc("https://helius-rpc.com.example.org/"));
        assert!(!is_helius_rpc("https://mainnet.example.org/helius-rpc.com"));
    }

    #[test]
    fn pool_resolution_tries_one_random_helius_then_the_other() {
        let lanes = [true, false, true, false];
        assert_eq!(pool_resolution_lane_order(&lanes, false), [0, 2, 1, 3]);
        assert_eq!(pool_resolution_lane_order(&lanes, true), [2, 0, 1, 3]);
        assert_eq!(pool_resolution_lane_order(&[true, false], true), [0, 1]);
        assert_eq!(pool_resolution_lane_order(&[false, false], true), [0, 1]);
    }

    #[test]
    fn cpmm_custom_quote_uses_token_input_and_output() {
        let quote = Pubkey::new_unique();
        assert_eq!(
            trade_token_type_for_buy(Pool::RaydiumCpmm, &quote).unwrap(),
            TradeTokenType::Token(quote)
        );
        assert_eq!(
            trade_token_type_for_sell(Pool::RaydiumCpmm, &quote).unwrap(),
            TradeTokenType::Token(quote)
        );
    }

    #[test]
    fn cpmm_route_verification_checks_both_mints_and_token_programs() {
        let base = Pubkey::new_unique();
        let quote = Pubkey::new_unique();
        let unknown = Pubkey::new_unique();
        assert_eq!(
            cpmm_route_token_programs(
                &base,
                &quote,
                &base,
                &quote,
                &spl_token_2022::ID,
                &spl_token::ID,
            )
            .unwrap(),
            (spl_token_2022::ID, spl_token::ID)
        );
        assert_eq!(
            cpmm_route_token_programs(
                &quote,
                &base,
                &base,
                &quote,
                &spl_token_2022::ID,
                &spl_token::ID,
            )
            .unwrap(),
            (spl_token::ID, spl_token_2022::ID)
        );
        assert!(cpmm_route_token_programs(
            &base,
            &unknown,
            &base,
            &quote,
            &spl_token_2022::ID,
            &spl_token::ID,
        )
        .is_err());
        assert!(cpmm_route_token_programs(
            &base,
            &base,
            &base,
            &quote,
            &spl_token_2022::ID,
            &spl_token::ID,
        )
        .is_err());
        assert!(
            cpmm_route_token_programs(&base, &quote, &base, &quote, &unknown, &spl_token::ID,)
                .is_err()
        );
    }

    #[test]
    fn bonk_route_identity_rejects_wrong_quote_pool_or_stonkfun_platform() {
        let mint = Pubkey::new_unique();
        let wsol: Pubkey = WSOL_MINT.parse().unwrap();
        let usd1: Pubkey = USD1_MINT.parse().unwrap();
        let other = Pubkey::new_unique();
        let bonk_platform = Pubkey::new_unique();
        assert!(validate_bonk_pool_identity(
            &mint,
            &wsol,
            &mint,
            &wsol,
            &bonk_accounts::GLOBAL_CONFIG,
            &bonk_platform,
        )
        .is_ok());
        assert!(validate_bonk_pool_identity(
            &mint,
            &usd1,
            &mint,
            &usd1,
            &bonk_accounts::USD1_GLOBAL_CONFIG,
            &bonk_platform,
        )
        .is_ok());
        for (quote, pool_quote, global, platform) in [
            (other, other, bonk_accounts::GLOBAL_CONFIG, bonk_platform),
            (wsol, usd1, bonk_accounts::GLOBAL_CONFIG, bonk_platform),
            (wsol, wsol, bonk_accounts::USD1_GLOBAL_CONFIG, bonk_platform),
            (
                wsol,
                wsol,
                bonk_accounts::GLOBAL_CONFIG,
                bonk_accounts::STONKFUN_STANDARD_PLATFORM_CONFIG,
            ),
        ] {
            assert!(validate_bonk_pool_identity(
                &mint,
                &quote,
                &mint,
                &pool_quote,
                &global,
                &platform,
            )
            .is_err());
        }
    }

    #[test]
    fn damm_v2_route_orients_token_programs_and_rejects_wrong_mints() {
        let token_a = Pubkey::new_unique();
        let token_b = Pubkey::new_unique();
        let unknown = Pubkey::new_unique();
        assert_eq!(
            pool_route_token_programs(
                "Meteora DAMM v2",
                &token_b,
                &token_a,
                &token_a,
                &token_b,
                &spl_token::ID,
                &spl_token_2022::ID,
            )
            .unwrap(),
            (spl_token_2022::ID, spl_token::ID)
        );
        assert!(pool_route_token_programs(
            "Meteora DAMM v2",
            &unknown,
            &token_a,
            &token_a,
            &token_b,
            &spl_token::ID,
            &spl_token_2022::ID,
        )
        .is_err());
    }

    #[test]
    fn amm_v4_route_accepts_both_directions_but_rejects_unrelated_mints() {
        let coin = Pubkey::new_unique();
        let pc = Pubkey::new_unique();
        let unrelated = Pubkey::new_unique();
        assert!(validate_amm_v4_route_mints(&coin, &pc, &coin, &pc).is_ok());
        assert!(validate_amm_v4_route_mints(&pc, &coin, &coin, &pc).is_ok());
        assert!(validate_amm_v4_route_mints(&unrelated, &pc, &coin, &pc).is_err());
        assert!(validate_amm_v4_route_mints(&coin, &coin, &coin, &pc).is_err());
        assert_eq!(
            amm_v4_quote_direction(&coin, &pc, &coin, &pc).unwrap(),
            true
        );
        assert_eq!(
            amm_v4_quote_direction(&pc, &coin, &coin, &pc).unwrap(),
            false
        );
        assert!(amm_v4_quote_direction(&unrelated, &pc, &coin, &pc).is_err());
        assert!(amm_v4_quote_direction(&coin, &coin, &coin, &pc).is_err());
    }

    #[test]
    fn amm_v4_quote_requires_open_pool() {
        assert!(!amm_v4_pool_open(0, 0, 100));
        assert!(!amm_v4_pool_open(7, 101, 100));
        assert!(amm_v4_pool_open(7, 100, 100));
        assert!(amm_v4_pool_open(6, 101, 100));
    }

    #[test]
    fn damm_simulation_has_default_fee_without_changing_temporal_submit_fee() {
        let simulation = simulation_fee_strategy().get_strategies(TradeType::Sell);
        assert!(simulation
            .iter()
            .any(|(lane, _, value)| { *lane == SwqosType::Default && value.tip == 0.0 }));
        let submit =
            default_fee_strategy(SwqosType::Temporal, 0.0001).get_strategies(TradeType::Sell);
        assert!(submit
            .iter()
            .any(|(lane, _, value)| { *lane == SwqosType::Temporal && value.tip == 0.0001 }));
    }

    #[test]
    fn damm_v2_minimum_output_uses_raw_units_and_rejects_dust() {
        assert_eq!(minimum_output(1_000_000, 1_500).unwrap(), 850_000);
        assert_eq!(minimum_output(101, 100).unwrap(), 99);
        assert!(minimum_output(1, 1_500).is_err());
        assert!(minimum_output(0, 0).is_err());
        assert!(minimum_output(100, 10_000).is_err());
    }

    #[test]
    fn classifies_pool_resolution_failures_without_exposing_details() {
        assert_eq!(
            pool_resolution_failure_kind(
                "HTTP status 429 Too Many Requests for https://rpc.example/key"
            ),
            "rate limited"
        );
        assert_eq!(
            pool_resolution_failure_kind("AccountNotFound; No pool found for mint"),
            "pool not found"
        );
        assert_eq!(
            pool_resolution_failure_kind("request timeout after 10 seconds"),
            "timeout"
        );
        assert_eq!(
            pool_resolution_failure_kind("transport failure at https://rpc.example/key"),
            "RPC unavailable"
        );
    }

    #[test]
    fn one_hundred_percent_drains_account_after_exact_fee() {
        let balance = 35_551_671;
        let fee = 5_000;
        let rent = 890_880;
        let (amount, reserve) =
            calculate_sol_transfer_amount(balance, fee, rent, None, Some(100)).expect("amount");
        assert_eq!(reserve, 0);
        assert_eq!(amount, 35_546_671);
        assert_eq!(balance - fee - amount, 0);
    }

    #[test]
    fn fixed_transfer_cannot_consume_rent_reserve() {
        assert!(
            calculate_sol_transfer_amount(1_000_000, 5_000, 890_880, Some(200_000), None,).is_err()
        );
    }

    #[test]
    fn sell_simulation_requires_exact_debit_and_protected_wsol_credit() {
        let verified =
            |ok, pre_input, post_input, input_account, pre_wsol, post_wsol, wsol_account| {
                super::sell_balance_and_minimum_verified(
                    ok,
                    pre_input,
                    post_input,
                    input_account,
                    pre_wsol,
                    post_wsol,
                    wsol_account,
                    1_000,
                    250,
                    40,
                )
            };
        assert!(verified(
            true,
            Some(1_000),
            Some(750),
            Some(750),
            Some(10),
            Some(50),
            Some(50)
        ));
        assert!(!verified(
            true,
            Some(999),
            Some(749),
            Some(749),
            Some(10),
            Some(50),
            Some(50)
        ));
        assert!(!verified(
            true,
            Some(1_000),
            Some(751),
            Some(751),
            Some(10),
            Some(50),
            Some(50)
        ));
        assert!(!verified(
            true,
            Some(1_000),
            Some(750),
            Some(751),
            Some(10),
            Some(50),
            Some(50)
        ));
        assert!(!verified(
            true,
            Some(1_000),
            Some(750),
            Some(750),
            Some(10),
            Some(49),
            Some(49)
        ));
        assert!(!verified(
            false,
            Some(1_000),
            Some(750),
            Some(750),
            Some(10),
            Some(50),
            Some(50)
        ));
    }
}
