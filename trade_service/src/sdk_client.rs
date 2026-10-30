use anyhow::{anyhow, bail, Context};
use sol_trade_sdk::common::{
    fast_fn::get_associated_token_address_with_program_id_fast, spl_token, spl_token_2022,
    GasFeeStrategy, GasFeeStrategyType, TradeConfig,
};
use sol_trade_sdk::instruction::meteora_damm_v2::MeteoraDammV2InstructionBuilder;
use sol_trade_sdk::swqos::{
    AstralaneTransport, SwqosConfig, SwqosRegion, SwqosTransport, SwqosType, TradeType,
};
use sol_trade_sdk::trading::core::params::SwapParams;
use sol_trade_sdk::trading::core::params::{
    BonkParams, DexParamEnum, MeteoraDammV2Params, MeteoraDbcParams, PumpFunParams, PumpSwapParams,
    RaydiumAmmV4Params, RaydiumCpmmParams, StonkFunParams, StonkFunViaSolParams,
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
    RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_client::rpc_filter::Memcmp;
use solana_client::rpc_filter::RpcFilterType;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::hash::Hash;
use solana_sdk::message::Message;
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
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{sleep, Duration, Instant};
use tracing::{info, warn};

use crate::config::{MevConfig, ProviderConfig};
use crate::models::{MevProvider, Pool, ResolvePoolResponse, USD1_MINT, USDC_MINT, WSOL_MINT};
use crate::pumpapi_client::{self, PumpApiTrade};
use crate::sanitize::redact_urls;

pub struct SdkTradeClient {
    lanes: Vec<SdkLane>,
    swqos_lanes: Vec<String>,
    mev_protection: bool,
    require_mev_submit: bool,
    rpc_count: usize,
}

struct SdkLane {
    swqos_type: SwqosType,
    client: TradingClient,
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
        )
    } else if *program == spl_token_2022::ID {
        matches!(
            Token2022Instruction::unpack(data)
                .context("invalid Token-2022 target ATA instruction")?,
            Token2022Instruction::InitializeAccount
                | Token2022Instruction::InitializeAccount2 { .. }
                | Token2022Instruction::InitializeAccount3 { .. }
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
                            Some("initializeAccount" | "initializeAccount2" | "initializeAccount3")
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

fn decoded_system_transfer_amount(data: &[u8], accounts: &[Pubkey], payer: &Pubkey) -> Option<u64> {
    if data.len() != 12 || data[..4] != [2, 0, 0, 0] || accounts.len() != 2 {
        return None;
    }
    if accounts[0] != *payer {
        return Some(0);
    }
    Some(u64::from_le_bytes(data[4..12].try_into().ok()?))
}

fn inner_payer_system_spend(
    groups: &[UiInnerInstructions],
    keys: &[Pubkey],
    payer: &Pubkey,
) -> anyhow::Result<Option<u64>> {
    let system_program = Pubkey::default().to_string();
    let mut total = 0u64;
    for group in groups {
        for instruction in &group.instructions {
            let amount = match instruction {
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
                        .and_then(|data| decoded_system_transfer_amount(&data, &accounts, payer))
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
                        .and_then(|data| decoded_system_transfer_amount(&data, &accounts, payer))
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
                            Some(source) if source == payer.to_string() => ix
                                .parsed
                                .pointer("/info/lamports")
                                .and_then(|value| value.as_u64()),
                            Some(_) => Some(0),
                            None => None,
                        }
                    }
                }
                _ => continue,
            };
            let Some(amount) = amount else {
                return Ok(None);
            };
            total = total
                .checked_add(amount)
                .context("inner System transfer total overflow")?;
        }
    }
    Ok(Some(total))
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

#[cfg(test)]
mod aggregator_simulation_tests {
    use super::*;
    use base64::Engine;

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
        }
    }

    #[test]
    fn inner_system_spend_counts_payer_and_marks_unknown_instructions() {
        let payer = Pubkey::new_unique();
        let recipient = Pubkey::new_unique();
        let keys = [Pubkey::default(), payer, recipient];
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
            inner_payer_system_spend(&[group], &keys, &payer).unwrap(),
            Some(123)
        );
        let unknown = UiInnerInstructions {
            index: 0,
            instructions: vec![UiInstruction::Compiled(
                solana_transaction_status_client_types::UiCompiledInstruction {
                    data: bs58::encode([0u8; 4]).into_string(),
                    ..instruction
                },
            )],
        };
        assert_eq!(
            inner_payer_system_spend(&[unknown], &keys, &payer).unwrap(),
            None
        );
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
    ) -> anyhow::Result<Vec<Pubkey>> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no default RPC lane available for aggregator preflight")?;
        let account = tokio::time::timeout(
            Duration::from_secs(5),
            lane.client.get_rpc().get_account(address),
        )
        .await
        .context("aggregator lookup table RPC timed out")?
        .map_err(|error| anyhow::anyhow!("{}", redact_urls(&error.to_string())))?;
        if account.owner != lookup_program::id() {
            bail!("aggregator lookup table has invalid owner");
        }
        let table = AddressLookupTable::deserialize(&account.data)
            .map_err(|_| anyhow!("invalid aggregator lookup table data"))?;
        if table.meta.deactivation_slot != u64::MAX {
            bail!("aggregator lookup table is deactivating or deactivated");
        }
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
        let inner_payer_system_spend_lamports = value
            .inner_instructions
            .as_ref()
            .map(|groups| inner_payer_system_spend(groups, resolved_keys, &self.wallet()))
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

    pub async fn resolve_stonkfun_sol_hop(&self, quote_mint: &Pubkey) -> anyhow::Result<Pubkey> {
        let wsol = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
        if quote_mint == &wsol {
            bail!("quote mint is already WSOL");
        }
        let mut last_error = None;
        for lane in self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
        {
            let rpc = lane.client.get_rpc();
            let found = tokio::time::timeout(Duration::from_secs(5), async {
                use sol_trade_sdk::instruction::utils::raydium_cpmm::{accounts, fetch_pool_state};
                use sol_trade_sdk::instruction::utils::raydium_cpmm_types::POOL_STATE_DISCRIMINATOR;
                for offset in [168usize, 200usize] {
                    let config = RpcProgramAccountsConfig {
                        filters: Some(vec![
                            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                0,
                                &POOL_STATE_DISCRIMINATOR,
                            )),
                            RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                offset,
                                quote_mint.as_ref(),
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
                            ensure_pool_contains_quote("stonkfun Via-SOL hop", quote_mint, &mints)?;
                            ensure_pool_contains_quote("stonkfun Via-SOL hop", &wsol, &mints)?;
                            let (base_balance, quote_balance) = tokio::try_join!(
                                rpc.get_token_account_balance(&params.base_vault),
                                rpc.get_token_account_balance(&params.quote_vault),
                            )?;
                            if base_balance.amount.parse::<u64>()? == 0
                                || quote_balance.amount.parse::<u64>()? == 0
                            {
                                continue;
                            }
                            return Ok::<Pubkey, anyhow::Error>(pool_id);
                        }
                    }
                }
                bail!("no WSOL/Quote CPMM account found")
            })
            .await;
            match found {
                Ok(Ok(pool_id)) => return Ok(pool_id),
                Ok(Err(error)) => last_error = Some(redact_urls(&error.to_string())),
                Err(_) => last_error = Some("SOL hop discovery timed out".to_string()),
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
                    lanes.push(SdkLane { swqos_type, client });
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
                Ok(Err(error)) => last_error = Some(error.to_string()),
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
                Ok(Err(error)) => last_error = Some(error.to_string()),
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
                Ok(Err(error)) => last_error = Some(error.to_string()),
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
                        let pool_id = pool_id.context("pool_id is required for meteora-dbc")?;
                        let params = MeteoraDbcParams::from_pool_address_by_rpc(
                            lane.client.get_rpc(),
                            pool_id,
                        )
                        .await?;
                        ensure_pool_contains_mint("meteora-dbc", mint, &[params.base_mint])?;
                        ensure_pool_contains_quote(
                            "meteora-dbc",
                            quote_mint,
                            &[params.quote_mint],
                        )?;
                        Ok(())
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
        let mut last_error = None;
        let mut last_submitted = None;
        let mut fallback_notes = Vec::new();
        let lanes = self.trade_execution_lanes(&trade)?;
        for lane in lanes {
            match self.buy_on_lane(lane, trade.clone()).await {
                Ok(mut result) if result.success => {
                    info!(
                        swqos_lane = lane.swqos_type.as_str(),
                        signature = ?result.signature,
                        "buy lane submitted successfully"
                    );
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
                    warn!(
                        swqos_lane = lane.swqos_type.as_str(),
                        signature = ?result.signature,
                        error = ?result.error,
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
        let dex_type = dex_type(trade.pool);
        let input_token_type = if trade.sol_hop_pool_id.is_some() {
            TradeTokenType::SOL
        } else {
            trade_token_type_for_buy(trade.pool, &trade.quote_mint)?
        };
        let amount = trade.amount_lamports.context("amount_lamports missing")?;
        if matches!(trade.pool, Pool::StonkFun | Pool::StonkFunSwap)
            && !is_wsol_quote(&trade.quote_mint)
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
        map_sdk_result(lane.client.buy(buy_params).await)
    }

    pub async fn sell(&self, trade: ExecuteTrade) -> anyhow::Result<ExecuteResult> {
        let mut last_error = None;
        let mut last_submitted = None;
        let mut fallback_notes = Vec::new();
        let lanes = self.trade_execution_lanes(&trade)?;
        for lane in lanes {
            match self.sell_on_lane(lane, trade.clone()).await {
                Ok(mut result) if result.success => {
                    info!(
                        swqos_lane = lane.swqos_type.as_str(),
                        signature = ?result.signature,
                        "sell lane submitted successfully"
                    );
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
                    warn!(
                        swqos_lane = lane.swqos_type.as_str(),
                        signature = ?result.signature,
                        error = ?result.error,
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
            dex_type: dex_type(trade.pool),
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
                let pool_id = pool_id.context("pool_id is required for meteora-dbc")?;
                let params =
                    MeteoraDbcParams::from_pool_address_by_rpc(client.get_rpc(), pool_id).await?;
                ensure_pool_contains_mint("meteora-dbc", mint, &[params.base_mint])?;
                ensure_pool_contains_quote("meteora-dbc", quote_mint, &[params.quote_mint])?;
                DexParamEnum::MeteoraDbc(params)
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
                Err(error) => last_error = Some(error),
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
        let signature =
            Signature::from_str(signature_text).context("parse submitted transaction signature")?;
        let deadline = Instant::now() + Duration::from_millis(trade.mev_submit_verify_ms);

        loop {
            let statuses = lane
                .client
                .get_rpc()
                .get_signature_statuses_with_history(&[signature])
                .await
                .map_err(|error| anyhow!("{}", redact_urls(&error.to_string())))?;
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
                    "{} submit signature not visible after {}ms; trying fallback lane",
                    lane.swqos_type.as_str(),
                    trade.mev_submit_verify_ms
                ));
                warn!(
                    swqos_lane = lane.swqos_type.as_str(),
                    signature = %signature,
                    verify_ms = trade.mev_submit_verify_ms,
                    "lane signature not visible, trying fallback"
                );
                return Ok(Some(false));
            }

            sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn resolve_pool(&self, mint: &Pubkey) -> anyhow::Result<ResolvePoolResponse> {
        let default_lanes = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
            .collect::<Vec<_>>();
        if default_lanes.is_empty() {
            bail!("no Default RPC lane configured for pool resolution");
        }

        let mut failures = Vec::with_capacity(default_lanes.len());
        for (lane_index, lane) in default_lanes.iter().enumerate() {
            let rpc = lane.client.get_rpc();
            let lane_result = tokio::time::timeout(Duration::from_secs(5), async {
                let (pump_result, dbc_result) = tokio::join!(
                    PumpFunParams::from_mint_by_rpc(rpc, mint),
                    MeteoraDbcParams::from_base_mint_by_rpc(rpc, mint),
                );
                if let Ok(params) = &pump_result {
                    if !params.bonding_curve.complete {
                        return Ok(ResolvePoolResponse {
                            mint: mint.to_string(),
                            pool: Pool::Pump,
                            pool_id: None,
                            quote_mint: params.quote_mint.to_string(),
                        });
                    }
                }
                if let Ok(params) = &dbc_result {
                    return Ok(ResolvePoolResponse {
                        mint: mint.to_string(),
                        pool: Pool::MeteoraDbc,
                        pool_id: Some(params.pool.to_string()),
                        quote_mint: params.quote_mint.to_string(),
                    });
                }
                let dbc_error = match dbc_result {
                    Err(error) => error,
                    Ok(_) => unreachable!("successful DBC resolution returned above"),
                };

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
                        Ok(_) => Err(format!(
                            "pump bonding curve is complete but migrated pools are unavailable: pump-amm={pump_amm_error}; meteora-dbc={dbc_error}"
                        )),
                        Err(pump_error) => Err(format!(
                            "no supported pool found for mint; pump={pump_error}; pump-amm={pump_amm_error}; meteora-dbc={dbc_error}"
                        )),
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
    if matches!(pool, Pool::StonkFun | Pool::StonkFunSwap) {
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
    if matches!(pool, Pool::StonkFun | Pool::StonkFunSwap) {
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
    let error_text = error.to_string();
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
    let error = error.unwrap_or("unknown error");
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
        |token, region, url| SwqosConfig::Temporal(token, region, url),
    )?;
    push_provider(
        &mut configs,
        "FLASHBLOCK",
        &mev.flashblock,
        |token, region, url| SwqosConfig::FlashBlock(token, region, url),
    )?;

    if mev.blockrazor.provider.enabled {
        let token = provider_token("BLOCKRAZOR", &mev.blockrazor.provider)?;
        configs.push(SwqosConfig::BlockRazor(
            token,
            parse_region(&mev.blockrazor.provider.region)?,
            mev.blockrazor.provider.custom_url.clone(),
            Some(parse_swqos_transport(&mev.blockrazor.transport)?),
        ));
    }

    if mev.astralane.provider.enabled {
        let token = provider_token("ASTRALANE", &mev.astralane.provider)?;
        configs.push(SwqosConfig::Astralane(
            token,
            parse_region(&mev.astralane.provider.region)?,
            mev.astralane.provider.custom_url.clone(),
            Some(parse_astralane_transport(&mev.astralane.transport)?),
        ));
    }

    push_provider(
        &mut configs,
        "SPEEDLANDING",
        &mev.speedlanding,
        |token, region, url| SwqosConfig::Speedlanding(token, region, url),
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

fn dex_type(pool: Pool) -> DexType {
    match pool {
        Pool::Pump => DexType::PumpFun,
        Pool::PumpAmm => DexType::PumpSwap,
        Pool::Bonk => DexType::Bonk,
        Pool::StonkFun | Pool::StonkFunSwap => DexType::StonkFun,
        Pool::MeteoraDammV2 => DexType::MeteoraDammV2,
        Pool::MeteoraDbc => DexType::MeteoraDbc,
        Pool::RaydiumAmmV4 => DexType::RaydiumAmmV4,
        Pool::RaydiumCpmm => DexType::RaydiumCpmm,
    }
}

fn default_fee_strategy(swqos_type: SwqosType, tip_sol: f64) -> GasFeeStrategy {
    let strategy = GasFeeStrategy::new();
    let tip = if swqos_type == SwqosType::Default {
        0.0
    } else {
        tip_sol
    };
    strategy.set(
        swqos_type,
        TradeType::Buy,
        GasFeeStrategyType::Normal,
        300_000,
        1_000,
        tip,
    );
    strategy.set(
        swqos_type,
        TradeType::Sell,
        GasFeeStrategyType::Normal,
        300_000,
        1_000,
        tip,
    );
    strategy
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
) -> anyhow::Result<ExecuteResult> {
    let (success, signatures, error, _) = result?;
    Ok(ExecuteResult {
        success,
        signature: signatures.first().map(ToString::to_string),
        error: error.map(|error| redact_urls(&format!("{error:?}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_sol_transfer_amount, default_fee_strategy, minimum_output,
        pool_resolution_failure_kind, simulation_fee_strategy,
    };
    use sol_trade_sdk::swqos::{SwqosType, TradeType};

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
}
