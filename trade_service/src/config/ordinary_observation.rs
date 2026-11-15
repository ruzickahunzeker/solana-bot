use anyhow::{bail, Result};

#[derive(Debug, Clone)]
pub(crate) struct OrdinaryObservationConfig {
    pub(crate) enabled: bool,
    pub(crate) batch_size: usize,
    pub(crate) poll_interval_ms: u64,
    pub(crate) min_age_ms: u64,
    pub(crate) request_interval_ms: u64,
}

impl Default for OrdinaryObservationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            batch_size: 8,
            poll_interval_ms: 30_000,
            min_age_ms: 30_000,
            request_interval_ms: 1_000,
        }
    }
}

impl OrdinaryObservationConfig {
    pub(crate) fn validate(&self) -> Result<()> {
        for (name, value, min, max) in [
            (
                "ORDINARY_OBSERVATION_BATCH_SIZE",
                self.batch_size as u64,
                1,
                64,
            ),
            (
                "ORDINARY_OBSERVATION_POLL_INTERVAL_MS",
                self.poll_interval_ms,
                1_000,
                300_000,
            ),
            (
                "ORDINARY_OBSERVATION_MIN_AGE_MS",
                self.min_age_ms,
                1_000,
                3_600_000,
            ),
            (
                "ORDINARY_OBSERVATION_REQUEST_INTERVAL_MS",
                self.request_interval_ms,
                1_000,
                60_000,
            ),
        ] {
            if !(min..=max).contains(&value) {
                bail!("{name} must be between {min} and {max}");
            }
        }
        Ok(())
    }
    pub(crate) fn from_env() -> Result<Self> {
        Self::read(|key| match std::env::var(key) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => bail!("{key} must be UTF-8"),
        })
    }

    fn read(read: impl Fn(&str) -> Result<Option<String>>) -> Result<Self> {
        let defaults = Self::default();
        let enabled = match read("ORDINARY_OBSERVATION_ENABLED")?.as_deref() {
            None | Some("false") | Some("0") => false,
            Some("true") | Some("1") => true,
            _ => bail!("ORDINARY_OBSERVATION_ENABLED must be true, false, 1 or 0"),
        };
        let number = |name: &str, default: u64, min: u64, max: u64| -> Result<u64> {
            let value = read(name)?
                .map(|raw| raw.parse::<u64>())
                .transpose()
                .map_err(|_| anyhow::anyhow!("{name} must be an integer"))?
                .unwrap_or(default);
            if !(min..=max).contains(&value) {
                bail!("{name} must be between {min} and {max}");
            }
            Ok(value)
        };
        Ok(Self {
            enabled,
            batch_size: number(
                "ORDINARY_OBSERVATION_BATCH_SIZE",
                defaults.batch_size as u64,
                1,
                64,
            )? as usize,
            poll_interval_ms: number(
                "ORDINARY_OBSERVATION_POLL_INTERVAL_MS",
                defaults.poll_interval_ms,
                1_000,
                300_000,
            )?,
            min_age_ms: number(
                "ORDINARY_OBSERVATION_MIN_AGE_MS",
                defaults.min_age_ms,
                1_000,
                3_600_000,
            )?,
            request_interval_ms: number(
                "ORDINARY_OBSERVATION_REQUEST_INTERVAL_MS",
                defaults.request_interval_ms,
                1_000,
                60_000,
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_scan_config_is_opt_in_and_bounds_all_timers_and_pages() {
        let defaults = OrdinaryObservationConfig::read(|_| Ok(None)).unwrap();
        assert!(!defaults.enabled);
        assert_eq!(
            (
                defaults.batch_size,
                defaults.poll_interval_ms,
                defaults.min_age_ms,
                defaults.request_interval_ms
            ),
            (8, 30_000, 30_000, 1_000)
        );
        for (name, bad) in [
            ("ORDINARY_OBSERVATION_ENABLED", "yes"),
            ("ORDINARY_OBSERVATION_BATCH_SIZE", "0"),
            ("ORDINARY_OBSERVATION_BATCH_SIZE", "65"),
            ("ORDINARY_OBSERVATION_POLL_INTERVAL_MS", "999"),
            ("ORDINARY_OBSERVATION_MIN_AGE_MS", "3600001"),
            ("ORDINARY_OBSERVATION_REQUEST_INTERVAL_MS", "999"),
            ("ORDINARY_OBSERVATION_REQUEST_INTERVAL_MS", "-1"),
        ] {
            assert!(
                OrdinaryObservationConfig::read(|key| Ok((key == name).then(|| bad.to_owned())))
                    .is_err(),
                "{name}"
            );
        }
    }
}
