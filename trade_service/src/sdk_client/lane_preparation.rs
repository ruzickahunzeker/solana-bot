use super::{ExecuteTrade, SdkLane, SdkTradeClient};
use crate::adapters::sdk::damm_probe::{DammBuyContext, DammBuyQuote, DammOutputEvidence};
use crate::adapters::sdk::ordinary_preparation::OrdinaryPreparationSource;
use crate::{
    application::ports::preparation_balance::PreparationBalanceRead,
    domain::{
        preparation_balance::{reject_balance, PreparationBalanceSnapshot},
        trade::Address,
    },
};
use sol_trade_sdk::trading::core::params::{DexParamEnum, MeteoraDammV2Params};
use solana_sdk::pubkey::Pubkey;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

enum CapturedDammOutput {
    Sell(Arc<DammOutputEvidence>),
    Buy(Arc<DammBuyQuote>),
}

pub(super) struct LanePreparation<'a> {
    wallet: &'a SdkTradeClient,
    lane: &'a SdkLane,
    balance: Mutex<Option<PreparationBalanceSnapshot>>,
    output: Mutex<Option<CapturedDammOutput>>,
    output_started: AtomicBool,
    read_rpc: &'a std::sync::Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
}

impl<'a> LanePreparation<'a> {
    pub(super) fn new(
        wallet: &'a SdkTradeClient,
        lane: &'a SdkLane,
        read_rpc: &'a std::sync::Arc<crate::adapters::rpc::read_runtime::SharedReadRuntime>,
    ) -> Self {
        Self {
            wallet,
            lane,
            balance: Mutex::new(None),
            output: Mutex::new(None),
            output_started: AtomicBool::new(false),
            read_rpc,
        }
    }

    pub(super) fn take_balance(&self) -> anyhow::Result<Option<PreparationBalanceSnapshot>> {
        self.balance
            .lock()
            .map(|mut value| value.take())
            .map_err(|_| reject_balance("preparation balance capture unavailable"))
    }

    pub(super) fn take_output(
        &self,
    ) -> anyhow::Result<Option<Arc<crate::adapters::sdk::damm_probe::DammOutputEvidence>>> {
        self.output
            .lock()
            .map_err(|_| reject_balance("DAMM output evidence capture unavailable"))?
            .take()
            .map(|output| match output {
                CapturedDammOutput::Sell(evidence) => Ok(evidence),
                CapturedDammOutput::Buy(_) => {
                    Err(reject_balance("DAMM buy quote cannot become sell evidence"))
                }
            })
            .transpose()
    }

    pub(super) fn take_buy_output(&self) -> anyhow::Result<Option<Arc<DammBuyQuote>>> {
        self.output
            .lock()
            .map_err(|_| reject_balance("DAMM output evidence capture unavailable"))?
            .take()
            .map(|output| match output {
                CapturedDammOutput::Buy(evidence) => Ok(evidence),
                CapturedDammOutput::Sell(_) => Err(reject_balance(
                    "DAMM sell evidence cannot become a buy quote",
                )),
            })
            .transpose()
    }
}

impl OrdinaryPreparationSource for LanePreparation<'_> {
    async fn load_params(&self, trade: &ExecuteTrade) -> anyhow::Result<DexParamEnum> {
        self.wallet
            .dex_params(
                &self.lane.client,
                trade.pool,
                &trade.mint,
                &trade.quote_mint,
                trade.pool_id.as_ref(),
                trade.sol_hop_pool,
                trade.sol_hop_pool_id.as_ref(),
            )
            .await
    }

    async fn token_balance(&self, mint: Pubkey) -> anyhow::Result<u64> {
        crate::adapters::sdk::preparation_balance::require_canonical_mode(
            self.lane.client.use_seed_optimize,
        )?;
        let wallet = Address::from_bytes(self.wallet.wallet().to_bytes());
        let mint = Address::from_bytes(mint.to_bytes());
        let source = crate::adapters::rpc::preparation_balance::RpcPreparationBalanceRead::new(
            wallet,
            self.lane.client.get_rpc().url(),
            self.read_rpc.clone(),
        );
        let snapshot = source.snapshot(mint).await?;
        let units = snapshot.checked_units(wallet, mint)?;
        let mut capture = self
            .balance
            .lock()
            .map_err(|_| reject_balance("preparation balance capture unavailable"))?;
        if capture.is_some() {
            return Err(reject_balance(
                "preparation attempted more than one balance capture",
            ));
        }
        *capture = Some(snapshot);
        Ok(units)
    }

    async fn mint_account_exists(&self, mint: Pubkey) -> anyhow::Result<bool> {
        self.read_rpc
            .preparation_account_exists(
                &self.lane.client.get_rpc().url(),
                Address::from_bytes(self.wallet.wallet().to_bytes()),
                Address::from_bytes(mint.to_bytes()),
            )
            .await
    }

    async fn damm_sell_output(
        &self,
        trade: &ExecuteTrade,
        params: &MeteoraDammV2Params,
        input_amount: u64,
    ) -> anyhow::Result<u64> {
        if self.output_started.swap(true, Ordering::AcqRel) {
            return Err(reject_balance(
                "DAMM output capture cannot be repeated or renewed",
            ));
        }
        let min_context_slot = self
            .balance
            .lock()
            .map_err(|_| reject_balance("preparation balance capture unavailable"))?
            .as_ref()
            .ok_or_else(|| reject_balance("DAMM quote has no captured balance evidence"))?
            .checked_slot(
                Address::from_bytes(self.wallet.wallet().to_bytes()),
                Address::from_bytes(trade.mint.to_bytes()),
            )?;
        let evidence = self
            .wallet
            .simulate_damm_v2_sell_output(
                self.lane,
                params,
                trade.mint,
                trade.quote_mint,
                input_amount,
                trade.recent_blockhash,
                min_context_slot,
            )
            .await?;
        let units = evidence.output_units();
        let mut capture = self
            .output
            .lock()
            .map_err(|_| reject_balance("DAMM output evidence capture unavailable"))?;
        if capture.is_some() {
            return Err(reject_balance(
                "DAMM preparation attempted more than one output capture",
            ));
        }
        *capture = Some(CapturedDammOutput::Sell(Arc::new(evidence)));
        Ok(units)
    }

    async fn damm_buy_output(
        &self,
        trade: &ExecuteTrade,
        params: &MeteoraDammV2Params,
        input_amount: u64,
    ) -> anyhow::Result<u64> {
        if self.output_started.swap(true, Ordering::AcqRel) {
            return Err(reject_balance(
                "DAMM output capture cannot be repeated or renewed",
            ));
        }
        crate::adapters::sdk::preparation_balance::require_canonical_mode(
            self.lane.client.use_seed_optimize,
        )?;
        let context = DammBuyContext {
            wallet: self.wallet.wallet(),
            provider: self.lane.swqos_type,
            version: self.lane.client.transaction_version,
            seed_optimized: self.lane.client.use_seed_optimize,
        };
        if trade.amount_lamports != Some(input_amount) {
            return Err(reject_balance(
                "DAMM buy quote input differs from ordinary preparation",
            ));
        }
        let quote =
            Arc::new(DammBuyQuote::read(context, trade, params, self.lane.client.get_rpc()).await?);
        let units = quote.output_units();
        let (account_slot, simulation_slot, probe_fee_lamports) = quote.context();
        tracing::debug!(
            account_slot,
            simulation_slot,
            probe_fee_lamports,
            "DAMM buy net quote prepared; not a final signing authorization"
        );
        let mut capture = self
            .output
            .lock()
            .map_err(|_| reject_balance("DAMM output evidence capture unavailable"))?;
        if capture.is_some() {
            return Err(reject_balance(
                "DAMM preparation attempted more than one output capture",
            ));
        }
        *capture = Some(CapturedDammOutput::Buy(quote));
        Ok(units)
    }
}
