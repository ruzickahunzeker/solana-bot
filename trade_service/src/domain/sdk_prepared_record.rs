use super::{sdk_buy_candidate::DammBuyCandidateRecord, sdk_candidate::SdkCandidateMetadata};
use anyhow::{ensure, Result};
use serde::Deserialize;

/// Nominal data for the original journal, never an execution owner or signing permission.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SdkPreparedRecord {
    DammSell(SdkCandidateMetadata),
    DammBuy(DammBuyCandidateRecord),
}

impl SdkPreparedRecord {
    pub(crate) fn buy(&self) -> Result<&DammBuyCandidateRecord> {
        match self {
            Self::DammBuy(value) => Ok(value),
            Self::DammSell(_) => anyhow::bail!("SDK prepared record is not DAMM buy"),
        }
    }

    pub(crate) fn wallet(&self) -> [u8; 32] {
        match self {
            Self::DammSell(value) => value.details().wallet,
            Self::DammBuy(value) => value.details().wallet,
        }
    }

    pub(crate) fn sell(&self) -> Result<&SdkCandidateMetadata> {
        match self {
            Self::DammSell(value) => Ok(value),
            Self::DammBuy(_) => anyhow::bail!("SDK prepared record is not DAMM sell"),
        }
    }

    pub(crate) fn validate(&self, message: &[u8]) -> Result<()> {
        match self {
            Self::DammSell(value) => value.validate(message),
            Self::DammBuy(value) => value.validate(message),
        }
    }

    pub(crate) fn encode(&self, message: &[u8]) -> Result<String> {
        // No enum wrapper: historical SellV1 journal bytes must remain unchanged.
        match self {
            Self::DammSell(value) => value.encode(message),
            Self::DammBuy(value) => value.encode(message),
        }
    }

    pub(crate) fn decode(json: &str, message: &[u8]) -> Result<Self> {
        ensure!(json.len() <= 65_536, "SDK prepared record exceeds limit");
        #[derive(Deserialize)]
        struct Discriminator {
            version: u16,
            profile: String,
        }
        // Do not try one payload decoder after another or collapse duplicate keys into Value.
        // The selected strict decoder still checks the entire payload and message digest.
        let tag: Discriminator = serde_json::from_str(json)?;
        match (tag.version, tag.profile.as_str()) {
            (1, "damm-v2-sell-v1") => {
                Ok(Self::DammSell(SdkCandidateMetadata::decode(json, message)?))
            }
            (1, "damm-v2-buy-v1") => Ok(Self::DammBuy(DammBuyCandidateRecord::decode(
                json, message,
            )?)),
            _ => anyhow::bail!("unsupported SDK prepared record version or profile"),
        }
    }
}

impl From<SdkCandidateMetadata> for SdkPreparedRecord {
    fn from(value: SdkCandidateMetadata) -> Self {
        Self::DammSell(value)
    }
}

#[cfg(test)]
mod tests;
