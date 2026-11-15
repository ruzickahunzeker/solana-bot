use super::{evidence::AccountSnapshot, plan::PreparedDammProbe};
use crate::{
    adapters::sdk::damm_instructions::pool_snapshot,
    domain::{
        preparation_balance::{BalanceFailureKind, PreparationBalanceFailure, MAX_BALANCE_AGE},
        sdk_candidate::{DammCandidateDetails, DammCandidatePool},
    },
    protocol::meteora_damm_v2::minimum_output,
};
use anyhow::{ensure, Result};
use std::time::Instant;

/// In-memory evidence from the validated unsigned probe, not signing authority.
/// It contains no RPC, signer, serialization or freshness-renewal capability.
pub(crate) struct DammOutputEvidence {
    wallet: [u8; 32],
    pool: DammCandidatePool,
    input_mint: [u8; 32],
    output_mint: [u8; 32],
    input_ata: [u8; 32],
    input_program: [u8; 32],
    input_decimals: u8,
    input_units: u64,
    output_units: u64,
    blockhash: [u8; 32],
    account_slot: u64,
    simulation_slot: u64,
    started: Instant,
}

impl DammOutputEvidence {
    pub(super) fn checked(
        prepared: &PreparedDammProbe,
        snapshot: &AccountSnapshot,
        output_units: u64,
        account_slot: u64,
        simulation_slot: u64,
        started: Instant,
    ) -> Result<Self> {
        let intent = prepared.accounts();
        ensure!(
            account_slot > 0 && simulation_slot >= account_slot && output_units > 0,
            "DAMM output evidence slot or amount is invalid"
        );
        let value = Self {
            wallet: intent.wallet.to_bytes(),
            pool: pool_snapshot(&intent.pool),
            input_mint: intent.input_mint.to_bytes(),
            output_mint: intent.output_mint.to_bytes(),
            input_ata: intent.input_ata.to_bytes(),
            input_program: snapshot.input.program.to_bytes(),
            input_decimals: snapshot.input.decimals,
            input_units: intent.amount,
            output_units,
            blockhash: prepared.transaction().message.recent_blockhash.to_bytes(),
            account_slot,
            simulation_slot,
            started,
        };
        value.recheck()?;
        Ok(value)
    }

    pub(crate) fn output_units(&self) -> u64 {
        self.output_units
    }

    pub(crate) fn recheck(&self) -> Result<()> {
        if Instant::now()
            .checked_duration_since(self.started)
            .is_none_or(|age| age > MAX_BALANCE_AGE)
        {
            return Err(PreparationBalanceFailure::new(
                BalanceFailureKind::Expired,
                "DAMM output quote evidence expired",
            )
            .into());
        }
        Ok(())
    }

    pub(crate) fn validate_details(&self, candidate: &DammCandidateDetails) -> Result<()> {
        self.recheck()?;
        let result = (|| {
            ensure!(
                candidate.wallet == self.wallet
                    && candidate.pool == self.pool
                    && candidate
                        .pool_hint
                        .is_none_or(|hint| hint == self.pool.pool)
                    && candidate.mint == self.input_mint
                    && candidate.quote_mint == self.output_mint
                    && candidate.input_account == self.input_ata
                    && candidate.input_program == self.input_program
                    && candidate.input_decimals == self.input_decimals
                    && candidate.input_units == self.input_units
                    && candidate.recent_blockhash == self.blockhash,
                "DAMM candidate differs from its actual output quote context"
            );
            ensure!(
                (1..=100).contains(&candidate.sell_percent)
                    && candidate
                        .balance_units
                        .checked_mul(candidate.sell_percent)
                        .map(|units| units / 100)
                        == Some(self.input_units),
                "DAMM output quote input differs from captured percentage balance"
            );
            ensure!(
                candidate.balance_slot > 0
                    && candidate.balance_slot <= self.account_slot
                    && self.account_slot <= self.simulation_slot,
                "DAMM output quote precedes the captured balance evidence"
            );
            ensure!(
                candidate.minimum_output_units
                    == minimum_output(self.output_units, candidate.slippage_bps)?,
                "DAMM candidate minimum output differs from evidenced quote"
            );
            Ok(())
        })();
        result.map_err(|error: anyhow::Error| {
            crate::domain::preparation_balance::reject_balance("DAMM output quote binding rejected")
                .context(error.to_string())
        })
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
impl DammOutputEvidence {
    pub(crate) fn fixture_for_profile(
        wallet: solana_sdk::pubkey::Pubkey,
        intent: &crate::adapters::sdk::trade_command::ExecuteTrade,
        request: &sol_trade_sdk::TradeSellParams,
        balance: u64,
        percent: u64,
        holding: &crate::domain::preparation_balance::PreparationBalanceSnapshot,
    ) -> std::sync::Arc<Self> {
        use crate::domain::trade::Address;
        let sol_trade_sdk::trading::core::params::DexParamEnum::MeteoraDammV2(pool) =
            &request.extension_params
        else {
            panic!("fixture needs DAMM parameters")
        };
        let input = holding
            .checked_input(
                Address::from_bytes(wallet.to_bytes()),
                Address::from_bytes(intent.mint.to_bytes()),
            )
            .unwrap();
        let slot = holding
            .checked_slot(
                Address::from_bytes(wallet.to_bytes()),
                Address::from_bytes(intent.mint.to_bytes()),
            )
            .unwrap();
        let minimum = request.fixed_output_token_amount.unwrap();
        let quoted =
            (u128::from(minimum) * 10_000).div_ceil(u128::from(10_000 - intent.slippage_bps));
        std::sync::Arc::new(Self {
            wallet: wallet.to_bytes(),
            pool: pool_snapshot(pool),
            input_mint: intent.mint.to_bytes(),
            output_mint: intent.quote_mint.to_bytes(),
            input_ata: input.account.to_bytes(),
            input_program: input.program.to_bytes(),
            input_decimals: input.decimals,
            input_units: balance.checked_mul(percent).unwrap() / 100,
            output_units: u64::try_from(quoted).unwrap(),
            blockhash: intent.recent_blockhash.to_bytes(),
            account_slot: slot,
            simulation_slot: slot,
            started: Instant::now(),
        })
    }
}
