use serde::{Deserialize, Serialize};

use crate::application::vanity::VanityCreateInput;
use crate::application::wallet_balances::{QuoteBalanceView, WalletBalancesSnapshot};
use crate::application::wallet_management::{
    CreateWalletInput, ExportedWallet, LoadedWallets, ManagedWalletView,
};
use crate::models::{VanityMatchSide, WalletInfo};

#[derive(Debug, Deserialize)]
pub(crate) struct OwnerQuery {
    pub(crate) owner_type: String,
    pub(crate) owner_id: String,
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
pub struct CreateVanityJobRequest {
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub match_side: VanityMatchSide,
    pub pattern: String,
    pub max_attempts: Option<u64>,
}

impl From<CreateVanityJobRequest> for VanityCreateInput {
    fn from(request: CreateVanityJobRequest) -> Self {
        Self {
            owner_type: request.owner_type,
            owner_id: request.owner_id,
            label: request.label,
            match_side: request.match_side,
            pattern: request.pattern,
            max_attempts: request.max_attempts,
        }
    }
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

impl From<QuoteBalanceView> for QuoteBalanceResponse {
    fn from(view: QuoteBalanceView) -> Self {
        Self {
            wallet_id: view.wallet_id,
            quote_mint: view.quote_mint.to_string(),
            decimals: view.balance.decimals,
            balance_units: view.balance.balance_units,
            balance_lamports: view.balance.balance_lamports,
        }
    }
}

impl From<CreateWalletRequest> for CreateWalletInput {
    fn from(request: CreateWalletRequest) -> Self {
        Self {
            owner_type: request.owner_type,
            owner_id: request.owner_id,
            label: request.label,
            private_key_base58: request.private_key_base58,
        }
    }
}

impl From<LoadedWallets> for WalletsResponse {
    fn from(loaded: LoadedWallets) -> Self {
        Self {
            default_wallet_id: loaded.default_wallet_id,
            wallets: loaded.wallets,
        }
    }
}

impl From<ManagedWalletView> for ManagedWalletResponse {
    fn from(view: ManagedWalletView) -> Self {
        let wallet = view.wallet;
        Self {
            wallet_id: wallet.wallet_id,
            owner_type: wallet.owner_type,
            owner_id: wallet.owner_id,
            label: wallet.label,
            pubkey: wallet.pubkey,
            is_default: wallet.is_default,
            enabled: wallet.enabled,
            created: view.created,
        }
    }
}

impl From<Vec<ManagedWalletView>> for ManagedWalletsResponse {
    fn from(wallets: Vec<ManagedWalletView>) -> Self {
        Self {
            wallets: wallets
                .into_iter()
                .map(ManagedWalletResponse::from)
                .collect(),
        }
    }
}

impl From<ExportedWallet> for ExportWalletResponse {
    fn from(exported: ExportedWallet) -> Self {
        Self {
            wallet_id: exported.wallet_id,
            private_key_base58: exported.private_key_base58,
        }
    }
}

impl From<WalletBalancesSnapshot> for WalletBalancesResponse {
    fn from(snapshot: WalletBalancesSnapshot) -> Self {
        Self {
            fetched_at_ms: snapshot.fetched_at_ms,
            wallets: snapshot
                .wallets
                .into_iter()
                .map(|row| WalletBalanceItem {
                    wallet_id: row.wallet_id,
                    pubkey: row.pubkey,
                    balance_lamports: row.balance_lamports,
                    error: row.error,
                })
                .collect(),
        }
    }
}
