use crate::domain::execution_policy::BroadcastPolicy;
use anyhow::{bail, Context, Result};

#[derive(Clone, Default)]
pub(crate) enum WalletAdminAuth {
    #[default]
    Legacy,
    Separate {
        token: String,
    },
}

impl std::fmt::Debug for WalletAdminAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Legacy => "Legacy",
            Self::Separate { .. } => "Separate(<redacted>)",
        })
    }
}

impl WalletAdminAuth {
    pub fn mode_name(&self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Separate { .. } => "separate",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SecurityConfig {
    pub broadcast: BroadcastPolicy,
    pub wallet_admin: WalletAdminAuth,
}

impl SecurityConfig {
    pub fn from_env(trade_token: &str) -> Result<Self> {
        let disabled = optional("TRADE_BROADCAST_DISABLED")?;
        let mode = optional("WALLET_ADMIN_AUTH_MODE")?;
        let token = optional("WALLET_ADMIN_API_TOKEN")?;
        Self::parse(
            disabled.as_deref(),
            mode.as_deref(),
            token.as_deref(),
            trade_token,
        )
    }

    pub fn parse(
        disabled: Option<&str>,
        mode: Option<&str>,
        token: Option<&str>,
        trade_token: &str,
    ) -> Result<Self> {
        let broadcast = match disabled.map(|s| s.trim().to_ascii_lowercase()) {
            None => BroadcastPolicy::Enabled,
            Some(value) if ["false", "0", "no", "off"].contains(&value.as_str()) => {
                BroadcastPolicy::Enabled
            }
            Some(value) if ["true", "1", "yes", "on"].contains(&value.as_str()) => {
                BroadcastPolicy::Disabled
            }
            _ => bail!("TRADE_BROADCAST_DISABLED must be a boolean"),
        };
        let wallet_admin = match mode
            .unwrap_or("legacy")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "legacy" => WalletAdminAuth::Legacy,
            "separate" => {
                let token = token
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .context("WALLET_ADMIN_AUTH_MODE=separate requires WALLET_ADMIN_API_TOKEN")?;
                if !token.bytes().all(|byte| byte.is_ascii_graphic()) {
                    bail!("WALLET_ADMIN_API_TOKEN must be a single printable ASCII Bearer token");
                }
                if token == trade_token.trim() {
                    bail!("WALLET_ADMIN_API_TOKEN must differ from LOCAL_TRADE_API_TOKEN");
                }
                WalletAdminAuth::Separate {
                    token: token.into(),
                }
            }
            _ => bail!("WALLET_ADMIN_AUTH_MODE must be legacy or separate"),
        };
        Ok(Self {
            broadcast,
            wallet_admin,
        })
    }
}

fn optional(key: &str) -> Result<Option<String>> {
    match std::env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{key} must contain valid Unicode"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_settings_preserve_legacy_behavior() {
        let config = SecurityConfig::parse(None, None, None, "trade-test").unwrap();
        assert_eq!(config.broadcast, BroadcastPolicy::Enabled);
        assert!(matches!(config.wallet_admin, WalletAdminAuth::Legacy));
    }

    #[test]
    fn explicit_policy_cannot_be_confused_with_simulation_defaults() {
        for value in ["true", "1", "on", "yes", " TRUE "] {
            assert_eq!(
                SecurityConfig::parse(Some(value), None, None, "trade-test")
                    .unwrap()
                    .broadcast,
                BroadcastPolicy::Disabled
            );
        }
        for value in ["false", "0", "off", "no"] {
            assert_eq!(
                SecurityConfig::parse(Some(value), None, None, "trade-test")
                    .unwrap()
                    .broadcast,
                BroadcastPolicy::Enabled
            );
        }
        for value in ["", "truthy", "disabled", "2"] {
            assert!(SecurityConfig::parse(Some(value), None, None, "trade-test").is_err());
        }
    }

    #[test]
    fn separate_admin_requires_distinct_nonempty_secret_and_valid_mode() {
        for token in [
            None,
            Some(""),
            Some("  "),
            Some(" trade-test "),
            Some("admin test"),
            Some("admin\nsecret"),
            Some("\u{00e9}secret"),
        ] {
            assert!(SecurityConfig::parse(None, Some("separate"), token, "trade-test").is_err());
        }
        for mode in ["", "seperate", "disabled"] {
            assert!(
                SecurityConfig::parse(None, Some(mode), Some("admin-test"), "trade-test").is_err()
            );
        }
        let config =
            SecurityConfig::parse(None, Some(" Separate "), Some(" admin-test "), "trade-test")
                .unwrap();
        assert!(
            matches!(&config.wallet_admin, WalletAdminAuth::Separate { token } if token == "admin-test")
        );
        assert!(!format!("{config:?}").contains("admin-test"));
    }

    #[test]
    fn staged_legacy_token_does_not_silently_enable_separate_mode() {
        let config =
            SecurityConfig::parse(None, Some("legacy"), Some("admin-test"), "trade-test").unwrap();
        assert!(matches!(config.wallet_admin, WalletAdminAuth::Legacy));
    }
}
