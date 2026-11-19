//! Original admission binding only, not a signing permit or a journal record.
use super::NativeFinalObservation;
use crate::{
    adapters::sdk::{
        ordinary_lanes::ensure_requested_provider,
        pump_curve_evidence::native_funding::Side as FundingSide,
    },
    application::{
        ports::ordinary_journal::TradeJournalMode,
        trade_admission::{AdmittedTrade, OrdinaryReservation},
    },
    domain::trade::{Address, ExecutionMethod, ExecutionMode, Pool, TradeAmount, WSOL_MINT},
};
use anyhow::{ensure, Context, Result};
use solana_sdk::transaction::VersionedTransaction;

struct NativeBoundIntent<'a> {
    observation: NativeFinalObservation,
    execution: &'a AdmittedTrade,
    anchor: &'a OrdinaryReservation,
}

fn check(observation: &NativeFinalObservation, execution: &AdmittedTrade) -> Result<()> {
    observation.packet()?;
    let envelope = &observation.template;
    let constraints = &envelope.source;
    let probe = &constraints.source.observation.probe;
    let funding = &probe.funding;
    let command = execution.command();
    let anchor = execution.reservation();
    anchor.ensure_wallet(
        &command.wallet_id,
        Address::from_bytes(funding.keys[6].to_bytes()),
    )?;
    ensure!(
        !anchor.request_id().is_empty()
            && !anchor.request_hash().is_empty()
            && command.idempotency_key == anchor.idempotency_key()
            && anchor.mode() == TradeJournalMode::Durable
            && command.route.pool == Pool::Pump
            && command.execution.method == ExecutionMethod::Sdk
            && command.execution.mode == ExecutionMode::Live
            && !command.aggregator_sol
            && command.route.sol_hop_pool.is_none()
            && command.route.sol_hop_pool_id.is_none()
            && command.route.mint.to_bytes() == funding.keys[1].to_bytes()
            && command.route.quote_mint.to_string() == WSOL_MINT
            && command
                .route
                .pool_id
                .is_none_or(|pool| pool.to_bytes() == funding.keys[0].to_bytes()),
        "native Pump evidence differs from its original direct durable intent"
    );
    let input = match command.amount {
        TradeAmount::BuySol { lamports } if funding.side == FundingSide::Buy => lamports,
        TradeAmount::SellPercent { percent }
            if funding.side == FundingSide::Sell && (1..=99).contains(&percent) =>
        {
            funding
                .user_base_units
                .and_then(|balance| balance.checked_mul(percent))
                .map(|units| units / 100)
                .context("native Pump original percentage balance is missing or overflows")?
        }
        _ => anyhow::bail!(
            "native Pump intent requires native buy or partial sell without account closure"
        ),
    };
    ensure!(
        input > 0
            && input == probe.input.get()
            && command.execution.slippage_bps == constraints.slippage_bps
            && command.execution.tip_sol.to_bits() == envelope.tip_sol.to_bits(),
        "native Pump amount, percentage or fee intent changed"
    );
    ensure_requested_provider(
        command.execution.use_mev,
        command.execution.mev_provider,
        envelope.provider,
    )
    .map_err(|_| {
        anyhow::anyhow!("native Pump selected provider differs from its original MEV policy")
    })
}

impl NativeFinalObservation {
    fn bind_intent(self, execution: &AdmittedTrade) -> Result<NativeBoundIntent<'_>> {
        check(&self, execution)?;
        Ok(NativeBoundIntent {
            observation: self,
            execution,
            anchor: execution.reservation(),
        })
    }
}

impl NativeBoundIntent<'_> {
    fn packet(&self) -> Result<&VersionedTransaction> {
        ensure!(
            std::ptr::eq(self.anchor, self.execution.reservation()),
            "native Pump admission identity changed after binding"
        );
        check(&self.observation, self.execution)?;
        self.observation.packet()
    }
}

mod tests;
