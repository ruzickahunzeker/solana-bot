use super::{NativeFundingSnapshot, Side};
use anyhow::{ensure, Context, Result};
use std::num::NonZeroU64;

#[derive(Clone, Copy)]
enum NativeIntentAmount {
    BuyLamports(NonZeroU64),
    SellPercent(u64),
}

#[derive(Clone, Copy)]
struct CostCeilings {
    output_ata_rent: Option<NonZeroU64>,
    network_fee: NonZeroU64,
    relay_tip: u64,
    other_instruction_debits: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct NativeBudget {
    input_units: NonZeroU64,
    native_debit_ceiling: u64,
    native_remaining_at_ceiling: u64,
}

impl NativeFundingSnapshot {
    // Costs are caller-supplied bounds. This checks affordability, not the origin of those bounds.
    fn check_budget(
        &self,
        amount: NativeIntentAmount,
        costs: CostCeilings,
    ) -> Result<NativeBudget> {
        let (units, trade_lamports, needs_ata) = match (self.side, amount) {
            (Side::Buy, NativeIntentAmount::BuyLamports(lamports)) => {
                (lamports, lamports.get(), self.user_base_units.is_none())
            }
            (Side::Sell, NativeIntentAmount::SellPercent(percent)) => {
                ensure!(
                    (1..=100).contains(&percent),
                    "native Pump sell percent is invalid"
                );
                let balance = self
                    .user_base_units
                    .context("native Pump sell balance is missing")?;
                // Preserve the ordinary SDK's checked integer computation and floor.
                let units = balance
                    .checked_mul(percent)
                    .context("native Pump sell amount overflows")?
                    / 100;
                (
                    NonZeroU64::new(units).context("native Pump sell amount rounds to zero")?,
                    0,
                    false,
                )
            }
            _ => anyhow::bail!("native Pump funding direction differs from budget intent"),
        };
        ensure!(
            costs.output_ata_rent.is_some() == needs_ata,
            "native Pump output ATA rent does not match snapshot"
        );
        // Never credit a simulated sell output, a future close refund, or an unverified balance.
        let native_debit_ceiling = trade_lamports
            .checked_add(costs.output_ata_rent.map_or(0, NonZeroU64::get))
            .and_then(|amount| amount.checked_add(costs.other_instruction_debits))
            .and_then(|amount| amount.checked_add(costs.network_fee.get()))
            .and_then(|amount| amount.checked_add(costs.relay_tip))
            .context("native Pump debit ceiling overflows")?;
        let native_remaining_at_ceiling = self
            .native_lamports
            .checked_sub(native_debit_ceiling)
            .context("native Pump wallet cannot fund the debit ceiling")?;
        Ok(NativeBudget {
            input_units: units,
            native_debit_ceiling,
            native_remaining_at_ceiling,
        })
    }
}

#[cfg(test)]
mod tests;
