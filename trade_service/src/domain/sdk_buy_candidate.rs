use super::{sdk_candidate::DammCandidatePool, trade::MevProvider};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BuyFunding {
    NativeSol,
    ExistingQuote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuyNativeBudget {
    pub before_lamports: u64,
    pub deposit_lamports: u64,
    pub rent_lamports: u64,
    pub base_fee_lamports: u64,
    pub remaining_lamports: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DammBuyDetails {
    pub wallet: [u8; 32],
    pub mint: [u8; 32],
    pub quote_mint: [u8; 32],
    pub pool_hint: Option<[u8; 32]>,
    pub pool: DammCandidatePool,
    pub funding: BuyFunding,
    pub input_account: [u8; 32],
    pub input_program: [u8; 32],
    pub input_decimals: u8,
    pub output_account: [u8; 32],
    pub output_program: [u8; 32],
    pub output_decimals: u8,
    pub input_units: u64,
    pub quoted_net_output_units: u64,
    pub minimum_output_units: u64,
    pub account_slot: u64,
    pub simulation_slot: u64,
    pub native: BuyNativeBudget,
    pub priority_fee_lamports: u64,
    pub tip_lamports: u64,
    pub create_output_account: bool,
    pub output_initialized: bool,
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

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
enum BuyRecordProfile {
    #[serde(rename = "damm-v2-buy-v1")]
    DammV2BuyV1,
}

/// Nominal record only. Deserialization cannot restore freshness, an owner or signing permission.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DammBuyCandidateRecord {
    version: u16,
    profile: BuyRecordProfile,
    details: DammBuyDetails,
    message_sha256: [u8; 32],
}

impl DammBuyCandidateRecord {
    pub(crate) fn new(details: DammBuyDetails, message: &[u8]) -> Result<Self> {
        ensure!(
            !message.is_empty() && message.len() <= 1232,
            "SDK buy candidate message size or digest mismatch"
        );
        let record = Self {
            version: 1,
            profile: BuyRecordProfile::DammV2BuyV1,
            details,
            message_sha256: Sha256::digest(message).into(),
        };
        record.validate(message)?;
        Ok(record)
    }

    pub(crate) fn details(&self) -> &DammBuyDetails {
        &self.details
    }

    pub(crate) fn validate(&self, message: &[u8]) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported SDK buy candidate record version"
        );
        ensure!(
            !message.is_empty()
                && message.len() <= 1232
                && self.message_sha256 == <[u8; 32]>::from(Sha256::digest(message)),
            "SDK buy candidate message size or digest mismatch"
        );
        let d = &self.details;
        let tip = f64::from_bits(d.tip_sol_bits);
        ensure!(
            d.wallet != [0; 32]
                && d.mint != d.quote_mint
                && d.input_units > 0
                && d.quoted_net_output_units > 0
                && d.minimum_output_units > 0
                && d.slippage_bps < 10_000
                && d.account_slot > 0
                && d.simulation_slot >= d.account_slot
                && d.recent_blockhash != [0; 32]
                && tip.is_finite()
                && tip >= 0.0,
            "SDK buy candidate amounts, context or fees invalid"
        );
        let minimum =
            u128::from(d.quoted_net_output_units) * u128::from(10_000 - d.slippage_bps) / 10_000;
        ensure!(
            minimum == u128::from(d.minimum_output_units),
            "SDK buy candidate net minimum mismatch"
        );
        let deposit = match d.funding {
            BuyFunding::NativeSol => d.input_units,
            BuyFunding::ExistingQuote => 0,
        };
        let n = &d.native;
        let spent = deposit
            .checked_add(n.rent_lamports)
            .and_then(|v| v.checked_add(n.base_fee_lamports));
        let extra = d.priority_fee_lamports.checked_add(d.tip_lamports);
        ensure!(
            n.deposit_lamports == deposit
                && spent.and_then(|v| n.before_lamports.checked_sub(v))
                    == Some(n.remaining_lamports)
                && extra.is_some_and(|v| v <= n.remaining_lamports),
            "SDK buy candidate native budget mismatch or overflow"
        );
        ensure!(
            d.pool_hint.is_none_or(|v| v == d.pool.pool)
                && (d.create_output_account || !d.output_initialized),
            "SDK buy candidate pool or ATA setup mismatch"
        );
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn encode(&self, message: &[u8]) -> Result<String> {
        self.validate(message)?;
        let json = serde_json::to_string(self)?;
        ensure!(
            json.len() <= 65_536,
            "SDK buy candidate record exceeds limit"
        );
        Ok(json)
    }

    #[cfg(test)]
    pub(crate) fn decode(json: &str, message: &[u8]) -> Result<Self> {
        ensure!(
            json.len() <= 65_536,
            "SDK buy candidate record exceeds limit"
        );
        let value: Self = serde_json::from_str(json)?;
        value.validate(message)?;
        Ok(value)
    }
}
