use serde::{Deserialize, Serialize};

pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const SOL_MINT: &str = "So11111111111111111111111111111111111111111";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USD1_MINT: &str = "USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB";

// These shared identities retain their existing V1 wire names and aliases.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Pool {
    Pump,
    PumpAmm,
    Bonk,
    #[serde(rename = "stonkfun", alias = "stonk-fun")]
    StonkFun,
    #[serde(rename = "stonkfun-swap", alias = "stonk-fun-swap")]
    StonkFunSwap,
    MeteoraDammV2,
    MeteoraDbc,
    RaydiumAmmV4,
    RaydiumCpmm,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMethod {
    Sdk,
    #[serde(rename = "pumpapi", alias = "pump-api")]
    Pumpapi,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MevProvider {
    Auto,
    #[serde(alias = "rpc")]
    Default,
    Jito,
    Temporal,
    #[serde(rename = "flashblock", alias = "flash-block")]
    FlashBlock,
    #[serde(rename = "blockrazor", alias = "block-razor")]
    BlockRazor,
    Astralane,
    #[serde(alias = "speed-landing")]
    Speedlanding,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TradeStatus {
    Accepted,
    WarmingParams,
    BuildingTx,
    Submitting,
    Simulated,
    Submitted,
    Confirmed,
    Failed,
    Rejected,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettlementAsset {
    Quote,
    Sol,
}

/// Validated address bytes; chain SDK representations belong at adapter boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address([u8; 32]);

impl Address {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&bs58::encode(self.0).into_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TradeAmount {
    BuySol { lamports: u64 },
    BuyQuote { mint: Address, units: u64 },
    SellPercent { percent: u64 },
}

impl TradeAmount {
    pub fn buy_units(self) -> Option<u64> {
        match self {
            Self::BuySol { lamports } => Some(lamports),
            Self::BuyQuote { units, .. } => Some(units),
            Self::SellPercent { .. } => None,
        }
    }

    pub fn sell_percent(self) -> Option<u64> {
        match self {
            Self::SellPercent { percent } => Some(percent),
            _ => None,
        }
    }

    pub fn side(self) -> Side {
        match self {
            Self::BuySol { .. } | Self::BuyQuote { .. } => Side::Buy,
            Self::SellPercent { .. } => Side::Sell,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionMode {
    Live,
    Simulate,
}

impl ExecutionMode {
    pub fn resolve(requested: Option<bool>, default_simulate: bool) -> Self {
        if requested.unwrap_or(default_simulate) {
            Self::Simulate
        } else {
            Self::Live
        }
    }

    pub fn simulate(self) -> bool {
        self == Self::Simulate
    }
}
