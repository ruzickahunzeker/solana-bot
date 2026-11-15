use crate::domain::trade::Address;
use std::time::{Duration, Instant};

pub(crate) const MAX_BALANCE_AGE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BalanceFailureKind {
    Invalid,
    Unavailable,
    Expired,
}

#[derive(Debug)]
pub(crate) struct PreparationBalanceFailure {
    kind: BalanceFailureKind,
    message: &'static str,
}
impl PreparationBalanceFailure {
    pub(crate) fn new(kind: BalanceFailureKind, message: &'static str) -> Self {
        Self { kind, message }
    }
    pub(crate) fn allows_lane_fallback(&self) -> bool {
        self.kind == BalanceFailureKind::Unavailable
    }
}
impl std::fmt::Display for PreparationBalanceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for PreparationBalanceFailure {}

/// Bound balance evidence from a read port, not protocol or signing authorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparationInput {
    pub(crate) account: Address,
    pub(crate) program: Address,
    pub(crate) decimals: u8,
}

pub(crate) struct PreparationBalanceSnapshot {
    wallet: Address,
    mint: Address,
    units: u64,
    program: Option<Address>,
    decimals: Option<u8>,
    input_account: Option<Address>,
    slot: u64,
    started: Instant,
}
impl PreparationBalanceSnapshot {
    fn from_metadata(
        wallet: Address,
        mint: Address,
        units: u64,
        program: Option<Address>,
        decimals: Option<u8>,
        slot: u64,
        started: Instant,
    ) -> anyhow::Result<Self> {
        if slot == 0 || (units > 0 && (program.is_none() || decimals.is_none())) {
            return Err(PreparationBalanceFailure::new(
                BalanceFailureKind::Invalid,
                "preparation balance metadata is incomplete",
            )
            .into());
        }
        let snapshot = Self {
            wallet,
            mint,
            units,
            program,
            decimals,
            input_account: None,
            slot,
            started,
        };
        snapshot.checked_units(wallet, mint)?;
        Ok(snapshot)
    }
    pub(crate) fn from_account(
        wallet: Address,
        mint: Address,
        units: u64,
        input: PreparationInput,
        slot: u64,
        started: Instant,
    ) -> anyhow::Result<Self> {
        let mut snapshot = Self::from_metadata(
            wallet,
            mint,
            units,
            Some(input.program),
            Some(input.decimals),
            slot,
            started,
        )?;
        snapshot.input_account = Some(input.account);
        Ok(snapshot)
    }
    pub(crate) fn checked_input(
        &self,
        wallet: Address,
        mint: Address,
    ) -> anyhow::Result<PreparationInput> {
        self.checked_units(wallet, mint)?;
        match (self.input_account, self.program, self.decimals) {
            (Some(account), Some(program), Some(decimals)) => Ok(PreparationInput {
                account,
                program,
                decimals,
            }),
            _ => Err(reject_balance("preparation raw input identity is missing")),
        }
    }
    pub(crate) fn checked_units(&self, wallet: Address, mint: Address) -> anyhow::Result<u64> {
        if self.wallet != wallet || self.mint != mint {
            return Err(PreparationBalanceFailure::new(
                BalanceFailureKind::Invalid,
                "preparation balance wallet or mint mismatch",
            )
            .into());
        }
        let age = Instant::now().checked_duration_since(self.started);
        if age.is_none_or(|age| age > MAX_BALANCE_AGE) {
            return Err(PreparationBalanceFailure::new(
                BalanceFailureKind::Expired,
                "preparation balance snapshot expired",
            )
            .into());
        }
        let _metadata = (self.program, self.decimals, self.slot);
        Ok(self.units)
    }
    pub(crate) fn recheck(&self, wallet: Address) -> anyhow::Result<()> {
        self.checked_units(wallet, self.mint).map(|_| ())
    }

    pub(crate) fn checked_slot(&self, wallet: Address, mint: Address) -> anyhow::Result<u64> {
        self.checked_units(wallet, mint)?;
        Ok(self.slot)
    }

    #[cfg(test)]
    pub(crate) fn fixture(wallet: Address, mint: Address, units: u64, started: Instant) -> Self {
        Self {
            wallet,
            mint,
            units,
            program: Some(Address::from_bytes([2; 32])),
            decimals: Some(6),
            input_account: None,
            slot: 42,
            started,
        }
    }
}

pub(crate) fn reject_balance(message: &'static str) -> anyhow::Error {
    PreparationBalanceFailure::new(BalanceFailureKind::Invalid, message).into()
}

#[cfg(test)]
mod tests;
