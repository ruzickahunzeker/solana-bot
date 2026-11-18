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
        let before = value
            .pre_balances
            .context("native Pump simulation omitted pre balances")?;
        let after = value
            .post_balances
            .context("native Pump simulation omitted post balances")?;
        let fee = value
            .fee
            .and_then(NonZeroU64::new)
            .context("native Pump simulation omitted a positive network fee")?;
        let keys = &self.transaction.message.account_keys;
        let index = |key| {
            keys.iter()
                .position(|candidate| *candidate == key)
                .context("native Pump simulation role is not in its message")
        };
        ensure!(
            before.len() == keys.len() && after.len() == keys.len(),
            "native Pump simulation native index count differs"
        );
        let wallet = index(self.funding.keys[6])?;
        let curve = index(self.funding.keys[0])?;
        let user_base = index(self.funding.keys[5])?;
        let curve_base = index(self.funding.keys[4])?;
        let compiled = &self.transaction.message.instructions[0];
        let buy = self.funding.side == Side::Buy;
        let recipients = [
            usize::from(compiled.accounts[1]),
            usize::from(compiled.accounts[if buy { 9 } else { 8 }]),
            usize::from(
                *compiled
                    .accounts
                    .last()
                    .context("native Pump recipient is missing")?,
            ),
        ];
        ensure!(
            before[wallet] >= fee.get(),
            "native Pump simulation cannot prefund its fee"
        );
        for i in 0..keys.len() {
            if recipients.contains(&i) {
                ensure!(
                    before[i] > 0 && after[i] >= before[i],
                    "native Pump simulation recipient is absent or debited"
                );
            } else if i != wallet && i != curve {
                ensure!(
                    before[i] == after[i],
                    "native Pump simulation changed native funds outside its profile"
                );
            }
        }
        let total_before: u128 = before.iter().map(|amount| u128::from(*amount)).sum();
        let total_after: u128 = after.iter().map(|amount| u128::from(*amount)).sum();
        ensure!(
            total_before == total_after + u128::from(fee.get()),
            "native Pump simulation native balances do not conserve funds"
        );
        let pre_tokens = value
            .pre_token_balances
            .context("native Pump simulation omitted pre token balances")?;
        let post_tokens = value
            .post_token_balances
            .context("native Pump simulation omitted post token balances")?;
        let amounts = |rows: &[UiTransactionTokenBalance]| -> Result<(u64, u64)> {
            ensure!(
                rows.len() == 2
                    && rows.iter().all(|row| {
                        let i = usize::from(row.account_index);
                        i == user_base || i == curve_base
                    })
                    && rows[0].account_index != rows[1].account_index,
                "native Pump simulation token index set differs"
            );
            let mint = self.funding.keys[1];
            let decimals = self.funding.curve.base_mint_layout.decimals;
            Ok((
                token_amount(rows, user_base, mint, keys[wallet], decimals)?,
                token_amount(rows, curve_base, mint, keys[curve], decimals)?,
            ))
        };
        let (user_before, curve_before) = amounts(&pre_tokens)?;
        let (user_after, curve_after) = amounts(&post_tokens)?;
        let output = if buy {
            ensure!(
                before[wallet]
                    .checked_sub(after[wallet])
                    .and_then(|amount| amount.checked_sub(fee.get()))
                    == Some(self.input.get())
                    && after[curve] >= before[curve],
                "native Pump simulation buy spend differs from exact input"
            );
            let output = user_after
                .checked_sub(user_before)
                .context("native Pump buy token delta is negative")?;
            ensure!(
                curve_before.checked_sub(curve_after) == Some(output),
                "native Pump simulation buy token conservation differs"
            );
            output
        } else {
            ensure!(
                user_before.checked_sub(user_after) == Some(self.input.get())
                    && curve_after.checked_sub(curve_before) == Some(self.input.get())
                    && before[curve] >= after[curve],
                "native Pump simulation sell token input differs"
            );
            // Include the fee even when the native payout is smaller than it; never saturate.
            let net =
                i128::from(after[wallet]) - i128::from(before[wallet]) + i128::from(fee.get());
            u64::try_from(net).context("native Pump simulation sell output is invalid")?
        };
        let output = NonZeroU64::new(output).context("native Pump simulation output is zero")?;
        fresh(self.funding.started)?;
        Ok(NativeProbeObservation {
            probe: self,
            output,
            network_fee: fee,
            simulation_slot: response.context.slot,
        })
    }
}

mod shared;
mod tests;
