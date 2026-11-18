//! Fixed final-envelope simulation read, never a sender or arbitrary encoded-packet API.
use super::pump_native_probe::EVIDENCE_ACCOUNTS;
use crate::domain::preparation_balance::MAX_BALANCE_AGE;
use anyhow::{ensure, Context, Result};
use base64::Engine;
use bincode::Options;
use serde_json::{json, Value};
use sol_trade_sdk::instruction::utils::pumpfun::{
    accounts, BUY_EXACT_SOL_IN_DISCRIMINATOR, SELL_DISCRIMINATOR,
};
use solana_sdk::{
    hash::Hash, message::VersionedMessage, pubkey::Pubkey, signature::Signature,
    transaction::VersionedTransaction,
};
use std::{collections::HashSet, time::Instant};

pub(crate) struct PumpNativeFinalPacket {
    binding: PumpNativeFinalBinding,
}

pub(super) struct PumpNativeFinalBinding {
    bytes: Vec<u8>,
    accounts: Vec<Pubkey>,
    minimum: u64,
    deadline: Instant,
}

pub(crate) struct PumpNativeFinalReply {
    binding: PumpNativeFinalBinding,
    response: Value,
}

impl PumpNativeFinalPacket {
    pub(crate) fn new(
        transaction: &VersionedTransaction,
        evidence: &[Pubkey],
        minimum: u64,
        deadline: Instant,
    ) -> Result<Self> {
        let now = Instant::now();
        ensure!(
            minimum > 0 && deadline > now && deadline <= now + MAX_BALANCE_AGE,
            "native Pump final requires a bounded nonzero evidence context"
        );
        ensure!(
            bincode::serialized_size(transaction)? <= 1232,
            "native Pump final exceeds packet size"
        );
        transaction
            .sanitize()
            .map_err(|_| anyhow::anyhow!("native Pump final message is malformed"))?;
        let VersionedMessage::Legacy(message) = &transaction.message else {
            anyhow::bail!("native Pump final requires its no-ALT legacy envelope")
        };
        ensure!(
            transaction.signatures == [Signature::default()]
                && message.header.num_required_signatures == 1
                && message.header.num_readonly_signed_accounts == 0
                && message
                    .account_keys
                    .first()
                    .is_some_and(|key| *key != Pubkey::default())
                && message.recent_blockhash != Hash::default()
                && matches!(message.instructions.len(), 3 | 4),
            "native Pump final requires its zero-signature fixed envelope"
        );
        let tip = usize::from(message.instructions.len() == 4);
        let compute: Pubkey = "ComputeBudget111111111111111111111111111111".parse()?;
        let mut price = vec![3];
        price.extend(1_000u64.to_le_bytes());
        let mut limit = vec![2];
        limit.extend(300_000u32.to_le_bytes());
        for (offset, data) in [price, limit].into_iter().enumerate() {
            let instruction = &message.instructions[tip + offset];
            ensure!(
                message.account_keys[usize::from(instruction.program_id_index)] == compute
                    && instruction.accounts.is_empty()
                    && instruction.data == data,
                "native Pump final compute budget differs from its fixed profile"
            );
        }
        let swap = message
            .instructions
            .last()
            .context("native Pump final omitted swap")?;
        let buy = swap.data.starts_with(&BUY_EXACT_SOL_IN_DISCRIMINATOR);
        let sell = swap.data.starts_with(&SELL_DISCRIMINATOR);
        ensure!(
            message.account_keys[usize::from(swap.program_id_index)] == accounts::PUMPFUN
                && ((buy && swap.data.len() == 25 && swap.data[24] <= 1)
                    || (sell && swap.data.len() == 24))
                && u64::from_le_bytes(swap.data[8..16].try_into()?) > 0
                && u64::from_le_bytes(swap.data[16..24].try_into()?) > 0,
            "native Pump final requires a positive exact-input native swap"
        );
        ensure!(
            evidence.len() == EVIDENCE_ACCOUNTS + tip
                && evidence.iter().all(|key| *key != Pubkey::default())
                && evidence.iter().collect::<HashSet<_>>().len() == evidence.len(),
            "native Pump final evidence roles overlap, are unset or have wrong count"
        );
        if tip == 1 {
            let transfer = &message.instructions[0];
            ensure!(
                message.account_keys[usize::from(transfer.program_id_index)]
                    == solana_system_interface::program::ID
                    && transfer.accounts.len() == 2
                    && transfer.accounts[0] == 0
                    && transfer.data.len() == 12,
                "native Pump final tip is not the fixed payer transfer"
            );
            let kind: solana_system_interface::instruction::SystemInstruction =
                bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .with_limit(12)
                    .reject_trailing_bytes()
                    .deserialize(&transfer.data)
                    .context("native Pump final tip instruction is malformed")?;
            ensure!(
                matches!(
                    kind,
                    solana_system_interface::instruction::SystemInstruction::Transfer { .. }
                ),
                "native Pump final tip is not a System transfer"
            );
            let destination = message.account_keys[usize::from(transfer.accounts[1])];
            ensure!(
                evidence.last() == Some(&destination)
                    && destination != message.account_keys[0]
                    && !swap
                        .accounts
                        .iter()
                        .any(|index| message.account_keys[usize::from(*index)] == destination)
                    && destination != accounts::PUMPFUN
                    && destination != compute,
                "native Pump final tip is not a distinct bound evidence role"
            );
        }
        let bytes = bincode::serialize(transaction)?;
        ensure!(bytes.len() <= 1232, "native Pump final exceeds packet size");
        ensure!(
            Instant::now() < deadline,
            "native Pump final expired during packet construction"
        );
        Ok(Self {
            binding: PumpNativeFinalBinding {
                bytes,
                accounts: evidence.to_vec(),
                minimum,
                deadline,
            },
        })
    }

    pub(super) fn deadline(&self) -> Instant {
        self.binding.deadline
    }

    pub(super) fn minimum(&self) -> u64 {
        self.binding.minimum
    }

    pub(super) fn account_count(&self) -> usize {
        self.binding.accounts.len()
    }

    pub(super) fn body(&self, id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"simulateTransaction","params":[
            base64::engine::general_purpose::STANDARD.encode(&self.binding.bytes),
            {"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64",
                "accounts":{"encoding":"base64","addresses":self.binding.accounts.iter().map(ToString::to_string).collect::<Vec<_>>()},
                "minContextSlot":self.binding.minimum,"innerInstructions":true}]})
    }

    pub(super) fn binding(&self) -> PumpNativeFinalBinding {
        PumpNativeFinalBinding {
            bytes: self.binding.bytes.clone(),
            accounts: self.binding.accounts.clone(),
            minimum: self.binding.minimum,
            deadline: self.binding.deadline,
        }
    }
}

impl PumpNativeFinalBinding {
    pub(super) fn complete(self, response: Value) -> Result<PumpNativeFinalReply> {
        ensure!(
            Instant::now() < self.deadline,
            "native Pump final reply expired after decoding"
        );
        Ok(PumpNativeFinalReply {
            binding: self,
            response,
        })
    }
}

impl PumpNativeFinalReply {
    pub(crate) fn for_template(
        self,
        transaction: &VersionedTransaction,
        accounts: &[Pubkey],
        minimum: u64,
    ) -> Result<Value> {
        ensure!(
            Instant::now() < self.binding.deadline,
            "native Pump final reply expired"
        );
        ensure!(
            bincode::serialized_size(transaction)? == self.binding.bytes.len() as u64
                && bincode::serialize(transaction)? == self.binding.bytes
                && accounts == self.binding.accounts.as_slice()
                && minimum == self.binding.minimum,
            "native Pump final response belongs to another template or evidence context"
        );
        ensure!(
            Instant::now() < self.binding.deadline,
            "native Pump final reply expired during binding verification"
        );
        Ok(self.response)
    }
}

#[cfg(test)]
pub(crate) mod tests;
