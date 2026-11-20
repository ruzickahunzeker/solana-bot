//! Native SOL V1 funding evidence only. It does not authorize a message or a signer.
use super::{
    instruction_amounts::{PumpWireAmounts, TradeAmounts, WireLayout},
    read_bank, PumpCurveAccounts, PumpCurveEvidence, ACCOUNT_COUNT,
};
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
use solana_sdk::{
    clock::Clock,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    sysvar,
};

use std::time::Instant;

struct NativeFundingPlan {
    curve: PumpCurveAccounts,
    keys: PumpNativeFundingKeys,
    base_program: Pubkey,
    side: Side,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NativeBankPhase {
    Preparation,
    AfterSimulation,
}

#[derive(Debug)]
struct NativeFundingSnapshot {
    started: Instant,
    keys: [Pubkey; NATIVE_ACCOUNTS],
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
        let started = Instant::now();
        read_bank(
            runtime,
            url,
            deadline,
            self.keys.query(minimum_slot)?,
            |slot, values| {
                let mut snapshot = self.decode(self.keys(), slot, minimum_slot, values)?;
                snapshot.started = started;
                Ok(snapshot)
            },
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
        self.decode_bank(request, slot, minimum, values, NativeBankPhase::Preparation)
    }

    fn decode_bank(
        &self,
        request: &[Pubkey],
        slot: u64,
        minimum: u64,
        values: &[Option<UiAccount>],
        phase: NativeBankPhase,
    ) -> Result<NativeFundingSnapshot> {
        let started = Instant::now();
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
            phase == NativeBankPhase::AfterSimulation || !curve.curve.complete,
            "native Pump funding curve is complete"
        );
        ensure!(
            !curve.curve.complete
                || (self.side == Side::Buy && curve.curve.real_token_reserves == 0),
            "native Pump completed post curve has inconsistent side or remaining real tokens"
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
            started,
            keys: *self.keys(),
            curve,
            side: self.side,
            epoch: clock.epoch,
            native_lamports: wallet.lamports,
            curve_base_units,
            user_base_units,
        })
    }
}

impl NativeFundingSnapshot {
    // Checks only critical funding roles; fees, creator accounts and the full message remain unapproved.
    fn inspect_native_roles(&self, instruction: &Instruction) -> Result<PumpWireAmounts> {
        use sol_trade_sdk::instruction::utils::pumpfun::{
            accounts, get_bonding_curve_v2_pda, get_user_volume_accumulator_pda,
        };
        ensure!(
            instruction.program_id == accounts::PUMPFUN,
            "native Pump instruction program differs"
        );
        let wire = PumpWireAmounts::decode(&instruction.data)?;
        ensure!(
            wire.layout == WireLayout::NativeV1,
            "native Pump evidence cannot validate a V2 instruction"
        );
        let buy = match (self.side, wire.amounts) {
            (Side::Buy, TradeAmounts::BuyExactInput { .. }) => true,
            (Side::Sell, TradeAmounts::SellExactInput { .. }) => false,
            _ => anyhow::bail!("native Pump instruction amount mode differs from funding intent"),
        };
        ensure!(
            wire.track_volume == buy.then_some(self.curve.curve.cashback),
            "native Pump volume flag differs from the bank curve"
        );
        let count = if buy {
            18
        } else {
            16 + usize::from(self.curve.curve.cashback)
        };
        ensure!(
            instruction.accounts.len() == count,
            "native Pump instruction account count differs"
        );
        for (index, expected) in [
            (0, AccountMeta::new_readonly(self.keys[2], false)),
            (2, AccountMeta::new_readonly(self.keys[1], false)),
            (3, AccountMeta::new(self.keys[0], false)),
            (4, AccountMeta::new(self.keys[4], false)),
            (5, AccountMeta::new(self.keys[5], false)),
            (6, AccountMeta::new(self.keys[6], true)),
            (
                if buy { 8 } else { 9 },
                AccountMeta::new_readonly(self.curve.base_mint_layout.program, false),
            ),
        ] {
            ensure!(
                instruction.accounts[index] == expected,
                "native Pump funding role or privilege differs"
            );
            ensure!(
                instruction
                    .accounts
                    .iter()
                    .filter(|meta| meta.pubkey == expected.pubkey)
                    .count()
                    == 1,
                "native Pump funding role is aliased"
            );
        }
        let curve_v2 =
            get_bonding_curve_v2_pda(&self.keys[1]).context("native Pump secondary curve PDA")?;
        let mut fixed_roles = vec![
            (
                7,
                AccountMeta::new_readonly(solana_system_interface::program::ID, false),
            ),
            (10, accounts::EVENT_AUTHORITY_META),
            (11, accounts::PUMPFUN_META),
            (if buy { 14 } else { 12 }, accounts::FEE_CONFIG_META),
            (if buy { 15 } else { 13 }, accounts::FEE_PROGRAM_META),
            (count - 2, AccountMeta::new_readonly(curve_v2, false)),
        ];
        if buy {
            fixed_roles.push((12, accounts::GLOBAL_VOLUME_ACCUMULATOR_META));
        }
        if buy || self.curve.curve.cashback {
            let user_volume = get_user_volume_accumulator_pda(&self.keys[6])
                .context("native Pump user volume PDA")?;
            fixed_roles.push((
                if buy { 13 } else { 14 },
                AccountMeta::new(user_volume, false),
            ));
        }
        for (index, expected) in fixed_roles {
            ensure!(
                instruction.accounts[index] == expected,
                "native Pump fixed program or derived role differs"
            );
        }
        ensure!(
            instruction
                .accounts
                .iter()
                .enumerate()
                .all(|(i, meta)| meta.is_signer == (i == 6)),
            "native Pump instruction contains an unexpected signer"
        );
        Ok(wire)
    }
}

mod budget;
mod candidate;
pub(crate) use candidate::NativeProfileUnsupported;
pub(crate) use candidate::{prepare_native_evidence, NativeEvidenceContext};
mod destinations;
#[cfg(test)]
mod instruction_tests;
mod native_swap;
mod probe;
pub(crate) use probe::NativeBoundIntent;
#[cfg(test)]
mod tests;
