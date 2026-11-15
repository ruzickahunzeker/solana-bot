use serde::Serialize;

pub use crate::api::dto::pools::{
    ResolvePoolRequest, ResolvePoolResponse, ResolveSolHopRequest, ResolveSolHopResponse,
    WarmupRequest, WarmupResponse,
};
pub use crate::api::dto::readiness::ReadyResponse;
pub use crate::api::dto::trades::{
    BatchTradeItemResponse, BatchTradeRequest, BatchTradeResponse, TradeRequest, TradeResponse,
};
pub use crate::application::trade_record::TradeRecordV1 as TradeRecord;
pub use crate::domain::trade::{
    ExecutionMethod, MevProvider, Pool, SettlementAsset, Side, TradeStatus, SOL_MINT, USD1_MINT,
    USDC_MINT, WSOL_MINT,
};

#[cfg(test)]
pub use crate::api::dto::wallets::WalletBalanceItem;
pub use crate::api::dto::wallets::{
    CreateVanityJobRequest, CreateWalletRequest, ExportWalletResponse, ManagedWalletResponse,
    ManagedWalletsResponse, QuoteBalanceResponse, UpdateWalletRequest, WalletBalancesResponse,
    WalletsResponse,
};

pub use crate::domain::wallet::WalletInfo;

pub use crate::domain::wallet::VanityMatchSide;

#[allow(unused_imports)]
pub use crate::api::dto::sol_transfers::SolTransferAmountMode;
#[cfg(test)]
pub use crate::api::dto::sol_transfers::SolTransferRequest;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
