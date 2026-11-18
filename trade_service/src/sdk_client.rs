use anyhow::{anyhow, bail, Context};
#[cfg(test)]
use sol_trade_sdk::common::{spl_token, spl_token_2022};
use sol_trade_sdk::swqos::{
    AstralaneTransport, SwqosConfig, SwqosRegion, SwqosTransport, SwqosType,
};
use sol_trade_sdk::trading::core::params::MeteoraDammV2Params;
use sol_trade_sdk::TradingClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature, Signer};
use solana_sdk::transaction::VersionedTransaction;
#[cfg(test)]
use solana_transaction_status_client_types::{UiInnerInstructions, UiInstruction};
use std::collections::HashMap;
#[cfg(test)]
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
use crate::adapters::sdk::protocol_params;
#[cfg(test)]
use crate::adapters::sdk::protocol_params::quote_tokens::{
    trade_token_type_for_buy, trade_token_type_for_sell,
};
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
mod original_damm;
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

#[cfg(test)]
fn pool_resolution_lane_order(helius_lanes: &[bool], swap_helius: bool) -> Vec<usize> {
    crate::adapters::sdk::pool_resolution::lane_order(helius_lanes, swap_helius)
}

pub use crate::adapters::sdk::trade_command::ExecuteTrade;

pub use crate::application::trade_execution::TradeExecutionResult as ExecuteResult;

pub(crate) use crate::adapters::sdk::aggregator_simulation::audit::check_target_token_instruction;
#[cfg(test)]
use crate::adapters::sdk::aggregator_simulation::audit::*;
pub use crate::adapters::sdk::aggregator_simulation::AggregatorSimulation;

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

    pub(crate) async fn aggregator_target_ata(
        &self,
        mint: &Pubkey,
        read_rpc: &crate::adapters::rpc::read_runtime::SharedReadRuntime,
    ) -> anyhow::Result<(Pubkey, Pubkey)> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no default RPC lane available for aggregator preflight")?;
        crate::adapters::sdk::aggregator_accounts::target_ata(
            read_rpc,
            &lane.client.get_rpc().url(),
            lane.client.get_rpc().commitment(),
            self.wallet(),
            *mint,
        )
        .await
    }

    pub(crate) async fn resolve_aggregator_lookup_table(
        &self,
        address: &Pubkey,
        read_rpc: &crate::adapters::rpc::read_runtime::SharedReadRuntime,
    ) -> anyhow::Result<Vec<Pubkey>> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no default RPC lane available for aggregator preflight")?;
        crate::adapters::sdk::aggregator_accounts::lookup_table(
            read_rpc,
            &lane.client.get_rpc().url(),
            lane.client.get_rpc().commitment(),
            *address,
        )
        .await
    }

    pub(crate) async fn simulate_unsigned_aggregator_transaction(
        &self,
        transaction: &VersionedTransaction,
        target_account_index: u8,
        target_mint: &Pubkey,
        target_ata: &Pubkey,
        target_token_program: &Pubkey,
        resolved_keys: &[Pubkey],
        read_rpc: &crate::adapters::rpc::read_runtime::SharedReadRuntime,
    ) -> anyhow::Result<AggregatorSimulation> {
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.swqos_type == SwqosType::Default)
            .context("no default RPC lane available for aggregator preflight")?;
        crate::adapters::sdk::aggregator_simulation::simulate(
            read_rpc,
            &lane.client.get_rpc().url(),
            self.wallet(),
            transaction,
            target_account_index,
            *target_mint,
            *target_ata,
            *target_token_program,
            resolved_keys,
        )
        .await
    }

    pub(crate) async fn resolve_stonkfun_sol_hop(
        &self,
        quote_mint: &Pubkey,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> anyhow::Result<Pubkey> {
        use crate::adapters::sdk::{pool_resolution::DiscoveryLane, sol_hop_resolution};
        let lanes = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
            .map(|lane| {
                DiscoveryLane::new(
                    lane.client.get_rpc().url(),
                    lane.client.get_rpc().commitment(),
                    lane.is_helius,
                )
            })
            .collect::<Vec<_>>();
        sol_hop_resolution::resolve(read_rpc, &lanes, &self.sol_hop_cache, *quote_mint).await
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
                    let trade_config = crate::adapters::sdk::client_config::lane_config(
                        rpc_url.clone(),
                        swqos_config.clone(),
                        commitment.clone(),
                        enable_sdk_log,
                        mev.enable_mev_protection,
                    );
                    let client = TradingClient::new(payer.clone(), trade_config).await;
                    lanes.push(SdkLane {
                        swqos_type,
                        is_helius,
                        client,
                    });
                }
            }

            let trade_config = crate::adapters::sdk::client_config::lane_config(
                rpc_url.clone(),
                SwqosConfig::Default(rpc_url.clone()),
                commitment.clone(),
                enable_sdk_log,
                mev.enable_mev_protection,
            );
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

    pub(crate) async fn warmup_params(
        &self,
        hint: &PoolRouteHint,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> anyhow::Result<()> {
        let mut last_error = None;
        for lane in self.execution_lanes() {
            let result = protocol_params::warmup_scoped(
                read_rpc.clone(),
                lane.client.get_rpc().url(),
                lane.client.get_rpc().commitment(),
                hint,
            )
            .await;
            match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if ordinary_lanes::preparation_rejection(&error).is_some() {
                        return Err(error.context(format!(
                            "{} parameter read rejected",
                            lane.swqos_type.as_str()
                        )));
                    }
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
        read_rpc: &crate::adapters::rpc::read_runtime::SharedReadRuntime,
        lane: &SdkLane,
        damm: &MeteoraDammV2Params,
        mint: Pubkey,
        quote_mint: Pubkey,
        input_amount: u64,
        recent_blockhash: Hash,
        min_context_slot: u64,
    ) -> anyhow::Result<crate::adapters::sdk::damm_probe::DammOutputEvidence> {
        if quote_mint != damm.token_a_mint && quote_mint != damm.token_b_mint {
            bail!("DAMM v2 quote mint is not in the pool");
        }
        let owner = lane.client.payer.pubkey();
        let rpc = crate::adapters::rpc::damm_quote_reads::DammQuoteReader::new(
            read_rpc,
            lane.client.get_rpc().url(),
        );
        let prepared = crate::adapters::sdk::damm_probe::prepare_damm_probe(
            owner,
            damm,
            mint,
            quote_mint,
            input_amount,
            recent_blockhash,
            lane.client.use_seed_optimize,
        )?;
        let evidence = crate::adapters::sdk::damm_probe::inspect_prepared_damm_probe_at_slot(
            &rpc,
            &prepared,
            min_context_slot,
        )
        .await?;
        crate::adapters::sdk::damm_probe::evidence_damm_probe_output(&evidence).await
    }

    pub(crate) async fn resolve_pool(
        &self,
        mint: &Pubkey,
        read_rpc: Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> anyhow::Result<PoolRouteHint> {
        use crate::adapters::sdk::pool_resolution::{self, DiscoveryLane};
        let lanes = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
            .map(|lane| {
                DiscoveryLane::new(
                    lane.client.get_rpc().url(),
                    lane.client.get_rpc().commitment(),
                    lane.is_helius,
                )
            })
            .collect::<Vec<_>>();
        pool_resolution::resolve(read_rpc, &lanes, Address::from_bytes(mint.to_bytes())).await
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

pub(crate) fn provider_swqos_type(provider: MevProvider) -> Option<SwqosType> {
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

#[cfg(test)]
fn pool_resolution_failure_kind(error: &str) -> String {
    crate::adapters::sdk::pool_resolution::failure_kind(error)
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
