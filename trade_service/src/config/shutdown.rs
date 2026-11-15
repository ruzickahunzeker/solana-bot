use anyhow::{ensure, Result};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ShutdownSettings {
    pub(crate) trade_task_drain_timeout_ms: u64,
}

impl Default for ShutdownSettings {
    fn default() -> Self {
        Self {
            trade_task_drain_timeout_ms: 30_000,
        }
    }
}

impl ShutdownSettings {
    pub(crate) fn from_env() -> Result<Self> {
        Self::validated(super::parse_env("TRADE_TASK_DRAIN_TIMEOUT_MS", 30_000)?)
    }

    fn validated(value: u64) -> Result<Self> {
        ensure!(
            (1..=300_000).contains(&value),
            "TRADE_TASK_DRAIN_TIMEOUT_MS must be between 1 and 300000"
        );
        Ok(Self {
            trade_task_drain_timeout_ms: value,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trade_task_drain_configuration_is_bounded_without_changing_submission_settings() {
        assert_eq!(
            ShutdownSettings::default().trade_task_drain_timeout_ms,
            30_000
        );
        for valid in [1, 30_000, 300_000] {
            assert_eq!(
                ShutdownSettings::validated(valid)
                    .unwrap()
                    .trade_task_drain_timeout_ms,
                valid
            );
        }
        for invalid in [0, 300_001, u64::MAX] {
            assert!(ShutdownSettings::validated(invalid).is_err());
        }
    }
}
