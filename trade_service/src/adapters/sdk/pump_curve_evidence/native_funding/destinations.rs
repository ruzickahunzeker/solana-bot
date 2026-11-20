//! Same-bank destination evidence; no fee-rate quote or signing authorization.
use super::NativeFundingSnapshot;
use crate::adapters::sdk::pump_curve_evidence::SharingPrefix;
use anyhow::{ensure, Context, Result};
use bincode::Options;
use serde::Deserialize;
use sol_trade_sdk::instruction::utils::pumpfun::{
    get_creator_vault_pda, GLOBAL_ACCOUNT_DISCRIMINATOR,
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::collections::HashSet;

const GLOBAL_FEE_PREFIX_END: usize = 1005;

#[derive(Deserialize)]
struct GlobalFeePrefix {
    _initialized: bool,
    _authority: [u8; 32],
    fee_recipient: [u8; 32],
    _initial_virtual_token_reserves: u64,
    _initial_virtual_sol_reserves: u64,
    _initial_real_token_reserves: u64,
    _token_total_supply: u64,
    fee_basis_points: u64,
    _withdraw_authority: [u8; 32],
    _enable_migrate: bool,
    _pool_migration_fee: u64,
    creator_fee_basis_points: u64,
    fee_recipients: [[u8; 32]; 7],
    _set_creator_authority: [u8; 32],
    _admin_set_creator_authority: [u8; 32],
    _create_v2_enabled: bool,
    _whitelist_pda: [u8; 32],
    _reserved_fee_recipient: [u8; 32],
    _mayhem_mode_enabled: bool,
    _reserved_fee_recipients: [[u8; 32]; 7],
    _is_cashback_enabled: bool,
    buyback_fee_recipients: [[u8; 32]; 8],
    buyback_basis_points: u64,
}

#[derive(Debug)]
pub(super) struct NativeDestinations {
    pub(super) protocol_fee: Pubkey,
    pub(super) creator_vault: Pubkey,
    pub(super) buyback: [Pubkey; 8],
}

impl NativeFundingSnapshot {
    pub(super) fn destinations(&self) -> Result<NativeDestinations> {
        // Do not interpret unknown/paused sharing status as proof of no sharing.
        ensure!(
            !self.curve.curve.mayhem && self.curve.sharing == SharingPrefix::Absent,
            "native Pump destination profile requires a standard curve and verified absent sharing"
        );
        let body = self
            .curve
            .global_data
            .get(8..GLOBAL_FEE_PREFIX_END)
            .context("native Pump Global fee prefix is truncated")?;
        ensure!(
            self.curve
                .global_data
                .starts_with(&GLOBAL_ACCOUNT_DISCRIMINATOR),
            "native Pump Global discriminator differs"
        );
        let prefix: GlobalFeePrefix = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .reject_trailing_bytes()
            .deserialize(body)
            .context("native Pump Global fee prefix is invalid")?;
        ensure!(
            prefix.fee_basis_points <= 10_000
                && prefix.creator_fee_basis_points <= 10_000
                && prefix.buyback_basis_points <= 10_000,
            "native Pump Global fee field is invalid"
        );
        ensure!(
            prefix.fee_recipient == self.curve.global_fee_recipient.to_bytes(),
            "native Pump Global primary recipient differs"
        );
        // Freeze one bank-backed primary recipient, not the SDK's static fallback.
        let protocol_fee = Pubkey::new_from_array(prefix.fee_recipient);
        let buyback = prefix.buyback_fee_recipients.map(Pubkey::new_from_array);
        let creator = self.curve.curve.creator;
        ensure!(
            creator != Pubkey::default(),
            "native Pump curve creator is missing"
        );
        let creator_vault =
            get_creator_vault_pda(&creator).context("native Pump creator vault PDA")?;
        ensure!(
            buyback.iter().all(|key| *key != Pubkey::default()),
            "native Pump buyback recipient is unset"
        );
        // The alternate primary list is parsed but does not broaden this frozen selection.
        let _alternate_primary = prefix.fee_recipients;
        Ok(NativeDestinations {
            protocol_fee,
            creator_vault,
            buyback,
        })
    }

    pub(super) fn inspect_native_destinations(&self, instruction: &Instruction) -> Result<()> {
        self.inspect_native_roles(instruction)?;
        ensure!(
            instruction
                .accounts
                .iter()
                .map(|meta| meta.pubkey)
                .collect::<HashSet<_>>()
                .len()
                == instruction.accounts.len(),
            "native Pump destination profile does not allow account aliases"
        );
        let destinations = self.destinations()?;
        let buy = self.side == super::Side::Buy;
        for (index, expected) in [
            (1, AccountMeta::new(destinations.protocol_fee, false)),
            (
                if buy { 9 } else { 8 },
                AccountMeta::new(destinations.creator_vault, false),
            ),
        ] {
            ensure!(
                instruction.accounts[index] == expected,
                "native Pump fee or creator destination differs from the bank"
            );
        }
        let extra = instruction
            .accounts
            .last()
            .context("native Pump buyback recipient is missing")?;
        ensure!(
            destinations.buyback.contains(&extra.pubkey) && extra.is_writable && !extra.is_signer,
            "native Pump buyback recipient differs from the bank"
        );
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests;
