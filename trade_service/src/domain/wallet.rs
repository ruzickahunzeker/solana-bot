#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QuoteBalance {
    pub decimals: u8,
    pub balance_units: u64,
    pub balance_lamports: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletDescriptor {
    pub wallet_id: String,
    pub pubkey: crate::domain::trade::Address,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WalletMetadata {
    pub wallet_id: String,
    pub pubkey: String,
    pub owner_type: String,
    pub owner_id: String,
    pub enabled: bool,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct WalletInfo {
    pub wallet_id: String,
    pub pubkey: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagedWallet {
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub pubkey: String,
    pub is_default: bool,
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VanityMatchSide {
    Prefix,
    Suffix,
}

impl VanityMatchSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Prefix => "prefix",
            Self::Suffix => "suffix",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VanityJob {
    pub job_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub match_side: String,
    pub pattern: String,
    pub status: String,
    pub attempts: u64,
    pub max_attempts: u64,
    pub wallet_id: Option<String>,
    pub pubkey: Option<String>,
    pub error: Option<String>,
}
