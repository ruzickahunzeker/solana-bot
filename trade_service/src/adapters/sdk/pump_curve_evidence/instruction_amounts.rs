//! Locked SDK amount encoding only, not a protocol/fee/account authorization.
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::instruction::utils::pumpfun::{
    BUY_DISCRIMINATOR, BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR, BUY_EXACT_SOL_IN_DISCRIMINATOR,
    BUY_V2_DISCRIMINATOR, SELL_DISCRIMINATOR, SELL_V2_DISCRIMINATOR,
};
use std::num::NonZeroU64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WireLayout {
    NativeV1,
    QuoteV2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TradeAmounts {
    BuyExactInput {
        spend: NonZeroU64,
        minimum: NonZeroU64,
    },
    BuyExactOutput {
        output: NonZeroU64,
        maximum_spend: NonZeroU64,
    },
    SellExactInput {
        input: NonZeroU64,
        minimum: NonZeroU64,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct PumpWireAmounts {
    pub(super) layout: WireLayout,
    pub(super) amounts: TradeAmounts,
    pub(super) track_volume: Option<bool>,
}

impl PumpWireAmounts {
    pub(super) fn decode(data: &[u8]) -> Result<Self> {
        let discriminator: [u8; 8] = data
            .get(..8)
            .context("Pump instruction discriminator is missing")?
            .try_into()?;
        let (layout, buy, exact_input) = match discriminator {
            BUY_EXACT_SOL_IN_DISCRIMINATOR => (WireLayout::NativeV1, true, true),
            BUY_DISCRIMINATOR => (WireLayout::NativeV1, true, false),
            SELL_DISCRIMINATOR => (WireLayout::NativeV1, false, true),
            BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR => (WireLayout::QuoteV2, true, true),
            BUY_V2_DISCRIMINATOR => (WireLayout::QuoteV2, true, false),
            SELL_V2_DISCRIMINATOR => (WireLayout::QuoteV2, false, true),
            _ => anyhow::bail!("Pump instruction amount variant is not audited"),
        };
        let legacy_buy = buy && layout == WireLayout::NativeV1;
        ensure!(
            data.len() == if legacy_buy { 25 } else { 24 },
            "Pump instruction differs from the locked SDK wire length"
        );
        let track_volume = if legacy_buy {
            ensure!(data[24] <= 1, "Pump track volume flag is invalid");
            Some(data[24] == 1)
        } else {
            None
        };
        let first = NonZeroU64::new(u64::from_le_bytes(data[8..16].try_into()?))
            .context("Pump first amount is zero")?;
        let second = NonZeroU64::new(u64::from_le_bytes(data[16..24].try_into()?))
            .context("Pump second amount is zero")?;
        let amounts = match (buy, exact_input) {
            (true, true) => TradeAmounts::BuyExactInput {
                spend: first,
                minimum: second,
            },
            (true, false) => TradeAmounts::BuyExactOutput {
                output: first,
                maximum_spend: second,
            },
            (false, _) => TradeAmounts::SellExactInput {
                input: first,
                minimum: second,
            },
        };
        Ok(Self {
            layout,
            amounts,
            track_volume,
        })
    }

    fn require_exact_input_buy(&self, spend: NonZeroU64, minimum: NonZeroU64) -> Result<()> {
        ensure!(
            self.amounts == TradeAmounts::BuyExactInput { spend, minimum },
            "Pump buy is not the frozen exact-input spend and minimum"
        );
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests;
