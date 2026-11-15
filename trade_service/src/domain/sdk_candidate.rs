use crate::domain::trade::MevProvider;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_JSON: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SdkCandidateProfile {
    DammV2SellV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DammCandidatePool {
    pub pool: [u8; 32],
    pub token_a_vault: [u8; 32],
    pub token_b_vault: [u8; 32],
    pub token_a_mint: [u8; 32],
    pub token_b_mint: [u8; 32],
    pub token_a_program: [u8; 32],
    pub token_b_program: [u8; 32],
    pub referral: Option<[u8; 32]>,
    pub rate_limiter_sysvar: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DammCandidateDetails {
    pub wallet: [u8; 32],
    pub mint: [u8; 32],
    pub quote_mint: [u8; 32],
    pub pool_hint: Option<[u8; 32]>,
    pub pool: DammCandidatePool,
    pub balance_units: u64,
    pub balance_slot: u64,
    pub input_account: [u8; 32],
    pub input_program: [u8; 32],
    pub input_decimals: u8,
    pub input_units: u64,
    pub sell_percent: u64,
    pub minimum_output_units: u64,
    pub slippage_bps: u64,
    pub tip_sol_bits: u64,
    pub relay: String,
    pub tip_destination: Option<[u8; 32]>,
    pub recent_blockhash: [u8; 32],
    pub simulate: bool,
    pub use_mev: Option<bool>,
    pub requested_mev_provider: Option<MevProvider>,
    pub wait_tx_confirmed: bool,
    pub mev_submit_verify_ms: u64,
}

/// A nominal candidate record, not signing authority, freshness or pool verification.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SdkCandidateMetadata {
    version: u16,
    profile: SdkCandidateProfile,
    details: DammCandidateDetails,
    message_sha256: [u8; 32],
}

impl SdkCandidateMetadata {
    pub(crate) fn damm_sell(details: DammCandidateDetails, message: &[u8]) -> Result<Self> {
        let value = Self {
            version: 1,
            profile: SdkCandidateProfile::DammV2SellV1,
            details,
            message_sha256: Sha256::digest(message).into(),
        };
        value.validate(message)?;
        Ok(value)
    }

    pub(crate) fn details(&self) -> &DammCandidateDetails {
        &self.details
    }

    pub(crate) fn validate(&self, message: &[u8]) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported SDK candidate metadata version"
        );
        ensure!(
            !message.is_empty()
                && message.len() <= 1232
                && self.message_sha256 == <[u8; 32]>::from(Sha256::digest(message)),
            "SDK candidate message size or digest mismatch"
        );
        let d = &self.details;
        let tip = f64::from_bits(d.tip_sol_bits);
        ensure!(
            (1..=100).contains(&d.sell_percent)
                && d.balance_slot > 0
                && d.input_units > 0
                && d.balance_units.checked_mul(d.sell_percent).map(|v| v / 100)
                    == Some(d.input_units)
                && d.minimum_output_units > 0
                && d.recent_blockhash != [0; 32]
                && tip.is_finite()
                && tip >= 0.0,
            "SDK candidate integer amount, balance or fee metadata invalid"
        );
        ensure!(
            d.pool_hint.is_none_or(|hint| hint == d.pool.pool),
            "SDK candidate pool hint mismatch"
        );
        Ok(())
    }

    pub(crate) fn encode(&self, message: &[u8]) -> Result<String> {
        self.validate(message)?;
        let json = serde_json::to_string(self)?;
        ensure!(
            json.len() <= MAX_JSON,
            "SDK candidate metadata exceeds limit"
        );
        Ok(json)
    }

    pub(crate) fn decode(json: &str, message: &[u8]) -> Result<Self> {
        ensure!(
            json.len() <= MAX_JSON,
            "SDK candidate metadata exceeds limit"
        );
        let value: Self = serde_json::from_str(json)?;
        value.validate(message)?;
        Ok(value)
    }
}
