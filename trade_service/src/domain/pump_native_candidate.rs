//! Nominal native Pump history only; decoding never restores freshness or execution authority.
use super::trade::{MevProvider, Side};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PumpNativeDetails {
    pub request_id: String,
    pub wallet_id: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub wallet: [u8; 32],
    pub mint: [u8; 32],
    pub curve: [u8; 32],
    pub curve_vault: [u8; 32],
    pub user_ata: [u8; 32],
    pub base_program: [u8; 32],
    pub base_decimals: u8,
    pub creator: [u8; 32],
    pub protocol_fee: [u8; 32],
    pub creator_vault: [u8; 32],
    pub buyback: [u8; 32],
    pub cashback: bool,
    pub side: Side,
    pub input_units: u64,
    pub source_token_units: Option<u64>,
    pub sell_percent: Option<u64>,
    pub quoted_output_units: u64,
    pub minimum_output_units: u64,
    pub final_output_units: u64,
    pub source_slot: u64,
    pub quote_slot: u64,
    pub final_slot: u64,
    pub final_epoch: u64,
    pub slippage_bps: u64,
    pub tip_sol_bits: u64,
    pub relay: String,
    pub tip_destination: Option<[u8; 32]>,
    pub tip_lamports: u64,
    pub network_fee_lamports: u64,
    pub priority_fee_lamports: u64,
    pub native_before_lamports: u64,
    pub maximum_native_debit: u64,
    pub remaining_native_lamports: u64,
    pub recent_blockhash: [u8; 32],
    pub use_mev: Option<bool>,
    pub requested_mev_provider: Option<MevProvider>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
enum NativeProfile {
    #[serde(rename = "pump-native-intent-v1")]
    NativeIntentV1,
    #[serde(rename = "pump-native-prepared-v1")]
    NativePreparedV1,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PumpNativeCandidateRecord {
    version: u16,
    profile: NativeProfile,
    details: PumpNativeDetails,
    message_sha256: [u8; 32],
}

impl PumpNativeCandidateRecord {
    pub(crate) fn new(details: PumpNativeDetails, message: &[u8]) -> Result<Self> {
        let record = Self {
            version: 1,
            profile: NativeProfile::NativeIntentV1,
            details,
            message_sha256: Sha256::digest(message).into(),
        };
        record.validate(message)?;
        Ok(record)
    }

    pub(crate) fn details(&self) -> &PumpNativeDetails {
        &self.details
    }

    pub(crate) fn into_prepared(mut self, message: &[u8]) -> Result<Self> {
        self.validate(message)?;
        self.profile = NativeProfile::NativePreparedV1;
        Ok(self)
    }

    pub(crate) fn is_prepared(&self) -> bool {
        self.profile == NativeProfile::NativePreparedV1
    }

    pub(crate) fn validate(&self, message: &[u8]) -> Result<()> {
        ensure!(
            self.version == 1
                && !message.is_empty()
                && message.len() <= 1232
                && self.message_sha256 == <[u8; 32]>::from(Sha256::digest(message)),
            "native Pump record message size, version or digest differs"
        );
        let d = &self.details;
        ensure!(
            !d.request_id.is_empty()
                && !d.wallet_id.is_empty()
                && !d.idempotency_key.trim().is_empty()
                && !d.request_hash.is_empty()
                && [
                    d.wallet,
                    d.mint,
                    d.curve,
                    d.curve_vault,
                    d.user_ata,
                    d.base_program,
                    d.creator,
                    d.protocol_fee,
                    d.creator_vault,
                    d.buyback
                ]
                .iter()
                .all(|key| *key != [0; 32])
                && d.wallet != d.mint
                && d.source_slot > 0
                && d.quote_slot >= d.source_slot
                && d.final_slot >= d.quote_slot
                && d.recent_blockhash != [0; 32]
                && d.input_units > 0,
            "native Pump record identity or context is invalid"
        );
        match d.side {
            Side::Buy => ensure!(
                d.sell_percent.is_none() && d.source_token_units.is_some(),
                "native Pump record buy requires the existing ATA profile"
            ),
            Side::Sell => {
                let percent = d.sell_percent.filter(|v| (1..=99).contains(v));
                ensure!(
                    percent
                        .and_then(|percent| d.source_token_units?.checked_mul(percent))
                        .map(|units| units / 100)
                        == Some(d.input_units),
                    "native Pump record percentage input differs or requires account closure"
                );
            }
        }
        ensure!(
            d.slippage_bps < 10_000,
            "native Pump record slippage is invalid"
        );
        let minimum =
            u128::from(d.quoted_output_units) * u128::from(10_000 - d.slippage_bps) / 10_000;
        ensure!(
            d.quoted_output_units > 0
                && d.minimum_output_units > 0
                && minimum == u128::from(d.minimum_output_units)
                && d.final_output_units >= d.minimum_output_units,
            "native Pump record minimum or final output differs"
        );
        let tip = f64::from_bits(d.tip_sol_bits);
        ensure!(
            tip.is_finite()
                && tip >= 0.0
                && tip * 1_000_000_000.0 < u64::MAX as f64
                && !d.relay.is_empty()
                && d.relay.len() <= 32
                && d.network_fee_lamports > d.priority_fee_lamports,
            "native Pump record fee metadata is invalid"
        );
        let wants_tip = d.relay != "Default" && tip > 0.0;
        let tip_units = if wants_tip {
            (tip * 1_000_000_000.0).round() as u64
        } else {
            0
        };
        ensure!(
            d.tip_lamports == tip_units
                && if wants_tip {
                    d.tip_destination.is_some_and(|key| key != [0; 32])
                } else {
                    d.tip_destination.is_none()
                },
            "native Pump record tip differs from its envelope"
        );
        let input_debit = if d.side == Side::Buy {
            d.input_units
        } else {
            0
        };
        let debit = input_debit
            .checked_add(d.network_fee_lamports)
            .and_then(|value| value.checked_add(d.tip_lamports));
        ensure!(
            debit == Some(d.maximum_native_debit)
                && debit.and_then(|value| d.native_before_lamports.checked_sub(value))
                    == Some(d.remaining_native_lamports),
            "native Pump record prefunded debit differs or overflows"
        );
        Ok(())
    }

    pub(crate) fn encode(&self, message: &[u8]) -> Result<String> {
        self.validate(message)?;
        let json = serde_json::to_string(self)?;
        ensure!(json.len() <= 65_536, "native Pump record exceeds limit");
        Ok(json)
    }

    pub(crate) fn decode(json: &str, message: &[u8]) -> Result<Self> {
        ensure!(json.len() <= 65_536, "native Pump record exceeds limit");
        let record: Self = serde_json::from_str(json)?;
        record.validate(message)?;
        Ok(record)
    }
}
