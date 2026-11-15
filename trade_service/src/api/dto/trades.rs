use serde::{Deserialize, Serialize};

use crate::{
    application::trade_input::TradeInput,
    application::trade_record::TradeRecordV1 as TradeRecord,
    domain::trade::{ExecutionMethod, MevProvider, Pool, SettlementAsset, Side, TradeStatus},
};

#[cfg(test)]
use crate::application::{
    trade_command::{CommandContext, TradeCommand},
    trade_settings::TradeCommandDefaults,
    validated_trade::ValidatedTrade as NormalizedTrade,
};

#[derive(Deserialize)]
pub(crate) struct ListTradesQuery {
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TradeRequest {
    pub wallet_id: Option<String>,
    pub side: Side,
    pub pool: Pool,
    pub mint: String,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
    pub settlement_asset: Option<SettlementAsset>,
    pub sol_hop_pool: Option<Pool>,
    pub sol_hop_pool_id: Option<String>,
    pub amount_sol: Option<f64>,
    pub amount_lamports: Option<u64>,
    pub amount_quote: Option<f64>,
    pub amount_usdc: Option<f64>,
    pub amount_quote_units: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_percent: Option<f64>,
    pub slippage_bps: Option<u64>,
    pub tip_sol: Option<f64>,
    pub execution_method: Option<ExecutionMethod>,
    pub use_mev: Option<bool>,
    pub mev_provider: Option<MevProvider>,
    pub simulate: Option<bool>,
    pub source: Option<String>,
    pub source_signature: Option<String>,
    pub idempotency_key: String,
}

impl TradeRequest {
    pub(crate) fn validation_input(&self) -> TradeInput<'_> {
        TradeInput {
            side: self.side,
            pool: self.pool,
            mint: &self.mint,
            pool_id: self.pool_id.as_deref(),
            quote_mint: self.quote_mint.as_deref(),
            settlement_asset: self.settlement_asset,
            sol_hop_pool: self.sol_hop_pool,
            sol_hop_pool_id: self.sol_hop_pool_id.as_deref(),
            amount_sol: self.amount_sol,
            amount_lamports: self.amount_lamports,
            amount_quote: self.amount_quote,
            amount_usdc: self.amount_usdc,
            amount_quote_units: self.amount_quote_units,
            sell_percent: self.sell_percent,
            slippage_percent: self.slippage_percent,
            slippage_bps: self.slippage_bps,
            tip_sol: self.tip_sol,
            execution_method: self.execution_method,
            use_mev: self.use_mev,
            mev_provider: self.mev_provider,
            simulate: self.simulate,
            idempotency_key: &self.idempotency_key,
        }
    }

    #[cfg(test)]
    pub(crate) fn to_command(
        &self,
        wallet_id: String,
        value: &NormalizedTrade,
        defaults: TradeCommandDefaults,
    ) -> anyhow::Result<TradeCommand> {
        TradeCommand::from_validated(
            CommandContext {
                wallet_id,
                idempotency_key: &self.idempotency_key,
                side: self.side,
                pool: self.pool,
                default_simulate: defaults.default_simulate,
                // PumpAPI has always used a different default than the SDK tip.
                pumpapi_priority_fee_sol: self.tip_sol.unwrap_or(defaults.pumpapi_priority_fee_sol),
            },
            value,
        )
    }
}

#[derive(Debug, Serialize)]
pub struct TradeResponse {
    pub request_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub execution_method: ExecutionMethod,
    pub status: TradeStatus,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub duplicate: bool,
}

impl From<TradeRecord> for TradeResponse {
    fn from(record: TradeRecord) -> Self {
        Self {
            request_id: record.request_id,
            wallet_id: record.wallet_id,
            wallet_pubkey: record.wallet_pubkey,
            execution_method: record.execution_method,
            status: record.status,
            signature: record.signature,
            error: record.error,
            duplicate: record.duplicate,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BatchTradeRequest {
    pub wallet_ids: Vec<String>,
    pub trade: TradeRequest,
}

#[derive(Debug, Serialize)]
pub struct BatchTradeResponse {
    pub items: Vec<BatchTradeItemResponse>,
}

#[derive(Debug, Serialize)]
pub struct BatchTradeItemResponse {
    pub wallet_id: String,
    pub request_id: Option<String>,
    pub status: Option<TradeStatus>,
    pub error: Option<String>,
    pub duplicate: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_view_borrows_all_policy_fields_without_wallet_or_source_metadata() {
        let request = TradeRequest {
            wallet_id: Some("not-a-validation-field".to_owned()),
            side: Side::Sell,
            pool: Pool::StonkFunSwap,
            mint: "literal mint".to_owned(),
            pool_id: Some("literal pool".to_owned()),
            quote_mint: Some("literal quote".to_owned()),
            settlement_asset: Some(SettlementAsset::Sol),
            sol_hop_pool: Some(Pool::RaydiumCpmm),
            sol_hop_pool_id: Some("literal hop".to_owned()),
            amount_sol: Some(0.001234567),
            amount_lamports: Some(1_234_567),
            amount_quote: Some(1.234567),
            amount_usdc: Some(2.345678),
            amount_quote_units: Some(u64::MAX),
            sell_percent: Some(37),
            slippage_percent: Some(8.76),
            slippage_bps: Some(876),
            tip_sol: Some(0.000321),
            execution_method: Some(ExecutionMethod::Pumpapi),
            use_mev: Some(false),
            mev_provider: Some(MevProvider::Temporal),
            simulate: Some(false),
            source: Some("metadata only".to_owned()),
            source_signature: Some("not a parsed address".to_owned()),
            idempotency_key: " literal key ".to_owned(),
        };
        let input = request.validation_input();
        assert_eq!(input.side, request.side);
        assert_eq!(input.pool, request.pool);
        assert_eq!(input.mint, request.mint);
        assert_eq!(input.mint.as_ptr(), request.mint.as_ptr());
        assert_eq!(input.pool_id, request.pool_id.as_deref());
        assert_eq!(
            input.pool_id.unwrap().as_ptr(),
            request.pool_id.as_ref().unwrap().as_ptr()
        );
        assert_eq!(input.quote_mint, request.quote_mint.as_deref());
        assert_eq!(
            input.quote_mint.unwrap().as_ptr(),
            request.quote_mint.as_ref().unwrap().as_ptr()
        );
        assert_eq!(input.settlement_asset, request.settlement_asset);
        assert_eq!(input.sol_hop_pool, request.sol_hop_pool);
        assert_eq!(input.sol_hop_pool_id, request.sol_hop_pool_id.as_deref());
        assert_eq!(
            input.sol_hop_pool_id.unwrap().as_ptr(),
            request.sol_hop_pool_id.as_ref().unwrap().as_ptr()
        );
        assert_eq!(input.amount_sol, request.amount_sol);
        assert_eq!(input.amount_lamports, request.amount_lamports);
        assert_eq!(input.amount_quote, request.amount_quote);
        assert_eq!(input.amount_usdc, request.amount_usdc);
        assert_eq!(input.amount_quote_units, request.amount_quote_units);
        assert_eq!(input.sell_percent, request.sell_percent);
        assert_eq!(input.slippage_percent, request.slippage_percent);
        assert_eq!(input.slippage_bps, request.slippage_bps);
        assert_eq!(input.tip_sol, request.tip_sol);
        assert_eq!(input.execution_method, request.execution_method);
        assert_eq!(input.use_mev, request.use_mev);
        assert_eq!(input.mev_provider, request.mev_provider);
        assert_eq!(input.simulate, request.simulate);
        assert_eq!(input.idempotency_key, request.idempotency_key);
        assert_eq!(
            input.idempotency_key.as_ptr(),
            request.idempotency_key.as_ptr()
        );

        let absent: TradeRequest = serde_json::from_value(serde_json::json!({
            "side": "buy", "pool": "pump", "mint": "literal", "idempotency_key": "key"
        }))
        .unwrap();
        let view = absent.validation_input();
        assert!(view.pool_id.is_none());
        assert!(view.quote_mint.is_none());
        assert!(view.settlement_asset.is_none());
        assert!(view.sol_hop_pool.is_none());
        assert!(view.sol_hop_pool_id.is_none());
        assert!(view.amount_sol.is_none());
        assert!(view.amount_lamports.is_none());
        assert!(view.amount_quote.is_none());
        assert!(view.amount_usdc.is_none());
        assert!(view.amount_quote_units.is_none());
        assert!(view.sell_percent.is_none());
        assert!(view.slippage_percent.is_none());
        assert!(view.slippage_bps.is_none());
        assert!(view.tip_sol.is_none());
        assert!(view.execution_method.is_none());
        assert!(view.use_mev.is_none());
        assert!(view.mev_provider.is_none());
        assert!(view.simulate.is_none());
    }
}

#[cfg(test)]
mod command_defaults_tests;
