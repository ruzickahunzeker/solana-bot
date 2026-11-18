use super::{
    security::{ApiAccessPolicy, WalletAdminAuth},
    Config,
};
use crate::{
    application::{
        sol_transfers::SolTransferPolicy, trade_intake::TradeIntakeSettings,
        trade_settings::OrdinaryExecutionSettings, wsol_submission::WsolSubmitPolicy,
    },
    domain::execution_policy::BroadcastPolicy,
};
use std::net::IpAddr;

/// Immutable request settings, projected once without retaining startup resources.
pub(crate) struct RequestSettings {
    trade_token: String,
    allowed_client_ips: Vec<IpAddr>,
    wallet_admin: WalletAdminAuth,
    intake: TradeIntakeSettings,
    execution: OrdinaryExecutionSettings,
    trade_response_wait_ms: u64,
    system_wallet_transfer: bool,
    conversion_live: bool,
}

impl RequestSettings {
    pub(crate) fn from_config(config: &Config) -> Self {
        let access = config.api_access_policy();
        Self {
            trade_token: access.trade_token.into(),
            allowed_client_ips: access.allowed_client_ips.to_vec(),
            wallet_admin: access.wallet_admin.clone(),
            intake: config.trade_intake_settings(),
            execution: config.ordinary_execution_settings(),
            trade_response_wait_ms: config.trade_response_wait_ms,
            system_wallet_transfer: config.allow_internal_system_wallet_transfer,
            conversion_live: config.conversion_live_enabled,
        }
    }

    pub(crate) fn api_access_policy(&self) -> ApiAccessPolicy<'_> {
        ApiAccessPolicy {
            trade_token: &self.trade_token,
            allowed_client_ips: &self.allowed_client_ips,
            wallet_admin: &self.wallet_admin,
        }
    }

    pub(crate) fn intake(&self) -> TradeIntakeSettings {
        self.intake
    }

    pub(crate) fn execution(&self) -> OrdinaryExecutionSettings {
        self.execution
    }

    pub(crate) fn trade_response_wait_ms(&self) -> u64 {
        self.trade_response_wait_ms
    }

    pub(crate) fn stonkfun_via_sol_enabled(&self) -> bool {
        self.intake.validation.via_sol_enabled
    }

    pub(crate) fn broadcast(&self) -> BroadcastPolicy {
        self.intake.broadcast
    }

    pub(crate) fn sol_transfer_policy(&self) -> SolTransferPolicy {
        SolTransferPolicy {
            broadcast: self.broadcast(),
            allow_system_wallet: self.system_wallet_transfer,
        }
    }

    pub(crate) fn wsol_submit_policy(&self) -> WsolSubmitPolicy {
        WsolSubmitPolicy {
            broadcast: self.broadcast(),
            live_enabled: self.conversion_live,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
impl crate::AppState {
    pub(crate) fn edit_fixture_config(&mut self, edit: impl FnOnce(&mut Config)) {
        edit(std::sync::Arc::make_mut(&mut self.config));
        self.request_settings = std::sync::Arc::new(RequestSettings::from_config(&self.config));
    }

    pub(crate) fn replace_fixture_config(&mut self, config: std::sync::Arc<Config>) {
        self.request_settings = std::sync::Arc::new(RequestSettings::from_config(&config));
        self.config = config;
    }
}
