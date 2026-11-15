use anyhow::{bail, Context, Result};

use crate::domain::sol_transfer::SolTransferAmount;
use crate::domain::sol_transfer::{SolTransferIntent, SolTransferSnapshot};

#[derive(Debug, Clone, Copy)]
pub(crate) enum TransferAmountMode {
    Fixed,
    Percent,
}

pub(crate) struct TransferAmountInput {
    pub mode: TransferAmountMode,
    pub fixed_lamports: Option<u64>,
    pub percent: Option<u64>,
}

pub(crate) fn validate_key(key: &str) -> Result<()> {
    if key.trim().is_empty() || key.len() > 128 {
        bail!("idempotency_key must be 1..128 characters");
    }
    Ok(())
}

pub(crate) fn select_amount(input: TransferAmountInput) -> Result<SolTransferAmount> {
    match (input.mode, input.fixed_lamports, input.percent) {
        (TransferAmountMode::Fixed, Some(amount), None) if amount > 0 => {
            Ok(SolTransferAmount::Fixed(amount))
        }
        (TransferAmountMode::Percent, None, Some(50)) => Ok(SolTransferAmount::Half),
        (TransferAmountMode::Percent, None, Some(100)) => Ok(SolTransferAmount::All),
        _ => bail!("fixed requires amount_lamports; percent supports only 50 or 100"),
    }
}

pub(crate) const SOL_TRANSFER_SAFETY_MARGIN_LAMPORTS: u64 = 10_000;

pub(crate) fn validate_snapshot(
    intent: &SolTransferIntent,
    snapshot: &SolTransferSnapshot,
) -> Result<()> {
    anyhow::ensure!(
        snapshot.source == intent.source && snapshot.destination == intent.destination,
        "SOL transfer snapshot identity mismatch"
    );
    let rent = if intent.amount == SolTransferAmount::All {
        anyhow::ensure!(
            snapshot.reserve_lamports == 0,
            "100 percent SOL transfer must not reserve rent"
        );
        0
    } else {
        snapshot
            .reserve_lamports
            .checked_sub(SOL_TRANSFER_SAFETY_MARGIN_LAMPORTS)
            .ok_or_else(|| anyhow::anyhow!("SOL transfer reserve is below safety margin"))?
    };
    let (fixed, percent) = intent.amount.legacy_params();
    let calculated = calculate_sol_transfer_amount(
        snapshot.balance_before_lamports,
        snapshot.fee_lamports,
        rent,
        fixed,
        percent,
    )?;
    anyhow::ensure!(
        calculated == (snapshot.amount_lamports, snapshot.reserve_lamports),
        "SOL transfer snapshot amount mismatch"
    );
    Ok(())
}

// Retains the legacy raw-input error ordering for SDK callers as well as validated intents.
pub(crate) fn calculate_sol_transfer_amount(
    balance: u64,
    fee: u64,
    rent_exempt_minimum: u64,
    fixed_lamports: Option<u64>,
    percent: Option<u64>,
) -> Result<(u64, u64)> {
    if fixed_lamports.is_none() && percent == Some(100) {
        let amount = balance
            .checked_sub(fee)
            .context("insufficient SOL balance for network fee")?;
        if amount == 0 {
            bail!("insufficient SOL balance after network fee");
        }
        return Ok((amount, 0));
    }

    let reserve = rent_exempt_minimum.saturating_add(SOL_TRANSFER_SAFETY_MARGIN_LAMPORTS);
    let spendable = balance
        .checked_sub(fee.saturating_add(reserve))
        .with_context(|| {
            format!(
                "insufficient SOL balance for fee and rent reserve: balance={balance}, fee={fee}, reserve={reserve} lamports"
            )
        })?;
    let amount = match (fixed_lamports, percent) {
        (Some(amount), None) if amount > 0 => amount,
        (None, Some(50)) => spendable / 2,
        _ => bail!("provide either a positive fixed amount or percent 50/100"),
    };
    if amount == 0 || amount > spendable {
        bail!(
            "insufficient SOL balance: spendable={} lamports, requested={} lamports",
            spendable,
            amount
        );
    }
    Ok((amount, reserve))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_amount_selection_preserves_mutual_exclusion_and_full_integer_values() {
        for mode in [TransferAmountMode::Fixed, TransferAmountMode::Percent] {
            for fixed in [
                None,
                Some(0),
                Some(1),
                Some(9_007_199_254_740_993),
                Some(u64::MAX),
            ] {
                for percent in [None, Some(0), Some(50), Some(100), Some(u64::MAX)] {
                    let result = select_amount(TransferAmountInput {
                        mode,
                        fixed_lamports: fixed,
                        percent,
                    });
                    let allowed = match mode {
                        TransferAmountMode::Fixed => {
                            fixed.is_some_and(|value| value > 0) && percent.is_none()
                        }
                        TransferAmountMode::Percent => {
                            fixed.is_none() && matches!(percent, Some(50 | 100))
                        }
                    };
                    assert_eq!(result.is_ok(), allowed);
                    match result {
                        Ok(amount) => assert_eq!(amount.legacy_params(), (fixed, percent)),
                        Err(error) => assert_eq!(
                            error.to_string(),
                            "fixed requires amount_lamports; percent supports only 50 or 100"
                        ),
                    }
                }
            }
        }
    }

    #[test]
    fn transfer_key_validation_keeps_byte_limit_and_only_uses_trim_for_emptiness() {
        for key in [
            "",
            " \t\n",
            "\u{2003}",
            &"x".repeat(129),
            &"\u{00e9}".repeat(65),
        ] {
            assert_eq!(
                validate_key(key).unwrap_err().to_string(),
                "idempotency_key must be 1..128 characters"
            );
        }
        for key in [" literal-key ", &"x".repeat(128), &"\u{00e9}".repeat(64)] {
            assert!(validate_key(key).is_ok());
        }
    }

    #[test]
    fn validated_transfer_amounts_use_original_fee_and_rent_policy() {
        assert_eq!(
            calculate_sol_transfer_amount(1_000_000, 5000, 890_880, Some(94_120), None).unwrap(),
            (94_120, 900_880)
        );
        assert_eq!(
            calculate_sol_transfer_amount(1_000_000, 5000, 890_880, None, Some(50)).unwrap(),
            (47_060, 900_880)
        );
        assert_eq!(
            calculate_sol_transfer_amount(1_000_000, 5000, u64::MAX, None, Some(100)).unwrap(),
            (995_000, 0)
        );
    }
}
