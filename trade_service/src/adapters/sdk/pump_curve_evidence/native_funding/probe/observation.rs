//! Same-response consistency only. A bound transport and post-account proof are still required.
use super::{fresh, NativeQuoteProbe, Side};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status_client_types::{
    option_serializer::OptionSerializer, UiTransactionTokenBalance,
};
use std::num::NonZeroU64;

struct NativeProbeObservation {
    probe: NativeQuoteProbe,
    output: NonZeroU64,
    network_fee: NonZeroU64,
    simulation_slot: u64,
}

fn token_amount(
    rows: &[UiTransactionTokenBalance],
    index: usize,
    mint: Pubkey,
    owner: Pubkey,
    decimals: u8,
) -> Result<u64> {
    let row = rows
        .iter()
        .find(|row| usize::from(row.account_index) == index)
        .context("native Pump simulation token role is missing")?;
    ensure!(
        row.mint == mint.to_string()
            && row.ui_token_amount.decimals == decimals
            && matches!(&row.owner, OptionSerializer::Some(value) if value == &owner.to_string())
            && matches!(&row.program_id, OptionSerializer::Some(value) if value == &spl_token_interface::ID.to_string()),
        "native Pump simulation token identity differs"
    );
    let text = &row.ui_token_amount.amount;
    ensure!(
        !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()),
        "native Pump simulation token amount is not an unsigned integer"
    );
    text.parse()
        .context("native Pump simulation token amount overflows")
}

impl NativeQuoteProbe {
    // Consumes the probe, retaining its immutable packet with this local observation.
    // This accepts an unbound response for now; it cannot grant quote/signing authority.
    fn inspect_observation(self, response: Value) -> Result<NativeProbeObservation> {
        self.packet()?;
        ensure!(
            response.get("value").and_then(|value| value.get("err")) == Some(&Value::Null),
            "native Pump simulation requires explicit success"
        );
        let response: Response<RpcSimulateTransactionResult> = serde_json::from_value(response)
            .context("native Pump simulation response is malformed")?;
        ensure!(
            self.funding.curve.base_mint_layout.program == spl_token_interface::ID,
            "native Pump observation profile requires classic SPL Token"
        );
        ensure!(
            response.context.slot >= self.funding.curve.slot,
            "native Pump simulation context precedes its evidence"
        );
        let value = response.value;
        ensure!(value.err.is_none(), "native Pump simulation failed");
        ensure!(
            value.replacement_blockhash.is_none()
                && value.loaded_addresses.as_ref().is_none_or(|loaded| {
                    loaded.writable.is_empty() && loaded.readonly.is_empty()
                }),
            "native Pump simulation changed the fixed legacy context"
        );
        let effects = effects::inspect(
            &self,
            &self.transaction.message,
            &self.transaction.message.instructions[0],
            &value,
            None,
        )?;
        fresh(self.funding.started)?;
        Ok(NativeProbeObservation {
            probe: self,
            output: effects.output,
            network_fee: effects.fee,
            simulation_slot: response.context.slot,
        })
    }
}

mod effects;
mod post_bank;
mod shared;
mod tests;
