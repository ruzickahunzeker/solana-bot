use crate::{
    application::trade_command::TradeCommand, config::PumpApiConfig, domain::trade::Address,
};

use super::request::PumpApiRequest;

/// Owned local intent, not evidence that the external message is authorized.
pub struct PumpApiTrade {
    config: PumpApiConfig,
    command: TradeCommand,
    wait_tx_confirmed: bool,
}

impl PumpApiTrade {
    pub(crate) fn new(
        command: &TradeCommand,
        config: PumpApiConfig,
        wait_tx_confirmed: bool,
    ) -> Self {
        Self {
            config,
            command: command.clone(),
            wait_tx_confirmed,
        }
    }

    pub(crate) fn command(&self) -> &TradeCommand {
        &self.command
    }
    pub(crate) fn config(&self) -> &PumpApiConfig {
        &self.config
    }
    pub(crate) fn simulate(&self) -> bool {
        self.command.execution.mode.simulate()
    }
    pub(crate) fn wait_tx_confirmed(&self) -> bool {
        self.wait_tx_confirmed
    }
}

pub(crate) fn pumpapi_trade(
    command: &TradeCommand,
    config: PumpApiConfig,
    wait_tx_confirmed: bool,
) -> PumpApiTrade {
    PumpApiTrade::new(command, config, wait_tx_confirmed)
}

pub(crate) fn pumpapi_request(wallet: Address, trade: &PumpApiTrade) -> PumpApiRequest {
    let command = trade.command();
    PumpApiRequest {
        wallet,
        side: command.amount.side(),
        mint: command.route.mint,
        quote_mint: command.quote_mint(),
        pool_id: command.route.pool_id,
        amount_units: command.amount.buy_units(),
        sell_percent: command.amount.sell_percent(),
        slippage_bps: command.execution.slippage_bps,
        priority_fee_sol: command.pumpapi_priority_fee_sol,
    }
}

#[cfg(test)]
mod tests;
