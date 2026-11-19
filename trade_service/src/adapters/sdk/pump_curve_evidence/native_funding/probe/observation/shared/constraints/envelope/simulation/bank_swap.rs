//! One readonly native bank-to-final-message path; no signer, journal or live permission.
use super::NativeFinalObservation;
use crate::adapters::{
    rpc::{read_http::ReadHttpTransport, read_runtime::SharedReadRuntime},
    sdk::pump_curve_evidence::native_funding::NativeFundingSnapshot,
};
use anyhow::Result;
use sol_trade_sdk::swqos::SwqosType;
use solana_sdk::{hash::Hash, pubkey::Pubkey};
use std::num::NonZeroU64;

impl NativeFundingSnapshot {
    pub(in crate::adapters::sdk::pump_curve_evidence::native_funding) async fn observe_complete<
        T: ReadHttpTransport,
    >(
        self,
        runtime: &SharedReadRuntime<T>,
        url: &str,
        input: NonZeroU64,
        buyback: Pubkey,
        blockhash: Hash,
        slippage_bps: u64,
        provider: SwqosType,
        tip_sol: f64,
        tip_destination: Option<Pubkey>,
    ) -> Result<NativeFinalObservation> {
        // Floor one is discovery-only; the original quote constraints derive the final minimum.
        self.prepare_native_swap(input, NonZeroU64::MIN, buyback)?
            .into_quote_probe(blockhash)?
            .shared_observation(runtime, url)
            .await?
            .swap_constraints(slippage_bps)?
            .prepare_envelope(provider, tip_sol, tip_destination)?
            .simulate(runtime, url)
            .await
    }
}

mod tests;
