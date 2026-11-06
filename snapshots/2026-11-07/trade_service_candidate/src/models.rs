use serde::{Deserialize, Serialize};

pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const SOL_MINT: &str = "So11111111111111111111111111111111111111111";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USD1_MINT: &str = "USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB";

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
    Aggregator,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatorSimulationReport {
    pub provider: String,
    pub quote_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_hash: Option<String>,
    pub expected_output_units: u64,
    pub minimum_output_units: u64,
    pub prioritization_fee_lamports: u64,
    pub compute_unit_limit: u64,
    pub explicit_system_spend_lamports: u64,
    pub simulated_fee_lamports: Option<u64>,
    pub simulated_payer_debit_lamports: Option<u64>,
    pub simulated_target_token_delta_units: Option<u64>,
    pub simulated_target_token_post_units: Option<u64>,
    pub simulated_units_consumed: Option<u64>,
    pub simulation_slot: u64,
    pub instruction_count: usize,
    pub address_lookup_table_count: usize,
    pub resolved_account_count: usize,
    pub instruction_audit_passed: bool,
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationReport {
    pub ok: bool,
    pub protocol: String,
    pub source: String,
    pub units_consumed: Option<u64>,
    pub logs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregator: Option<AggregatorSimulationReport>,
}

#[derive(Debug, Deserialize)]
pub struct WarmupRequest {
    pub pool: Pool,
    pub mint: String,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResolvePoolRequest {
    pub mint: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveSolHopRequest {
    pub quote_mint: String,
}

#[derive(Debug, Serialize)]
pub struct ResolveSolHopResponse {
    pub available: bool,
    pub pool: Option<Pool>,
    pub pool_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvePoolResponse {
    pub mint: String,
    pub pool: Pool,
    pub pool_id: Option<String>,
    pub quote_mint: String,
}

#[derive(Debug, Serialize)]
pub struct QuoteBalanceResponse {
    pub wallet_id: String,
    pub quote_mint: String,
    pub decimals: u8,
    pub balance_units: u64,
    pub balance_lamports: u64,
}

#[derive(Debug, Serialize)]
pub struct WarmupResponse {
    pub ok: bool,
    pub cache_key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TradeRequest {
    pub wallet_id: Option<String>,
    pub side: Side,
    pub pool: Pool,
    pub mint: String,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
    pub settlement_asset: Option<SettlementAsset>,
    pub sol_hop_pool: Option<Pool>,
    pub sol_hop_pool_id: Option<String>,
    /// Optional server-issued SDK via-SOL route snapshot. If omitted, the
    /// service resolves both legs itself before building the trade.
    #[serde(default)]
    pub sdk_route_id: Option<String>,
    /// Optional server-issued decision binding this SDK request to a wallet,
    /// normalized trade fingerprint, and short-lived verified route.
    #[serde(default)]
    pub sdk_decision_id: Option<String>,
    /// Short-lived proof of an unsigned two-hop SDK simulation. Accepted only
    /// by the candidate's explicit simulate-only PumpSwap bridge path.
    #[serde(default)]
    pub sdk_preview_id: Option<String>,
    pub amount_sol: Option<f64>,
    pub amount_lamports: Option<u64>,
    pub amount_quote: Option<f64>,
    pub amount_usdc: Option<f64>,
    pub amount_quote_units: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_percent: Option<f64>,
    pub slippage_bps: Option<u64>,
    pub tip_sol: Option<f64>,
    pub execution_method: Option<ExecutionMethod>,
    pub use_mev: Option<bool>,
    pub mev_provider: Option<MevProvider>,
    pub simulate: Option<bool>,
    pub source: Option<String>,
    pub source_signature: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettlementAsset {
    Quote,
    Sol,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TradeRecord {
    pub request_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub side: Side,
    pub pool: Pool,
    pub mint: String,
    pub pool_id: Option<String>,
    pub execution_method: ExecutionMethod,
    pub status: TradeStatus,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub simulation: Option<SimulationReport>,
    pub duplicate: bool,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct TradeResponse {
    pub request_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub execution_method: ExecutionMethod,
    pub status: TradeStatus,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub simulation: Option<SimulationReport>,
    pub duplicate: bool,
}

impl From<TradeRecord> for TradeResponse {
    fn from(record: TradeRecord) -> Self {
        Self {
            request_id: record.request_id,
            wallet_id: record.wallet_id,
            wallet_pubkey: record.wallet_pubkey,
            execution_method: record.execution_method,
            status: record.status,
            signature: record.signature,
            error: record.error,
            simulation: record.simulation,
            duplicate: record.duplicate,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub ready: bool,
    pub wallet: String,
    pub default_wallet_id: String,
    pub wallet_count: usize,
    pub wallets: Vec<WalletInfo>,
    pub has_blockhash: bool,
    pub blockhash_age_ms: Option<u128>,
    pub rpc_ok: bool,
    pub paused: bool,
    pub rpc_lanes: usize,
    pub swqos_lanes: Vec<String>,
    pub active_swqos_lanes: Vec<String>,
    pub mev_protection: bool,
    pub require_mev_submit: bool,
    pub pumpapi_executor: bool,
    pub trade_persistence_mode: String,
    pub trade_database_healthy: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct WalletInfo {
    pub wallet_id: String,
    pub pubkey: String,
}

#[derive(Debug, Serialize)]
pub struct WalletsResponse {
    pub default_wallet_id: String,
    pub wallets: Vec<WalletInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWalletRequest {
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub private_key_base58: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWalletRequest {
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct ManagedWalletResponse {
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub pubkey: String,
    pub is_default: bool,
    pub enabled: bool,
    pub created: bool,
}

#[derive(Debug, Serialize)]
pub struct ManagedWalletsResponse {
    pub wallets: Vec<ManagedWalletResponse>,
}

#[derive(Debug, Serialize)]
pub struct ExportWalletResponse {
    pub wallet_id: String,
    pub private_key_base58: String,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct CreateVanityJobRequest {
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub match_side: VanityMatchSide,
    pub pattern: String,
    pub max_attempts: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct WalletBalanceItem {
    pub wallet_id: String,
    pub pubkey: String,
    pub balance_lamports: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WalletBalancesResponse {
    pub fetched_at_ms: u128,
    pub wallets: Vec<WalletBalanceItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SolTransferAmountMode {
    Fixed,
    Percent,
}

impl SolTransferAmountMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Percent => "percent",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SolTransferRequest {
    pub owner_type: String,
    pub owner_id: String,
    pub to_wallet_id: Option<String>,
    pub to_address: Option<String>,
    pub amount_mode: SolTransferAmountMode,
    pub amount_lamports: Option<u64>,
    pub percent: Option<u64>,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct SolTransferResponse {
    pub transfer_id: String,
    pub wallet_id: String,
    pub from_pubkey: String,
    pub to_pubkey: String,
    pub status: String,
    pub amount_lamports: Option<u64>,
    pub fee_lamports: Option<u64>,
    pub balance_before_lamports: Option<u64>,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub duplicate: bool,
}

#[derive(Debug, Deserialize)]
pub struct BatchTradeRequest {
    pub wallet_ids: Vec<String>,
    pub trade: TradeRequest,
}

#[derive(Debug, Serialize)]
pub struct BatchTradeResponse {
    pub items: Vec<BatchTradeItemResponse>,
}

#[derive(Debug, Serialize)]
pub struct BatchTradeItemResponse {
    pub wallet_id: String,
    pub request_id: Option<String>,
    pub status: Option<TradeStatus>,
    pub error: Option<String>,
    pub duplicate: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
