use super::post_bank::NativePostBank;
use super::{NativeProbeObservation, NativeQuoteProbe};
use crate::adapters::rpc::{
    pump_native_probe::{PumpNativeProbePacket, EVIDENCE_ACCOUNTS},
    read_http::ReadHttpTransport,
    read_runtime::SharedReadRuntime,
};
use anyhow::{ensure, Result};
use solana_sdk::pubkey::Pubkey;

struct NativeBankObservation {
    observation: NativeProbeObservation,
    bank: NativePostBank,
}

impl NativeQuoteProbe {
    fn evidence_accounts(&self) -> [Pubkey; EVIDENCE_ACCOUNTS] {
        let swap = &self.transaction.message.instructions[0];
        let buy = self.funding.side == super::Side::Buy;
        let roles = [1, if buy { 9 } else { 8 }, swap.accounts.len() - 1];
        std::array::from_fn(|index| {
            if index < self.funding.keys.len() {
                self.funding.keys[index]
            } else {
                self.transaction.message.account_keys[usize::from(swap.accounts[roles[index - 8]])]
            }
        })
    }

    // The bank and response are bound; recipient policy and final authorization remain unproven.
    async fn shared_observation<T: ReadHttpTransport>(
        self,
        runtime: &SharedReadRuntime<T>,
        url: &str,
    ) -> Result<NativeBankObservation> {
        self.packet()?;
        ensure!(
            self.funding.curve.base_mint_layout.program == spl_token_interface::ID,
            "native Pump shared observation requires classic SPL Token"
        );
        let keys = self.evidence_accounts();
        let minimum = self.funding.curve.slot;
        let packet = PumpNativeProbePacket::new(
            self.packet()?,
            &keys,
            minimum,
            self.funding.started + super::super::MAX_BALANCE_AGE,
        )?;
        let reply = runtime.pump_native_probe(url, packet).await?;
        let response = reply.for_probe(self.packet()?, &keys, minimum)?;
        let bank = self.inspect_post_bank(&response)?;
        let observation = self.inspect_observation(response)?;
        Ok(NativeBankObservation { observation, bank })
    }
}

mod tests;
