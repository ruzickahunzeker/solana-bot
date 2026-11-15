#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SolTransferAmount {
    Fixed(u64),
    Half,
    All,
}

impl SolTransferAmount {
    pub(crate) fn legacy_params(self) -> (Option<u64>, Option<u64>) {
        match self {
            Self::Fixed(lamports) => (Some(lamports), None),
            Self::Half => (None, Some(50)),
            Self::All => (None, Some(100)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SolTransferSnapshot {
    pub source: crate::domain::trade::Address,
    pub destination: crate::domain::trade::Address,
    pub amount_lamports: u64,
    pub fee_lamports: u64,
    pub balance_before_lamports: u64,
    pub reserve_lamports: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SolTransferIntent {
    pub idempotency_key: String,
    pub owner_type: String,
    pub owner_id: String,
    pub wallet_id: String,
    pub source: crate::domain::trade::Address,
    pub destination: crate::domain::trade::Address,
    pub amount: SolTransferAmount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SolTransferAttemptState {
    Reserved,
    Signed,
    Submitted,
    Confirmed,
    Unknown,
    Failed,
}

impl SolTransferAttemptState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Signed => "signed",
            Self::Submitted => "submitted",
            Self::Confirmed => "confirmed",
            Self::Unknown => "unknown",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        Ok(match value {
            "reserved" => Self::Reserved,
            "signed" => Self::Signed,
            "submitted" => Self::Submitted,
            "confirmed" => Self::Confirmed,
            "unknown" => Self::Unknown,
            "failed" => Self::Failed,
            _ => anyhow::bail!("invalid SOL transfer attempt state"),
        })
    }

    pub(crate) fn projection_status(self) -> &'static str {
        match self {
            Self::Reserved => "processing",
            Self::Signed => "unknown",
            _ => self.as_str(),
        }
    }
}
