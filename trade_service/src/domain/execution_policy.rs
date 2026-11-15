use anyhow::{bail, Result};

pub(crate) const BROADCAST_DISABLED_ERROR: &str =
    "transaction broadcasting is disabled by server policy";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BroadcastPolicy {
    #[default]
    Enabled,
    Disabled,
}

impl BroadcastPolicy {
    pub fn ensure_allowed(self) -> Result<()> {
        if self == Self::Disabled {
            bail!(BROADCAST_DISABLED_ERROR);
        }
        Ok(())
    }

    pub fn ensure_trade_allowed(self, sdk_pipeline: bool, simulate: bool) -> Result<()> {
        if sdk_pipeline || !simulate {
            self.ensure_allowed()?;
        }
        Ok(())
    }
}
