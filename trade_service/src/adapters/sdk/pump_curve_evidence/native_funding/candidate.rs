//! Read-only bank evidence entry. It has no execution owner, key or broadcast capability.
use super::{NativeBoundIntent, NativeFundingPlan, PumpCurveAccounts, Side};
use crate::{
    adapters::{
        rpc::{read_http::ReadHttpTransport, read_runtime::SharedReadRuntime},
        sdk::ordinary_lanes::ensure_requested_provider,
    },
    application::{ports::ordinary_journal::TradeJournalMode, trade_admission::AdmittedTrade},
    domain::trade::{Address, ExecutionMethod, ExecutionMode, Pool, TradeAmount, WSOL_MINT},
};
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::swqos::SwqosType;
use solana_sdk::{hash::Hash, pubkey::Pubkey};
use std::{num::NonZeroU64, time::Instant};

pub(crate) struct NativeEvidenceContext {
    pub minimum_slot: Option<u64>,
    pub blockhash: Hash,
    pub provider: SwqosType,
    pub buyback: Pubkey,
    pub tip_destination: Option<Pubkey>,
    pub deadline: Instant,
}

#[derive(Debug)]
pub(crate) struct NativeProfileUnsupported(&'static str);

impl std::fmt::Display for NativeProfileUnsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for NativeProfileUnsupported {}

pub(crate) async fn prepare_native_evidence<'a, T: ReadHttpTransport>(
    execution: &'a AdmittedTrade,
    runtime: &SharedReadRuntime<T>,
    url: &str,
    context: NativeEvidenceContext,
) -> Result<NativeBoundIntent<'a>> {
    let command = execution.command();
    let anchor = execution.reservation();
    let wallet: Pubkey = anchor
        .wallet_pubkey()
        .parse()
        .context("native Pump evidence wallet is invalid")?;
    anchor.ensure_wallet(&command.wallet_id, Address::from_bytes(wallet.to_bytes()))?;
    ensure!(
        anchor.mode() == TradeJournalMode::Durable
            && command.idempotency_key == anchor.idempotency_key()
            && command.route.pool == Pool::Pump
            && command.route.quote_mint.to_string() == WSOL_MINT
            && command.execution.method == ExecutionMethod::Sdk
            && command.execution.mode == ExecutionMode::Live
            && !command.aggregator_sol
            && command.route.sol_hop_pool.is_none()
            && command.route.sol_hop_pool_id.is_none()
            && matches!(
                command.amount,
                TradeAmount::BuySol { lamports: 1.. }
                    | TradeAmount::SellPercent { percent: 1..=99 }
            ),
        "native Pump evidence requires its original direct durable native intent"
    );
    ensure!(
        context.minimum_slot.is_none_or(|slot| slot > 0)
            && context.blockhash != Hash::default()
            && context.deadline > Instant::now(),
        "native Pump evidence context is missing or expired"
    );
    ensure_requested_provider(
        command.execution.use_mev,
        command.execution.mev_provider,
        context.provider,
    )?;
    let curve = PumpCurveAccounts::new(
        Pubkey::new_from_array(command.route.mint.to_bytes()),
        command
            .route
            .pool_id
            .map(|value| Pubkey::new_from_array(value.to_bytes())),
        sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT,
    )?;
    tokio::time::timeout_at(tokio::time::Instant::from_std(context.deadline), async {
        // The first read chooses a program only; the second full bank remains the authority.
        let discovered = if let Some(minimum) = context.minimum_slot {
            curve.read(runtime, url, minimum, context.deadline).await?
        } else {
            curve.discover(runtime, url, context.deadline).await?
        };
        if discovered.base_mint_layout.program != spl_token_interface::ID {
            return Err(NativeProfileUnsupported(
                "native Pump controlled profile requires classic SPL Token",
            )
            .into());
        }
        let funding = NativeFundingPlan::new(
            curve,
            wallet,
            discovered.base_mint_layout.program,
            command.amount.side(),
        )?
        .read(runtime, url, discovered.slot, context.deadline)
        .await?;
        if funding.curve.curve.mayhem
            || funding.curve.sharing != super::super::SharingPrefix::Absent
        {
            return Err(NativeProfileUnsupported(
                "native Pump controlled profile does not support sharing or mayhem",
            )
            .into());
        }
        if funding.side == Side::Buy && funding.user_base_units.is_none() {
            return Err(NativeProfileUnsupported(
                "native Pump controlled profile requires an existing user ATA",
            )
            .into());
        }
        let input = match command.amount {
            TradeAmount::BuySol { lamports } => lamports,
            TradeAmount::SellPercent { percent } => funding
                .user_base_units
                .and_then(|balance| balance.checked_mul(percent))
                .map(|units| units / 100)
                .context("native Pump evidence percentage balance is missing or overflows")?,
            _ => unreachable!("native intent checked before account reads"),
        };
        let observation = funding
            .observe_complete(
                runtime,
                url,
                NonZeroU64::new(input).context("native Pump evidence input rounds to zero")?,
                context.buyback,
                context.blockhash,
                command.execution.slippage_bps,
                context.provider,
                command.execution.tip_sol,
                context.tip_destination,
            )
            .await?;
        let bound = observation.bind_intent(execution)?;
        bound.record()?;
        bound.packet()?;
        Ok(bound)
    })
    .await
    .context("native Pump complete evidence deadline expired")?
}

#[cfg(test)]
mod tests;
