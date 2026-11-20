//! Bound complete-envelope simulation evidence, not a quote or signing permit.
use super::{tip_lamports, NativeEnvelopeTemplate, PRIORITY_LAMPORTS};
use crate::adapters::{
    rpc::{
        pump_native_final::PumpNativeFinalPacket, read_http::ReadHttpTransport,
        read_runtime::SharedReadRuntime,
    },
    sdk::{
        pump_curve_evidence::native_funding::probe::observation::{
            effects, post_bank::NativePostBank,
        },
        pumpapi_assets::decode_data,
    },
};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};
use solana_sdk::{message::VersionedMessage, pubkey::Pubkey, transaction::VersionedTransaction};
use std::num::NonZeroU64;

pub(in crate::adapters::sdk::pump_curve_evidence::native_funding) struct NativeFinalObservation {
    template: NativeEnvelopeTemplate,
    bank: NativePostBank,
    output: NonZeroU64,
    network_fee: NonZeroU64,
    simulation_slot: u64,
}

impl NativeEnvelopeTemplate {
    fn evidence_accounts(&self) -> Vec<Pubkey> {
        let mut keys = self
            .source
            .source
            .observation
            .probe
            .evidence_accounts()
            .to_vec();
        keys.extend(self.destination);
        keys
    }

    async fn simulate<T: ReadHttpTransport>(
        self,
        runtime: &SharedReadRuntime<T>,
        url: &str,
    ) -> Result<NativeFinalObservation> {
        self.packet()?;
        ensure!(
            self.source
                .source
                .observation
                .probe
                .funding
                .curve
                .base_mint_layout
                .program
                == spl_token_interface::ID,
            "native Pump final simulation requires classic SPL Token"
        );
        let keys = self.evidence_accounts();
        let source = &self.source.source.observation;
        let minimum = source.simulation_slot;
        let until =
            source.probe.funding.started + crate::domain::preparation_balance::MAX_BALANCE_AGE;
        let request = PumpNativeFinalPacket::new(self.packet()?, &keys, minimum, until)?;
        let reply = runtime.pump_native_final(url, request).await?;
        let response = reply.for_template(self.packet()?, &keys, minimum)?;
        self.inspect_final(response)
    }

    fn inspect_final(self, response: Value) -> Result<NativeFinalObservation> {
        self.packet()?;
        ensure!(
            response.get("value").and_then(|value| value.get("err")) == Some(&Value::Null),
            "native Pump final observation requires explicit success"
        );
        let decoded: Response<RpcSimulateTransactionResult> = Response::deserialize(&response)
            .context("native Pump final observation is malformed")?;
        let source = &self.source.source.observation;
        let probe = &source.probe;
        ensure!(
            probe.funding.curve.base_mint_layout.program == spl_token_interface::ID,
            "native Pump final observation requires classic SPL Token"
        );
        ensure!(
            decoded.context.slot >= source.simulation_slot,
            "native Pump final observation precedes its quote"
        );
        ensure!(
            decoded.value.replacement_blockhash.is_none()
                && decoded
                    .value
                    .loaded_addresses
                    .as_ref()
                    .is_none_or(|loaded| loaded.writable.is_empty() && loaded.readonly.is_empty()),
            "native Pump final observation changed its fixed message context"
        );
        let VersionedMessage::Legacy(message) = &self.packet()?.message else {
            anyhow::bail!("native Pump final observation requires legacy")
        };
        let effects = effects::inspect(
            probe,
            message,
            message
                .instructions
                .last()
                .context("native Pump final observation omitted swap")?,
            &decoded.value,
            self.destination
                .map(|key| (key, tip_lamports(self.tip_sol))),
        )?;
        let expected_fee = source
            .network_fee
            .get()
            .checked_add(PRIORITY_LAMPORTS)
            .context("native Pump final fee overflows")?;
        ensure!(
            effects.fee.get() == expected_fee,
            "native Pump final fee differs from the frozen budget"
        );
        ensure!(
            effects.output >= self.source.minimum,
            "native Pump final output is below the frozen minimum"
        );
        let bank =
            probe.inspect_post_bank_message(&response, message, self.evidence_accounts().len())?;
        if let Some(destination) = self.destination {
            let index = message
                .account_keys
                .iter()
                .position(|key| *key == destination)
                .context("native Pump final tip role is missing")?;
            let account = decoded
                .value
                .accounts
                .as_ref()
                .and_then(|accounts| accounts.last())
                .and_then(Option::as_ref)
                .context("native Pump final tip metadata is missing")?;
            decode_data(
                account,
                solana_system_interface::program::ID,
                0,
                "native Pump final tip",
            )?;
            ensure!(
                decoded
                    .value
                    .post_balances
                    .as_ref()
                    .and_then(|balances| balances.get(index))
                    == Some(&account.lamports),
                "native Pump final tip metadata contradicts its balance"
            );
        }
        self.packet()?;
        Ok(NativeFinalObservation {
            template: self,
            bank,
            output: effects.output,
            network_fee: effects.fee,
            simulation_slot: decoded.context.slot,
        })
    }
}

impl NativeFinalObservation {
    pub(in crate::adapters::sdk::pump_curve_evidence::native_funding) fn packet(
        &self,
    ) -> Result<&VersionedTransaction> {
        let source = &self.template.source.source.observation;
        ensure!(
            self.output >= self.template.source.minimum
                && source.network_fee.get().checked_add(PRIORITY_LAMPORTS)
                    == Some(self.network_fee.get())
                && self.bank.snapshot.started == source.probe.funding.started
                && self.bank.snapshot.keys == source.probe.funding.keys
                && self.bank.snapshot.curve.slot == self.simulation_slot
                && self.simulation_slot >= source.simulation_slot,
            "native Pump final evidence changed after observation"
        );
        self.template.packet()
    }
}

mod bank_swap;
pub(crate) use bank_swap::NativeBoundIntent;
#[cfg(test)]
mod tests;
