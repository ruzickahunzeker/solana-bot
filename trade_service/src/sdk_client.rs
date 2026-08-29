use anyhow::{anyhow, bail, Context};
use sol_trade_sdk::common::{
    fast_fn::get_associated_token_address_with_program_id_fast, spl_token, spl_token_2022,
    GasFeeStrategy, GasFeeStrategyType, TradeConfig,
};
use sol_trade_sdk::swqos::{
    AstralaneTransport, SwqosConfig, SwqosRegion, SwqosTransport, SwqosType, TradeType,
};
use sol_trade_sdk::trading::core::params::{
    BonkParams, DexParamEnum, MeteoraDammV2Params, PumpFunParams, PumpSwapParams,
    RaydiumAmmV4Params, RaydiumCpmmParams,
};
use sol_trade_sdk::trading::factory::DexType;
use sol_trade_sdk::{
    find_pool_by_mint, TradeBuyParams, TradeSellParams, TradeTokenType, TradingClient,
};
use solana_account_decoder::UiAccountData;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::hash::Hash;
use solana_sdk::message::Message;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature, Signer};
use solana_sdk::transaction::Transaction;
use solana_system_interface::instruction as system_instruction;
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

impl SdkTradeClient {
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
            let result = match pool {
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
                    let params =
                        RaydiumCpmmParams::from_pool_address_by_rpc(lane.client.get_rpc(), pool_id)
                            .await?;
                    ensure_pool_contains_quote(
                        "raydium-cpmm",
                        quote_mint,
                        &[params.base_mint, params.quote_mint],
                    )?;
                    Ok(())
                }
            };
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
            )
            .await?;
        let dex_type = dex_type(trade.pool);
        let input_token_type = trade_token_type_for_buy(trade.pool, &trade.quote_mint)?;
        let amount = trade.amount_lamports.context("amount_lamports missing")?;
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
        let params = self
            .dex_params(
                &lane.client,
                trade.pool,
                &trade.mint,
                &trade.quote_mint,
                trade.pool_id.as_ref(),
            )
            .await?;
        let sell_params = TradeSellParams {
            dex_type: dex_type(trade.pool),
            output_token_type: trade_token_type_for_sell(trade.pool, &trade.quote_mint)?,
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
            fixed_output_token_amount: sell_fixed_output_token_amount(trade.pool),
            gas_fee_strategy: default_fee_strategy(lane.swqos_type, trade.tip_sol),
            simulate: trade.simulate,
            grpc_recv_us: None,
        };
        map_sdk_result(
            lane.client
                .sell_by_percent(sell_params, balance, percent)
                .await,
        )
    }

    async fn dex_params(
        &self,
        client: &TradingClient,
        pool: Pool,
        mint: &Pubkey,
        quote_mint: &Pubkey,
        pool_id: Option<&Pubkey>,
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
        let mut total = 0_u64;
        for account in accounts {
            if let UiAccountData::Json(parsed) = account.account.data {
                if parsed.program != "spl-token" && parsed.program != "spl-token-2022" {
                    continue;
                }
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
        if total > 0 {
            return Ok(total);
        }
        self.token_balance_from_atas(client, mint).await
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
                let pump_result = PumpFunParams::from_mint_by_rpc(rpc, mint).await;
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
                            "pump bonding curve is complete but PumpSwap pool is unavailable: {pump_amm_error}"
                        )),
                        Err(pump_error) => Err(format!(
                            "no supported pool found for mint; pump={pump_error}; pump-amm={pump_amm_error}"
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

fn sell_fixed_output_token_amount(pool: Pool) -> Option<u64> {
    match pool {
        Pool::MeteoraDammV2 => Some(0),
        _ => None,
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

fn trade_token_type_for_buy(pool: Pool, quote_mint: &Pubkey) -> anyhow::Result<TradeTokenType> {
    if is_usdc_quote(quote_mint) {
        if pool == Pool::Bonk {
            bail!("bonk does not support USDC quote in this SDK; use WSOL or USD1");
        }
        return Ok(TradeTokenType::USDC);
    }
    if is_usd1_quote(quote_mint) {
        if pool != Pool::Bonk {
            bail!("USD1 quote is only supported for bonk in this SDK");
        }
        return Ok(TradeTokenType::USD1);
    }
    if is_wsol_quote(quote_mint) {
        return Ok(match pool {
            Pool::Pump => TradeTokenType::SOL,
            Pool::PumpAmm
            | Pool::Bonk
            | Pool::MeteoraDammV2
            | Pool::RaydiumAmmV4
            | Pool::RaydiumCpmm => TradeTokenType::WSOL,
        });
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
        if pool != Pool::Bonk {
            bail!("USD1 quote is only supported for bonk in this SDK");
        }
        return Ok(TradeTokenType::USD1);
    }
    if is_wsol_quote(quote_mint) {
        return Ok(TradeTokenType::SOL);
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
        Pool::MeteoraDammV2 => DexType::MeteoraDammV2,
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
    use super::{calculate_sol_transfer_amount, pool_resolution_failure_kind};

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
