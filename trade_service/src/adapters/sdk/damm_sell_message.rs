use super::{
    ordinary_lanes::{SdkInvocationFailure, SdkLaneReport},
    preparation_input::{self, SdkInput},
    trade_command::ExecuteTrade,
};
use crate::domain::{
    preparation_balance::{reject_balance, PreparationBalanceSnapshot},
    trade::{Address, Pool, Side},
};
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::{
    common::{GasFeeStrategyType, TradeTransactionVersion},
    constants::{SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    swqos::{SwqosType, TradeType},
    trading::{core::params::DexParamEnum, factory::DexType, MiddlewareManager},
    TradeSellParams, TradeTokenType,
};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
use std::{future::Future, sync::Arc};

mod candidate;
mod guard;
mod instructions;
mod metadata;
mod simulation;
pub(crate) use candidate::DammSellCandidate;
use guard::Invocation;
#[cfg(test)]
pub(crate) use metadata::candidate_fixture;
pub(crate) use metadata::validate_candidate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DammSellContext {
    pub(crate) wallet: Pubkey,
    pub(crate) provider: SwqosType,
    pub(crate) version: TradeTransactionVersion,
    pub(crate) seed_optimized: bool,
}

/// Frozen nominal SDK candidate, not complete pool/fee authority or a signed journal.
pub(crate) struct DammSellMessageProfile {
    context: DammSellContext,
    intent: ExecuteTrade,
    holding: Arc<PreparationBalanceSnapshot>,
    balance: u64,
    percent: u64,
    minimum: u64,
    quote: Arc<super::damm_probe::DammOutputEvidence>,
    protocol: Vec<Instruction>,
    pool_metadata: Option<crate::domain::sdk_candidate::DammCandidatePool>,
}

impl DammSellMessageProfile {
    pub(crate) fn validate_intent(&self, current: &ExecuteTrade) -> Result<()> {
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
            "DAMM frozen message profile belongs to a different execution intent"
        );
        Ok(())
    }

    pub(crate) fn new(
        context: DammSellContext,
        intent: &ExecuteTrade,
        request: &TradeSellParams,
        balance: u64,
        percent: u64,
        holding: Arc<PreparationBalanceSnapshot>,
        quote: Arc<super::damm_probe::DammOutputEvidence>,
    ) -> Result<Arc<Self>> {
        let minimum = request
            .fixed_output_token_amount
            .filter(|amount| *amount > 0)
            .ok_or_else(|| reject_balance("DAMM final minimum output is missing or zero"))?;
        let mut profile = Self {
            context,
            intent: intent.clone(),
            holding,
            balance,
            percent,
            minimum,
            quote,
            protocol: vec![],
            pool_metadata: None,
        };
        profile.validate_fields(context, request, balance, percent)?;
        profile.pool_metadata = Some(metadata::pool_snapshot(request)?);
        profile.protocol =
            instructions::protocol(context.wallet, request, profile.input_amount()?, minimum)?;
        profile
            .quote
            .validate_details(&metadata::details(&profile, None)?)?;
        Ok(Arc::new(profile))
    }

    fn input_amount(&self) -> Result<u64> {
        let amount = self
            .balance
            .checked_mul(self.percent)
            .context("DAMM sell input amount overflow")?
            / 100;
        ensure!(amount > 0, "DAMM sell input amount rounds to zero");
        Ok(amount)
    }

    fn recheck(&self) -> Result<()> {
        self.holding
            .recheck(Address::from_bytes(self.context.wallet.to_bytes()))?;
        self.quote.validate_details(&metadata::details(self, None)?)
    }

    fn validate_fields(
        &self,
        context: DammSellContext,
        request: &TradeSellParams,
        balance: u64,
        percent: u64,
    ) -> Result<()> {
        ensure!(context == self.context, "DAMM execution context changed");
        ensure!(
            context.version == TradeTransactionVersion::V0 && !context.seed_optimized,
            "DAMM final profile requires the canonical V0-compatible SDK mode"
        );
        let intent = &self.intent;
        ensure!(
            intent.pool == Pool::MeteoraDammV2
                && intent.sol_hop_pool.is_none()
                && intent.sol_hop_pool_id.is_none()
                && intent.amount_lamports.is_none()
                && intent.sell_percent == Some(percent)
                && (1..=100).contains(&percent)
                && balance == self.balance
                && percent == self.percent,
            "DAMM final sell intent or percentage changed"
        );
        ensure!(
            request.dex_type == DexType::MeteoraDammV2
                && request.mint == intent.mint
                && request.input_token_amount == 0
                && request.slippage_basis_points == Some(intent.slippage_bps)
                && request.recent_blockhash == Some(intent.recent_blockhash)
                && intent.recent_blockhash != solana_sdk::hash::Hash::default()
                && request.wait_tx_confirmed == intent.wait_tx_confirmed
                && !request.wait_for_all_submits
                && request.with_tip
                && request.create_output_token_ata
                && !request.close_output_token_ata
                && request.close_mint_token_ata == (percent == 100)
                && request.address_lookup_table_accounts.is_empty()
                && request.durable_nonce.is_none()
                && request.fixed_output_token_amount == Some(self.minimum)
                && request.simulate == intent.simulate
                && request.grpc_recv_us.is_none(),
            "DAMM final request differs from the frozen ordinary profile"
        );
        let DexParamEnum::MeteoraDammV2(pool) = &request.extension_params else {
            return Err(reject_balance("DAMM final protocol parameters are missing"));
        };
        ensure!(
            pool.swap_mode == 0 && intent.pool_id.is_none_or(|pool_id| pool_id == pool.pool),
            "DAMM final pool or exact-input mode differs from intent"
        );
        let output = output_mint(request.output_token_type);
        ensure!(
            output == intent.quote_mint
                && [WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT].contains(&output),
            "DAMM final output asset differs from intent"
        );
        let wallet = Address::from_bytes(context.wallet.to_bytes());
        ensure!(
            self.holding
                .checked_units(wallet, Address::from_bytes(intent.mint.to_bytes()))?
                == balance,
            "DAMM final balance differs from captured evidence"
        );
        preparation_input::validate(
            Some(&self.holding),
            wallet,
            intent,
            Side::Sell,
            SdkInput::Sell(request),
        )?;
        self.input_amount()?;
        self.validate_fees(request)
    }

    fn validate_fees(&self, request: &TradeSellParams) -> Result<()> {
        let fees = request.gas_fee_strategy.get_strategies(TradeType::Sell);
        let provider = self.context.provider;
        let only_default = self.intent.simulate || provider == SwqosType::Default;
        ensure!(
            self.intent.tip_sol.is_finite()
                && self.intent.tip_sol >= 0.0
                && fees.len() == if only_default { 1 } else { 2 },
            "DAMM final fee strategy layout changed"
        );
        for (lane, kind, fee) in fees {
            let expected_tip = if lane == SwqosType::Default {
                0.0
            } else {
                ensure!(
                    !only_default && lane == provider,
                    "DAMM unexpected fee lane"
                );
                self.intent.tip_sol
            };
            ensure!(
                kind == GasFeeStrategyType::Normal
                    && fee.cu_limit == 300_000
                    && fee.cu_price == 1_000
                    && fee.tip == expected_tip,
                "DAMM final fee budget differs from intent"
            );
        }
        Ok(())
    }

    pub(crate) fn validate_request(
        &self,
        context: DammSellContext,
        request: &TradeSellParams,
        balance: u64,
        percent: u64,
    ) -> Result<()> {
        self.validate_fields(context, request, balance, percent)?;
        self.recheck()?;
        ensure!(
            self.protocol
                == instructions::protocol(
                    context.wallet,
                    request,
                    self.input_amount()?,
                    self.minimum,
                )?,
            "DAMM final protocol accounts or instructions changed"
        );
        Ok(())
    }

    pub(super) async fn invoke<F, Fut>(
        self: &Arc<Self>,
        invoke: F,
    ) -> Result<SdkLaneReport, SdkInvocationFailure>
    where
        F: FnOnce(MiddlewareManager) -> Fut + Send,
        Fut: Future<Output = Result<SdkLaneReport, SdkInvocationFailure>> + Send,
    {
        if self.intent.simulate {
            return Err(SdkInvocationFailure {
                signatures: vec![],
                message: "DAMM simulation must use the unsigned candidate path".into(),
            });
        }
        self.recheck().map_err(|error| SdkInvocationFailure {
            signatures: vec![],
            message: error.to_string(),
        })?;
        let invocation = Invocation::new(self.clone());
        let result = invoke(invocation.middleware()).await;
        invocation.finish(result)
    }
}

fn output_mint(asset: TradeTokenType) -> Pubkey {
    match asset {
        TradeTokenType::SOL | TradeTokenType::WSOL => WSOL_TOKEN_ACCOUNT,
        TradeTokenType::USDC => USDC_TOKEN_ACCOUNT,
        TradeTokenType::USD1 => sol_trade_sdk::constants::USD1_TOKEN_ACCOUNT,
        TradeTokenType::Token(mint) if mint == SOL_TOKEN_ACCOUNT => WSOL_TOKEN_ACCOUNT,
        TradeTokenType::Token(mint) => mint,
    }
}

#[cfg(test)]
mod tests;
