use anyhow::{anyhow, bail, Context};
use ring::rand::{SecureRandom, SystemRandom};
use sol_trade_sdk::common::{
    fast_fn::get_associated_token_address_with_program_id_fast, spl_token, spl_token_2022,
    TradeConfig,
};
use sol_trade_sdk::swqos::{
    AstralaneTransport, SwqosConfig, SwqosRegion, SwqosTransport, SwqosType,
};
use sol_trade_sdk::trading::core::params::{
    DexParamEnum, MeteoraDammV2Params, MeteoraDbcParams, PumpFunParams, PumpSwapParams,
    RaydiumCpmmParams,
};
use sol_trade_sdk::trading::factory::DexType;
use sol_trade_sdk::{find_pool_by_mint, TradingClient};
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
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature, Signer};
use solana_sdk::transaction::VersionedTransaction;
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
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};
use tracing::{info, warn};

use crate::adapters::sdk::ordinary_lanes;
#[cfg(test)]
use crate::adapters::sdk::ordinary_params::default_fee_strategy;
#[cfg(test)]
use crate::adapters::sdk::ordinary_params::simulation_fee_strategy;
#[cfg(test)]
use crate::adapters::sdk::protocol_params::quote_tokens::{
    trade_token_type_for_buy, trade_token_type_for_sell,
};
use crate::adapters::sdk::protocol_params::{self, checks::ensure_pool_contains_quote};
#[cfg(test)]
use crate::application::sol_transfer_policy::calculate_sol_transfer_amount;
use crate::config::{MevConfig, ProviderConfig};
use crate::domain::execution_policy::BroadcastPolicy;
use crate::domain::{route::PoolRouteHint, trade::Address};
use crate::models::{MevProvider, Pool, Side, WSOL_MINT};
#[cfg(test)]
use crate::protocol::meteora_damm_v2::minimum_output;
use crate::pumpapi_client::{self, PumpApiTrade};
use crate::sanitize::redact_urls;
#[cfg(test)]
mod isolation_tests;
mod lane_execution;
mod lane_preparation;
#[cfg(test)]
mod ordinary_param_contract_tests;
#[cfg(test)]
mod param_contract_tests;
pub(crate) mod sol_transfer;
#[cfg(test)]
mod sol_transfer_contract_tests;
pub(crate) mod wsol_conversion;

pub struct SdkTradeClient {
    lanes: Vec<SdkLane>,
    sol_hop_cache: Mutex<HashMap<Pubkey, (Pubkey, Instant)>>,
    swqos_lanes: Vec<String>,
    mev_protection: bool,
    require_mev_submit: bool,
    rpc_count: usize,
    broadcast_policy: BroadcastPolicy,
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

pub use crate::adapters::sdk::trade_command::ExecuteTrade;

pub use crate::application::trade_execution::TradeExecutionResult as ExecuteResult;

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
            assert!(check_target_token_instruction(&program, &[22], &[target], &target).is_ok());
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
    pub(crate) fn ensure_broadcast_allowed(&self) -> anyhow::Result<()> {
        self.broadcast_policy.ensure_allowed()
    }

    fn wallet_signer(&self) -> Arc<Keypair> {
        self.lanes[0].client.payer.clone()
    }

    pub(crate) fn sign_prepared_quote(
        &self,
        prepared: crate::adapters::sdk::quote_submission::PreparedQuoteTransaction,
    ) -> anyhow::Result<crate::domain::submission::SignedSubmission> {
        self.ensure_broadcast_allowed()?;
        let message = prepared.into_verified_message(&self.wallet())?;
        let signer = self.wallet_signer();
        let transaction = VersionedTransaction::try_new(message, &[signer.as_ref()])
            .context("failed to sign aggregator transaction")?;
        Ok(crate::domain::submission::SignedSubmission {
            signature: transaction.signatures[0].to_string(),
            bytes: bincode::serialize(&transaction)?,
        })
    }

    pub async fn send_aggregator_transaction(
        &self,
        transaction: &VersionedTransaction,
    ) -> anyhow::Result<Signature> {
        self.ensure_broadcast_allowed()?;
        let payer = self.wallet_signer();
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
    pub(crate) async fn new(
        rpc_urls: Vec<String>,
        private_key_base58: String,
        mev: MevConfig,
        commitment: CommitmentConfig,
        enable_sdk_log: bool,
        broadcast_policy: BroadcastPolicy,
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
            broadcast_policy,
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

    pub(crate) fn read_access(
        &self,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> crate::adapters::rpc::wallet_reads::RpcWalletReader {
        crate::adapters::rpc::wallet_reads::RpcWalletReader::new(
            crate::domain::trade::Address::from_bytes(self.wallet().to_bytes()),
            self.rpcs(),
            read_rpc,
        )
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

    pub(crate) fn readiness_probe(&self) -> crate::adapters::rpc::readiness::RpcReadinessProbe {
        crate::adapters::rpc::readiness::RpcReadinessProbe::new(
            crate::domain::readiness::ReadinessWalletMetadata {
                wallet: crate::domain::trade::Address::from_bytes(self.wallet().to_bytes()),
                rpc_lanes: self.rpc_count(),
                swqos_lanes: self.swqos_lanes(),
                active_swqos_lanes: self.active_swqos_lanes(),
                mev_protection: self.mev_protection(),
                require_mev_submit: self.require_mev_submit(),
            },
            self.execution_lanes()
                .into_iter()
                .map(|lane| lane.client.get_rpc().url())
                .collect(),
        )
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

    pub async fn pumpapi_trade(&self, trade: PumpApiTrade) -> anyhow::Result<ExecuteResult> {
        if !trade.simulate() {
            self.ensure_broadcast_allowed()?;
        }
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no Default RPC lane configured for PumpAPI execution")?;
        if trade.simulate() {
            pumpapi_client::simulate_trade(
                lane.client.get_rpc(),
                Address::from_bytes(lane.client.payer.pubkey().to_bytes()),
                trade,
            )
            .await
        } else {
            pumpapi_client::execute_live(&lane.client, trade, self.broadcast_policy).await
        }
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
            let result = protocol_params::warmup_on_rpc(
                lane.client.get_rpc(),
                pool,
                mint,
                quote_mint,
                pool_id,
            )
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

    pub(crate) async fn buy(
        &self,
        trade: ExecuteTrade,
        read_rpc: impl FnOnce() -> Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime> + Send,
    ) -> anyhow::Result<ExecuteResult> {
        self.execute_sdk(trade, Side::Buy, read_rpc).await
    }

    pub(crate) async fn sell(
        &self,
        trade: ExecuteTrade,
        read_rpc: impl FnOnce() -> Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime> + Send,
    ) -> anyhow::Result<ExecuteResult> {
        self.execute_sdk(trade, Side::Sell, read_rpc).await
    }

    async fn execute_sdk(
        &self,
        trade: ExecuteTrade,
        side: Side,
        read_rpc: impl FnOnce() -> Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime> + Send,
    ) -> anyhow::Result<ExecuteResult> {
        self.ensure_broadcast_allowed()?;
        let lanes = self.trade_execution_lanes(&trade)?;
        let read_rpc = read_rpc();
        let lanes = lanes
            .into_iter()
            .map(|lane| lane_execution::LaneExecution::new(self, lane, side, &read_rpc))
            .collect::<Vec<_>>();
        ordinary_lanes::execute(lanes, &trade).await
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
        if quote_mint != damm.token_a_mint && quote_mint != damm.token_b_mint {
            bail!("DAMM v2 quote mint is not in the pool");
        }
        let owner = lane.client.payer.pubkey();
        let rpc = lane.client.get_rpc();
        let prepared = crate::adapters::sdk::damm_probe::prepare_damm_probe(
            owner,
            damm,
            mint,
            quote_mint,
            input_amount,
            recent_blockhash,
            lane.client.use_seed_optimize,
        )?;
        let evidence =
            crate::adapters::sdk::damm_probe::inspect_prepared_damm_probe(rpc, &prepared).await?;
        crate::adapters::sdk::damm_probe::quote_evidenced_damm_probe(&evidence).await
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
        protocol_params::dex_params(
            client.get_rpc(),
            pool,
            mint,
            quote_mint,
            pool_id,
            sol_hop_pool,
            sol_hop_pool_id,
        )
        .await
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

    pub async fn resolve_pool(&self, mint: &Pubkey) -> anyhow::Result<PoolRouteHint> {
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
                let (pump_result, dbc_result) = tokio::join!(
                    PumpFunParams::from_mint_by_rpc(rpc, mint),
                    MeteoraDbcParams::from_base_mint_by_rpc(rpc, mint),
                );
                if let Ok(params) = &pump_result {
                    if !params.bonding_curve.complete {
                        return Ok(PoolRouteHint {
                            mint: Address::from_bytes(mint.to_bytes()),
                            pool: Pool::Pump,
                            pool_id: None,
                            quote_mint: Address::from_bytes(params.quote_mint.to_bytes()),
                        });
                    }
                }
                if let Ok(params) = &dbc_result {
                    return Ok(PoolRouteHint {
                        mint: Address::from_bytes(mint.to_bytes()),
                        pool: Pool::MeteoraDbc,
                        pool_id: Some(Address::from_bytes(params.pool.to_bytes())),
                        quote_mint: Address::from_bytes(params.quote_mint.to_bytes()),
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
                    Ok(params) => Ok(PoolRouteHint {
                        mint: Address::from_bytes(mint.to_bytes()),
                        pool: Pool::PumpAmm,
                        pool_id: Some(Address::from_bytes(params.pool.to_bytes())),
                        quote_mint: Address::from_bytes(params.quote_mint.to_bytes()),
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

#[cfg(test)]
mod tests {
    use super::{
        calculate_sol_transfer_amount, default_fee_strategy, is_helius_rpc, minimum_output,
        pool_resolution_failure_kind, pool_resolution_lane_order, simulation_fee_strategy,
        trade_token_type_for_buy, trade_token_type_for_sell,
    };
    use crate::models::Pool;
    use sol_trade_sdk::swqos::{SwqosType, TradeType};
    use sol_trade_sdk::TradeTokenType;
    use solana_sdk::pubkey::Pubkey;

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
    fn damm_v2_minimum_output_handles_u64_boundary_without_float_rounding() {
        assert_eq!(minimum_output(u64::MAX, 0).unwrap(), u64::MAX);
        assert_eq!(minimum_output(u64::MAX, 9_999).unwrap(), u64::MAX / 10_000);
        assert_eq!(minimum_output(10_000, 9_999).unwrap(), 1);
        assert!(minimum_output(9_999, 9_999).is_err());
    }

    #[test]
    fn damm_v2_minimum_output_preserves_failure_categories() {
        for (quote, slippage) in [(0, 0), (100, 10_000), (100, u64::MAX)] {
            assert_eq!(
                minimum_output(quote, slippage).unwrap_err().to_string(),
                "invalid DAMM v2 quote or slippage"
            );
        }
        assert_eq!(
            minimum_output(1, 1_500).unwrap_err().to_string(),
            "DAMM v2 minimum output rounds to zero"
        );
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
