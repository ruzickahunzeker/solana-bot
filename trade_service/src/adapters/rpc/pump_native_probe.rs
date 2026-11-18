//! A fixed unsigned native Pump probe. No execution RPC or externally encoded packet input.
use crate::domain::preparation_balance::MAX_BALANCE_AGE;
use anyhow::{ensure, Result};
use base64::Engine;
use serde_json::{json, Value};
use sol_trade_sdk::instruction::utils::pumpfun::{
    accounts, BUY_EXACT_SOL_IN_DISCRIMINATOR, SELL_DISCRIMINATOR,
};
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Signature, transaction::Transaction};
use std::{collections::HashSet, time::Instant};

pub(crate) const EVIDENCE_ACCOUNTS: usize = 11;

pub(crate) struct PumpNativeProbePacket {
    bytes: Vec<u8>,
    accounts: [Pubkey; EVIDENCE_ACCOUNTS],
    minimum: u64,
    deadline: Instant,
}

pub(super) struct PumpNativeProbeBinding {
    bytes: Vec<u8>,
    accounts: [Pubkey; EVIDENCE_ACCOUNTS],
    minimum: u64,
    deadline: Instant,
}

pub(crate) struct PumpNativeProbeReply {
    binding: PumpNativeProbeBinding,
    response: Value,
}

impl PumpNativeProbePacket {
    pub(crate) fn new(
        transaction: &Transaction,
        accounts: &[Pubkey; EVIDENCE_ACCOUNTS],
        minimum: u64,
        deadline: Instant,
    ) -> Result<Self> {
        let now = Instant::now();
        ensure!(
            minimum > 0 && deadline > now && deadline <= now + MAX_BALANCE_AGE,
            "native Pump probe requires a bounded nonzero evidence context"
        );
        ensure!(
            bincode::serialized_size(transaction)? <= 1232,
            "native Pump probe exceeds packet size"
        );
        solana_sdk::transaction::VersionedTransaction::from(transaction.clone())
            .sanitize()
            .map_err(|_| anyhow::anyhow!("native Pump probe message is malformed"))?;
        let message = &transaction.message;
        ensure!(
            transaction.signatures == [Signature::default()]
                && message.header.num_required_signatures == 1
                && message
                    .account_keys
                    .first()
                    .is_some_and(|payer| *payer != Pubkey::default())
                && message.recent_blockhash != Hash::default()
                && message.instructions.len() == 1,
            "native Pump probe requires one zero-signature legacy swap"
        );
        let swap = &message.instructions[0];
        let buy = swap.data.starts_with(&BUY_EXACT_SOL_IN_DISCRIMINATOR);
        let sell = swap.data.starts_with(&SELL_DISCRIMINATOR);
        ensure!(
            message.account_keys[usize::from(swap.program_id_index)] == accounts::PUMPFUN
                && ((buy && swap.data.len() == 25 && swap.data[24] <= 1)
                    || (sell && swap.data.len() == 24))
                && u64::from_le_bytes(swap.data[8..16].try_into()?) > 0
                && swap.data[16..24] == 1u64.to_le_bytes(),
            "native Pump probe requires its exact-input discovery wire"
        );
        ensure!(
            accounts.iter().all(|key| *key != Pubkey::default())
                && accounts.iter().collect::<HashSet<_>>().len() == EVIDENCE_ACCOUNTS,
            "native Pump probe evidence roles overlap or are unset"
        );
        let bytes = bincode::serialize(transaction)?;
        ensure!(bytes.len() <= 1232, "native Pump probe exceeds packet size");
        ensure!(
            Instant::now() < deadline,
            "native Pump probe expired during packet construction"
        );
        Ok(Self {
            bytes,
            accounts: *accounts,
            minimum,
            deadline,
        })
    }

    pub(super) fn binding(&self) -> PumpNativeProbeBinding {
        PumpNativeProbeBinding {
            bytes: self.bytes.clone(),
            accounts: self.accounts,
            minimum: self.minimum,
            deadline: self.deadline,
        }
    }

    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(super) fn minimum(&self) -> u64 {
        self.minimum
    }

    pub(super) fn body(&self, id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"simulateTransaction","params":[
            base64::engine::general_purpose::STANDARD.encode(&self.bytes),
            {"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64",
                "accounts":{"encoding":"base64","addresses":self.accounts.map(|key|key.to_string())},
                "minContextSlot":self.minimum,"innerInstructions":true}]})
    }
}

impl PumpNativeProbeBinding {
    pub(super) fn complete(self, response: Value) -> Result<PumpNativeProbeReply> {
        ensure!(
            Instant::now() < self.deadline,
            "native Pump probe expired after response parsing"
        );
        Ok(PumpNativeProbeReply {
            binding: self,
            response,
        })
    }
}

impl PumpNativeProbeReply {
    pub(crate) fn for_probe(
        self,
        transaction: &Transaction,
        accounts: &[Pubkey; EVIDENCE_ACCOUNTS],
        minimum: u64,
    ) -> Result<Value> {
        ensure!(
            Instant::now() < self.binding.deadline,
            "native Pump probe reply expired"
        );
        ensure!(
            bincode::serialized_size(transaction)? == self.binding.bytes.len() as u64
                && bincode::serialize(transaction)? == self.binding.bytes
                && *accounts == self.binding.accounts
                && minimum == self.binding.minimum,
            "native Pump probe response belongs to another packet or evidence context"
        );
        ensure!(
            Instant::now() < self.binding.deadline,
            "native Pump probe reply expired during packet verification"
        );
        Ok(self.response)
    }
}

#[cfg(test)]
pub(crate) mod tests;
