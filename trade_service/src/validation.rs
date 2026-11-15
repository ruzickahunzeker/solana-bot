use crate::adapters::sdk::address_parser::SdkAddressParser;
use crate::application::trade_settings::TradeValidationSettings;
use crate::application::trade_validation::TradeValidator;
use crate::config::Config;
use crate::domain::trade::Address;
use crate::models::{TradeRequest, WarmupRequest};

#[cfg(test)]
mod settings_tests;

pub use crate::application::validated_trade::ValidatedTrade as NormalizedTrade;
pub use crate::application::validated_trade::ValidatedWarmup as NormalizedWarmup;

pub(crate) fn validate_execution_policy(
    trade: &NormalizedTrade,
    config: &Config,
) -> anyhow::Result<()> {
    crate::application::trade_execution::ensure_execution_allowed(
        trade,
        config.enable_simulate,
        config.security.broadcast,
    )
}

pub fn validate_warmup(request: &WarmupRequest) -> anyhow::Result<NormalizedWarmup> {
    TradeValidator::new(&SdkAddressParser).validate_warmup(&request.validation_input())
}

pub(crate) fn parse_resolve_mint(value: &str) -> anyhow::Result<Address> {
    TradeValidator::new(&SdkAddressParser).parse_query_address("mint", value)
}

pub(crate) fn parse_hop_quote(value: &str) -> anyhow::Result<Address> {
    TradeValidator::new(&SdkAddressParser).parse_query_address("quote_mint", value)
}

pub fn validate_trade(request: &TradeRequest, config: &Config) -> anyhow::Result<NormalizedTrade> {
    validate_with_settings(
        request,
        &TradeValidationSettings {
            default_buy_sol: config.default_buy_sol,
            default_slippage_percent: config.default_slippage_percent,
            default_tip_sol: config.default_tip_sol,
            via_sol_enabled: config.stonkfun_via_sol.enabled,
            via_sol_max_buy_sol: config.stonkfun_via_sol.max_buy_sol,
        },
    )
}

pub(crate) fn validate_with_settings(
    request: &TradeRequest,
    settings: &TradeValidationSettings,
) -> anyhow::Result<NormalizedTrade> {
    TradeValidator::new(&SdkAddressParser).validate(&request.validation_input(), settings)
}

// Keep replay separate from current settings and admission rules.
pub(crate) fn replay_uses_sol_aggregator(request: &TradeRequest) -> anyhow::Result<bool> {
    TradeValidator::new(&SdkAddressParser).replay_uses_sol_aggregator(&request.validation_input())
}

pub(crate) fn replay_sol_observation_amount(
    request: &TradeRequest,
) -> anyhow::Result<(u64, Option<u64>)> {
    TradeValidator::new(&SdkAddressParser)
        .replay_sol_observation_amount(&request.validation_input())
}
