use super::{evidence::BuyOutputEvidence, Funding, PreparedDammBuyProbe};
use crate::{
    adapters::sdk::{damm_instructions::pool_snapshot, trade_command::ExecuteTrade},
    domain::{preparation_balance::reject_balance, trade::Pool},
};
use anyhow::{ensure, Result};
use sol_trade_sdk::{
    common::{GasFeeStrategyType, TradeTransactionVersion},
    swqos::{SwqosType, TradeType},
    trading::{core::params::DexParamEnum, factory::DexType},
    TradeBuyParams, TradeTokenType,
};
use solana_sdk::pubkey::Pubkey;

mod message;
pub(crate) use message::DammBuyMessage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DammBuyContext {
    pub wallet: Pubkey,
    pub provider: SwqosType,
    pub version: TradeTransactionVersion,
    pub seed_optimized: bool,
}

/// Binds a net discovery quote to preparation, not the final signed message or a journal owner.
pub(crate) struct DammBuyQuote {
    context: DammBuyContext,
    intent: ExecuteTrade,
    plan: PreparedDammBuyProbe,
    evidence: BuyOutputEvidence,
    minimum: u64,
}

impl DammBuyQuote {
    pub(crate) async fn read(
        context: DammBuyContext,
        intent: &ExecuteTrade,
        params: &sol_trade_sdk::trading::core::params::MeteoraDammV2Params,
        rpc: &solana_client::nonblocking::rpc_client::RpcClient,
    ) -> Result<Self> {
        ensure!(
            context.version == TradeTransactionVersion::V0 && !context.seed_optimized,
            "DAMM buy quote requires canonical V0-compatible SDK mode"
        );
        ensure!(
            intent.pool == Pool::MeteoraDammV2
                && intent.sol_hop_pool.is_none()
                && intent.sol_hop_pool_id.is_none()
                && intent.sell_percent.is_none(),
            "DAMM buy quote requires a direct buy-only intent"
        );
        let amount = intent
            .amount_lamports
            .ok_or_else(|| anyhow::anyhow!("DAMM buy input amount is missing"))?;
        let plan = super::prepare(
            context.wallet,
            params,
            intent.quote_mint,
            intent.mint,
            amount,
            intent.recent_blockhash,
            context.seed_optimized,
        )?;
        let evidence = super::evidence::quote_current(rpc, &plan).await?;
        Self::new(context, intent, plan, evidence)
    }

    pub(crate) fn new(
        context: DammBuyContext,
        intent: &ExecuteTrade,
        plan: PreparedDammBuyProbe,
        evidence: BuyOutputEvidence,
    ) -> Result<Self> {
        ensure!(
            context.version == TradeTransactionVersion::V0 && !context.seed_optimized,
            "DAMM buy quote requires canonical V0-compatible SDK mode"
        );
        ensure!(
            intent.pool == Pool::MeteoraDammV2
                && intent.sol_hop_pool.is_none()
                && intent.sol_hop_pool_id.is_none()
                && intent.sell_percent.is_none()
                && intent.amount_lamports == Some(plan.accounts.amount)
                && intent.mint == plan.accounts.output_mint
                && intent.quote_mint == plan.accounts.input_mint
                && context.wallet == plan.accounts.wallet
                && intent
                    .pool_id
                    .is_none_or(|pool| pool == plan.accounts.pool.pool)
                && intent.recent_blockhash == plan.transaction.message.recent_blockhash
                && intent.recent_blockhash != solana_sdk::hash::Hash::default()
                && intent.tip_sol.is_finite()
                && intent.tip_sol >= 0.0,
            "DAMM buy discovery quote differs from its ordinary intent"
        );
        let minimum = evidence.minimum(&plan, intent.slippage_bps)?;
        let (account_slot, simulation_slot, _) = evidence.context();
        ensure!(
            account_slot > 0 && simulation_slot >= account_slot,
            "DAMM buy quote context is invalid"
        );
        Ok(Self {
            context,
            intent: intent.clone(),
            plan,
            evidence,
            minimum,
        })
    }

    pub(crate) fn output_units(&self) -> u64 {
        self.evidence.output_units()
    }
    pub(crate) fn context(&self) -> (u64, u64, u64) {
        self.evidence.context()
    }

    pub(crate) fn validate(
        &self,
        context: DammBuyContext,
        intent: &ExecuteTrade,
        request: &TradeBuyParams,
    ) -> Result<()> {
        self.validate_inner(context, intent, request)
            .map_err(|error| {
                reject_balance("DAMM buy quote binding rejected").context(error.to_string())
            })
    }

    pub(crate) fn recheck_request(
        &self,
        context: DammBuyContext,
        request: &TradeBuyParams,
    ) -> Result<()> {
        self.validate(context, &self.intent, request)
    }

    fn validate_inner(
        &self,
        context: DammBuyContext,
        current: &ExecuteTrade,
        request: &TradeBuyParams,
    ) -> Result<()> {
        ensure!(
            context == self.context,
            "DAMM buy wallet, provider or SDK mode changed"
        );
        let frozen = &self.intent;
        ensure!(
            (
                frozen.pool,
                frozen.mint,
                frozen.quote_mint,
                frozen.pool_id,
                frozen.sol_hop_pool,
                frozen.sol_hop_pool_id,
                frozen.amount_lamports,
                frozen.sell_percent
            ) == (
                current.pool,
                current.mint,
                current.quote_mint,
                current.pool_id,
                current.sol_hop_pool,
                current.sol_hop_pool_id,
                current.amount_lamports,
                current.sell_percent
            ) && (
                frozen.slippage_bps,
                frozen.tip_sol.to_bits(),
                frozen.recent_blockhash,
                frozen.wait_tx_confirmed,
                frozen.mev_submit_verify_ms,
                frozen.simulate,
                frozen.use_mev,
                frozen.mev_provider
            ) == (
                current.slippage_bps,
                current.tip_sol.to_bits(),
                current.recent_blockhash,
                current.wait_tx_confirmed,
                current.mev_submit_verify_ms,
                current.simulate,
                current.use_mev,
                current.mev_provider
            ),
            "DAMM buy frozen execution intent changed"
        );
        ensure!(
            request.dex_type == DexType::MeteoraDammV2
                && request.mint == frozen.mint
                && Some(request.input_token_amount) == frozen.amount_lamports
                && request.slippage_basis_points == Some(frozen.slippage_bps)
                && request.recent_blockhash == Some(frozen.recent_blockhash)
                && request.wait_tx_confirmed == frozen.wait_tx_confirmed
                && !request.wait_for_all_submits
                && request.create_input_token_ata
                && !request.close_input_token_ata
                && request.address_lookup_table_accounts.is_empty()
                && request.durable_nonce.is_none()
                && request.simulate == frozen.simulate
                && request.use_exact_sol_amount == Some(true)
                && request.grpc_recv_us.is_none(),
            "DAMM buy request differs from its frozen input profile"
        );
        let token = match self.plan.funding {
            Funding::NativeSol => TradeTokenType::WSOL,
            Funding::ExistingUsdc => TradeTokenType::USDC,
        };
        ensure!(
            request.input_token_type == token,
            "DAMM buy input asset type changed"
        );
        let DexParamEnum::MeteoraDammV2(pool) = &request.extension_params else {
            anyhow::bail!("DAMM buy request has no DAMM pool parameters");
        };
        ensure!(
            pool.swap_mode == 0 && pool_snapshot(pool) == pool_snapshot(&self.plan.accounts.pool),
            "DAMM buy pool, programs or instruction context changed"
        );
        ensure!(
            request.create_mint_ata || !self.evidence.output_initialized(),
            "DAMM buy skips target ATA creation although its quote needed initialization"
        );
        ensure!(
            request.fixed_output_token_amount == Some(self.minimum)
                && self.minimum == self.evidence.minimum(&self.plan, frozen.slippage_bps)?,
            "DAMM buy minimum differs from its evidenced net quote"
        );
        let fees = request.gas_fee_strategy.get_strategies(TradeType::Buy);
        ensure!(
            fees.len()
                == if context.provider == SwqosType::Default {
                    1
                } else {
                    2
                },
            "DAMM buy fee strategy layout changed"
        );
        for (lane, kind, fee) in fees {
            let tip = if lane == SwqosType::Default {
                0.0
            } else {
                ensure!(
                    lane == context.provider,
                    "DAMM buy has an unexpected fee provider"
                );
                frozen.tip_sol
            };
            ensure!(
                kind == GasFeeStrategyType::Normal
                    && fee.cu_limit == 300_000
                    && fee.cu_price == 1_000
                    && fee.tip == tip,
                "DAMM buy fee strategy differs from its frozen intent"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
