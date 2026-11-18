//! One native/token effect checker for discovery and the fixed complete envelope.
use super::{token_amount, NativeQuoteProbe, Side};
use anyhow::{ensure, Context, Result};
use solana_client::rpc_response::RpcSimulateTransactionResult;
use solana_sdk::{
    message::{compiled_instruction::CompiledInstruction, Message},
    pubkey::Pubkey,
};
use solana_transaction_status_client_types::UiTransactionTokenBalance;
use std::num::NonZeroU64;

pub(super) struct NativeEffects {
    pub(super) output: NonZeroU64,
    pub(super) fee: NonZeroU64,
}

pub(super) fn inspect(
    probe: &NativeQuoteProbe,
    message: &Message,
    compiled: &CompiledInstruction,
    value: &RpcSimulateTransactionResult,
    tip: Option<(Pubkey, u64)>,
) -> Result<NativeEffects> {
    let before = value
        .pre_balances
        .as_ref()
        .context("native Pump simulation omitted pre balances")?;
    let after = value
        .post_balances
        .as_ref()
        .context("native Pump simulation omitted post balances")?;
    let fee = value
        .fee
        .and_then(NonZeroU64::new)
        .context("native Pump simulation omitted a positive network fee")?;
    let keys = &message.account_keys;
    let index = |key| {
        keys.iter()
            .position(|candidate| *candidate == key)
            .context("native Pump simulation role is not in its message")
    };
    ensure!(
        before.len() == keys.len() && after.len() == keys.len(),
        "native Pump simulation native index count differs"
    );
    let wallet = index(probe.funding.keys[6])?;
    let curve = index(probe.funding.keys[0])?;
    let user_base = index(probe.funding.keys[5])?;
    let curve_base = index(probe.funding.keys[4])?;
    let buy = probe.funding.side == Side::Buy;
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
    let tip = tip
        .map(|(key, amount)| -> Result<_> {
            ensure!(
                !probe.transaction.message.account_keys.contains(&key),
                "native Pump simulation tip aliases an original account"
            );
            Ok((index(key)?, amount))
        })
        .transpose()?;
    let tip_lamports = tip.map(|(_, amount)| amount).unwrap_or(0);
    ensure!(
        fee.get()
            .checked_add(tip_lamports)
            .is_some_and(|debit| before[wallet] >= debit),
        "native Pump simulation cannot prefund its fee"
    );
    for i in 0..keys.len() {
        if let Some((_, amount)) = tip.filter(|(index, _)| *index == i) {
            ensure!(
                before[i] > 0 && after[i].checked_sub(before[i]) == Some(amount),
                "native Pump simulation tip is absent or differs from the frozen transfer"
            );
        } else if recipients.contains(&i) {
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
        .as_ref()
        .context("native Pump simulation omitted pre token balances")?;
    let post_tokens = value
        .post_token_balances
        .as_ref()
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
        let mint = probe.funding.keys[1];
        let decimals = probe.funding.curve.base_mint_layout.decimals;
        Ok((
            token_amount(rows, user_base, mint, keys[wallet], decimals)?,
            token_amount(rows, curve_base, mint, keys[curve], decimals)?,
        ))
    };
    let (user_before, curve_before) = amounts(pre_tokens)?;
    let (user_after, curve_after) = amounts(post_tokens)?;
    let output = if buy {
        ensure!(
            before[wallet]
                .checked_sub(after[wallet])
                .and_then(|amount| amount.checked_sub(fee.get()))
                .and_then(|amount| amount.checked_sub(tip_lamports))
                == Some(probe.input.get())
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
            user_before.checked_sub(user_after) == Some(probe.input.get())
                && curve_after.checked_sub(curve_before) == Some(probe.input.get())
                && before[curve] >= after[curve],
            "native Pump simulation sell token input differs"
        );
        // Include fee and the explicit envelope transfer even if payout is smaller.
        let net = i128::from(after[wallet]) - i128::from(before[wallet])
            + i128::from(fee.get())
            + i128::from(tip_lamports);
        u64::try_from(net).context("native Pump simulation sell output is invalid")?
    };
    let output = NonZeroU64::new(output).context("native Pump simulation output is zero")?;
    Ok(NativeEffects { output, fee })
}
