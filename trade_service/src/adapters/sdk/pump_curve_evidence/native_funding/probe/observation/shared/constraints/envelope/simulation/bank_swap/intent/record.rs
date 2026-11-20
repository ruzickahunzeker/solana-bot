use super::NativeBoundIntent;
use crate::{
    adapters::sdk::damm_instructions::tip_lamports,
    application::sdk_preparation::validate_pump_history_intent,
    domain::pump_native_candidate::{PumpNativeCandidateRecord, PumpNativeDetails},
};
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::swqos::SwqosType;

impl NativeBoundIntent<'_> {
    pub(in crate::adapters::sdk) fn record(&self) -> Result<PumpNativeCandidateRecord> {
        let packet = self.packet()?;
        let final_bank = &self.observation;
        let envelope = &final_bank.template;
        let constraints = &envelope.source;
        let quote = &constraints.source.observation;
        let probe = &quote.probe;
        let funding = &probe.funding;
        let destinations = funding.destinations()?;
        let command = self.execution.command();
        let details = PumpNativeDetails {
            request_id: self.anchor.request_id().into(),
            wallet_id: self.anchor.wallet_id().into(),
            idempotency_key: self.anchor.idempotency_key().into(),
            request_hash: self.anchor.request_hash().into(),
            wallet: funding.keys[6].to_bytes(),
            mint: funding.keys[1].to_bytes(),
            curve: funding.keys[0].to_bytes(),
            curve_vault: funding.keys[4].to_bytes(),
            user_ata: funding.keys[5].to_bytes(),
            base_program: funding.curve.base_mint_layout.program.to_bytes(),
            base_decimals: funding.curve.base_mint_layout.decimals,
            creator: funding.curve.curve.creator.to_bytes(),
            protocol_fee: destinations.protocol_fee.to_bytes(),
            creator_vault: destinations.creator_vault.to_bytes(),
            buyback: probe
                .instruction
                .accounts
                .last()
                .context("native Pump record buyback role is missing")?
                .pubkey
                .to_bytes(),
            cashback: funding.curve.curve.cashback,
            side: funding.side,
            input_units: probe.input.get(),
            source_token_units: funding.user_base_units,
            sell_percent: command.amount.sell_percent(),
            quoted_output_units: quote.output.get(),
            minimum_output_units: constraints.minimum.get(),
            final_output_units: final_bank.output.get(),
            source_slot: funding.curve.slot,
            quote_slot: quote.simulation_slot,
            final_slot: final_bank.simulation_slot,
            final_epoch: final_bank.bank.snapshot.epoch,
            slippage_bps: constraints.slippage_bps,
            tip_sol_bits: envelope.tip_sol.to_bits(),
            relay: envelope.provider.as_str().into(),
            tip_destination: envelope.destination.map(|value| value.to_bytes()),
            tip_lamports: if envelope.provider == SwqosType::Default {
                0
            } else {
                tip_lamports(envelope.tip_sol)
            },
            network_fee_lamports: final_bank.network_fee.get(),
            priority_fee_lamports: super::super::super::super::PRIORITY_LAMPORTS,
            native_before_lamports: constraints.native.available_before,
            maximum_native_debit: envelope.native.maximum_debit,
            remaining_native_lamports: envelope.native.remaining_without_output,
            recent_blockhash: packet.message.recent_blockhash().to_bytes(),
            use_mev: command.execution.use_mev,
            requested_mev_provider: command.execution.mev_provider,
        };
        let record = PumpNativeCandidateRecord::new(details, &packet.message.serialize())?;
        reconstruction::validate(&record, &packet.message.serialize())?;
        validate_pump_history_intent(self.execution, &record)?;
        self.packet()?;
        Ok(record)
    }

    pub(in crate::adapters::sdk) fn check_record(
        &self,
        record: &PumpNativeCandidateRecord,
        message: &[u8],
    ) -> Result<()> {
        ensure!(
            message == self.packet()?.message.serialize(),
            "native Pump record differs from the original frozen message"
        );
        record.validate(message)?;
        reconstruction::validate(record, message)?;
        ensure!(
            *record == self.record()?,
            "native Pump record differs from the original admitted evidence"
        );
        self.packet()?;
        Ok(())
    }
}

impl PumpNativeCandidateRecord {
    #[cfg(test)]
    pub(crate) async fn history_fixture(
        side: sol_trade_sdk::swqos::TradeType,
        provider: SwqosType,
    ) -> (
        crate::application::ordinary_claims::test_support::Fixture,
        Self,
        Vec<u8>,
    ) {
        let (fixture, observation) =
            super::tests::admitted(crate::config::TradePersistenceMode::Durable, side, provider)
                .await;
        let (record, message) = {
            let bound = observation.bind_intent(&fixture.execution).unwrap();
            (
                bound.record().unwrap(),
                bound.packet().unwrap().message.serialize(),
            )
        };
        (fixture, record, message)
    }
}

mod reconstruction;
#[cfg(test)]
mod tests;
