//! Native SOL V1 funding evidence only. It does not authorize a message or a signer.
use super::{read_bank, PumpCurveAccounts, PumpCurveEvidence, ACCOUNT_COUNT};
use crate::{
    adapters::{
        rpc::{
            parameter_reads::request::pump_curve::{PumpNativeFundingKeys, NATIVE_ACCOUNTS},
            read_http::ReadHttpTransport,
            read_runtime::SharedReadRuntime,
        },
        sdk::pumpapi_assets::{check_mint, check_vault, decode_data},
    },
    domain::trade::Side,
};
use anyhow::{ensure, Context, Result};
use bincode::Options;
use sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
use solana_account_decoder::UiAccount;
use solana_sdk::{clock::Clock, pubkey::Pubkey, sysvar};

use std::time::Instant;

struct NativeFundingPlan {
    curve: PumpCurveAccounts,
    keys: PumpNativeFundingKeys,
    base_program: Pubkey,
    side: Side,
}

#[derive(Debug)]
struct NativeFundingSnapshot {
    curve: PumpCurveEvidence,
    side: Side,
    epoch: u64,
    native_lamports: u64,
    curve_base_units: u64,
    user_base_units: Option<u64>,
}

impl NativeFundingPlan {
    fn new(
        curve: PumpCurveAccounts,
        wallet: Pubkey,
        base_program: Pubkey,
        side: Side,
    ) -> Result<Self> {
        ensure!(
            curve.quote == WSOL_TOKEN_ACCOUNT,
            "native Pump funding requires a SOL curve"
        );
        let keys = PumpNativeFundingKeys::new(&curve.keys, wallet, base_program)?;
        Ok(Self {
            curve,
            keys,
            base_program,
            side,
        })
    }

    fn keys(&self) -> &[Pubkey; NATIVE_ACCOUNTS] {
        self.keys.addresses()
    }

    async fn read<T: ReadHttpTransport>(
        &self,
        runtime: &SharedReadRuntime<T>,
        url: &str,
        minimum_slot: u64,
        deadline: Instant,
    ) -> Result<NativeFundingSnapshot> {
        read_bank(
            runtime,
            url,
            deadline,
            self.keys.query(minimum_slot)?,
            |slot, values| self.decode(self.keys(), slot, minimum_slot, values),
        )
        .await
    }

    fn decode(
        &self,
        request: &[Pubkey],
        slot: u64,
        minimum: u64,
        values: &[Option<UiAccount>],
    ) -> Result<NativeFundingSnapshot> {
        ensure!(
            request == self.keys().as_slice() && minimum > 0,
            "native Pump funding snapshot request is inconsistent"
        );
        let [_, base_mint, _, _, curve_base, user_base, wallet, clock] = values else {
            anyhow::bail!("native Pump funding account count is inconsistent");
        };
        let curve = self.curve.decode(
            &request[..ACCOUNT_COUNT],
            slot,
            minimum,
            &values[..ACCOUNT_COUNT],
        )?;
        ensure!(
            !curve.curve.complete,
            "native Pump funding curve is complete"
        );
        ensure!(
            curve.base_mint_layout.program == self.base_program,
            "native Pump funding mint program differs from ATA plan"
        );

        let clock_bytes = decode_data(
            clock.as_ref().context("native Pump Clock is missing")?,
            sysvar::id(),
            40,
            "native Pump Clock",
        )?;
        let clock: Clock = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .reject_trailing_bytes()
            .deserialize(&clock_bytes)
            .context("native Pump Clock layout is invalid")?;
        ensure!(
            clock.slot == slot,
            "native Pump Clock slot differs from the bank"
        );

        let mint = check_mint(
            base_mint.as_ref().context("native Pump mint is missing")?,
            self.base_program,
            None,
            clock.epoch,
            "native Pump base mint",
        )?;
        let curve_base_units = check_vault(
            curve_base
                .as_ref()
                .context("native Pump curve base ATA is missing")?,
            self.keys()[1],
            self.base_program,
            &mint.required,
            self.keys()[0],
            "native Pump curve base ATA",
        )?;
        ensure!(
            curve_base_units >= curve.curve.real_token_reserves,
            "native Pump curve base ATA is below reported real reserves"
        );
        let user_base_units = user_base
            .as_ref()
            .map(|account| {
                check_vault(
                    account,
                    self.keys()[1],
                    self.base_program,
                    &mint.required,
                    self.keys()[6],
                    "native Pump user base ATA",
                )
            })
            .transpose()?;
        ensure!(
            self.side != Side::Sell || user_base_units.is_some(),
            "native Pump sell input ATA is missing"
        );

        let wallet = wallet.as_ref().context("native Pump wallet is missing")?;
        decode_data(
            wallet,
            solana_system_interface::program::ID,
            0,
            "native Pump wallet",
        )?;
        Ok(NativeFundingSnapshot {
            curve,
            side: self.side,
            epoch: clock.epoch,
            native_lamports: wallet.lamports,
            curve_base_units,
            user_base_units,
        })
    }
}

mod budget;
#[cfg(test)]
mod tests;
