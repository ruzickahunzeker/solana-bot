use crate::adapters::sdk::address_parser::SdkAddressParser;
#[cfg(test)]
use crate::application::trade_settings::TradeValidationSettings;
use crate::application::trade_validation::TradeValidator;
#[cfg(test)]
use crate::config::Config;
use crate::domain::trade::Address;
use crate::models::{TradeRequest, WarmupRequest};

#[cfg(test)]
mod settings_tests;

#[cfg(test)]
pub use crate::application::validated_trade::ValidatedTrade as NormalizedTrade;
pub use crate::application::validated_trade::ValidatedWarmup as NormalizedWarmup;

#[cfg(test)]
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

#[cfg(test)]
pub fn validate_trade(request: &TradeRequest, config: &Config) -> anyhow::Result<NormalizedTrade> {
    validate_with_settings(request, &config.trade_validation_settings())
}

#[cfg(test)]
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
