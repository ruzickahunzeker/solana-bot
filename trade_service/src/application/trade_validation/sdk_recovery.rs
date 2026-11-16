use super::*;
use crate::domain::sdk_candidate::DammCandidateDetails;
use crate::domain::{
    sdk_buy_candidate::{BuyFunding, DammBuyDetails},
    sdk_prepared_record::SdkPreparedRecord,
};
use anyhow::ensure;

impl TradeValidator<'_> {
    pub(crate) fn replay_sdk_candidate(
        &self,
        request: &TradeInput<'_>,
        candidate: &SdkPreparedRecord,
    ) -> anyhow::Result<ValidatedTrade> {
        match candidate {
            SdkPreparedRecord::DammSell(record) => {
                self.replay_sdk_observation(request, record.details())
            }
            SdkPreparedRecord::DammBuy(record) => self.replay_sdk_buy(request, record.details()),
        }
    }

    fn replay_sdk_buy(
        &self,
        request: &TradeInput<'_>,
        candidate: &DammBuyDetails,
    ) -> anyhow::Result<ValidatedTrade> {
        self.validate_amount_fields(request)?;
        let method = request.execution_method.unwrap_or(ExecutionMethod::Sdk);
        let settlement_asset = request.settlement_asset.unwrap_or(SettlementAsset::Quote);
        ensure!(
            request.side == Side::Buy
                && request.pool == Pool::MeteoraDammV2
                && method == ExecutionMethod::Sdk
                && request.simulate != Some(true)
                && !candidate.simulate
                && settlement_asset == SettlementAsset::Quote
                && request.sol_hop_pool.is_none()
                && request.sol_hop_pool_id.is_none(),
            "stored SDK observation is not a direct live DAMM buy"
        );
        let mint = self.parse_pubkey("mint", request.mint)?;
        let quote_mint = self.normalize_quote_mint(request.quote_mint)?;
        let pool_id = self.parse_optional_pubkey("pool_id", request.pool_id)?;
        ensure!(
            !self.uses_sol_aggregator(request, &quote_mint)
                && match candidate.funding {
                    BuyFunding::NativeSol => self.is_wsol_quote(&quote_mint),
                    BuyFunding::ExistingQuote => self.is_usdc_quote(&quote_mint),
                },
            "stored SDK buy funding or payment branch mismatch"
        );
        // An omitted historical native default is the original integer, never today's Config
        // or a float round trip. Explicit fields still use the unchanged V1 amount parser.
        let amount = self.normalize_amount_units(
            request,
            || Ok(candidate.input_units),
            &quote_mint,
            settlement_asset,
        )?;
        let slippage_bps = if let Some(percent) = request.slippage_percent {
            self.percent_to_bps("slippage_percent", percent)?
        } else {
            request.slippage_bps.unwrap_or(candidate.slippage_bps)
        };
        let tip_sol = request
            .tip_sol
            .unwrap_or(f64::from_bits(candidate.tip_sol_bits));
        let (use_mev, mev_provider) = self.normalize_mev_override(request)?;
        ensure!(
            mint.to_bytes() == candidate.mint
                && quote_mint.to_bytes() == candidate.quote_mint
                && pool_id.map(Address::to_bytes) == candidate.pool_hint
                && amount > 0
                && amount == candidate.input_units
                && slippage_bps == candidate.slippage_bps
                && slippage_bps < 10_000
                && tip_sol.is_finite()
                && tip_sol >= 0.0
                && tip_sol.to_bits() == candidate.tip_sol_bits
                && use_mev == candidate.use_mev
                && mev_provider == candidate.requested_mev_provider,
            "stored SDK request differs from its original signed candidate"
        );
        Ok(ValidatedTrade {
            mint,
            quote_mint,
            pool_id,
            settlement_asset,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: Some(amount),
            sell_percent: None,
            slippage_bps,
            tip_sol,
            execution_method: method,
            use_mev,
            mev_provider,
            simulate: request.simulate,
            aggregator_sol: false,
        })
    }

    /// Historical intent only: defaults come from the signed candidate, not current settings.
    pub(crate) fn replay_sdk_observation(
        &self,
        request: &TradeInput<'_>,
        candidate: &DammCandidateDetails,
    ) -> anyhow::Result<ValidatedTrade> {
        self.validate_amount_fields(request)?;
        let method = request.execution_method.unwrap_or(ExecutionMethod::Sdk);
        ensure!(
            request.side == Side::Sell
                && request.pool == Pool::MeteoraDammV2
                && method == ExecutionMethod::Sdk
                && request.simulate != Some(true)
                && !candidate.simulate
                && request.sol_hop_pool.is_none()
                && request.sol_hop_pool_id.is_none(),
            "stored SDK observation is not a direct live DAMM sell"
        );
        let mint = self.parse_pubkey("mint", request.mint)?;
        let quote_mint = self.normalize_quote_mint(request.quote_mint)?;
        let pool_id = self.parse_optional_pubkey("pool_id", request.pool_id)?;
        let slippage_bps = if let Some(percent) = request.slippage_percent {
            self.percent_to_bps("slippage_percent", percent)?
        } else {
            request.slippage_bps.unwrap_or(candidate.slippage_bps)
        };
        let tip_sol = request
            .tip_sol
            .unwrap_or(f64::from_bits(candidate.tip_sol_bits));
        let (use_mev, mev_provider) = self.normalize_mev_override(request)?;
        ensure!(
            mint.to_bytes() == candidate.mint
                && quote_mint.to_bytes() == candidate.quote_mint
                && pool_id.map(Address::to_bytes) == candidate.pool_hint
                && request.sell_percent == Some(candidate.sell_percent)
                && slippage_bps == candidate.slippage_bps
                && slippage_bps <= 10_000
                && tip_sol.is_finite()
                && tip_sol >= 0.0
                && tip_sol.to_bits() == candidate.tip_sol_bits
                && use_mev == candidate.use_mev
                && mev_provider == candidate.requested_mev_provider,
            "stored SDK request differs from its original signed candidate"
        );
        Ok(ValidatedTrade {
            mint,
            quote_mint,
            pool_id,
            settlement_asset: request.settlement_asset.unwrap_or(SettlementAsset::Quote),
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            amount_lamports: None,
            sell_percent: request.sell_percent,
            slippage_bps,
            tip_sol,
            execution_method: method,
            use_mev,
            mev_provider,
            simulate: request.simulate,
            aggregator_sol: false,
        })
    }
}

#[cfg(test)]
mod tests;
