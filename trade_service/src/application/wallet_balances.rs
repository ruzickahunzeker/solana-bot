use std::{future::Future, pin::Pin};

use crate::{
    application::ports::{
        address_parser::TradeAddressParser,
        wallet_balances::{WalletBalanceRuntime, WalletMetadataSource},
        wallet_reads::WalletReadPort,
    },
    domain::{
        trade::Address,
        wallet::{QuoteBalance, WalletMetadata},
    },
};

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BalanceFailureKind {
    NotFound,
    OwnerMismatch,
    InvalidMint,
    Unavailable,
    InvalidMetadata,
    Backend,
}

#[derive(Debug)]
pub(crate) struct BalanceQueryError {
    pub kind: BalanceFailureKind,
    pub message: String,
}

impl BalanceQueryError {
    fn new(kind: BalanceFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct QuoteBalanceView {
    pub wallet_id: String,
    pub quote_mint: Address,
    pub balance: QuoteBalance,
}

#[derive(Debug)]
pub(crate) struct WalletBalanceRow {
    pub wallet_id: String,
    pub pubkey: String,
    pub balance_lamports: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct WalletBalancesSnapshot {
    pub fetched_at_ms: u128,
    pub wallets: Vec<WalletBalanceRow>,
}

pub(crate) type BalanceQueryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, BalanceQueryError>> + Send + 'a>>;

pub(crate) trait WalletBalanceQueries: Send + Sync {
    fn quote<'a>(
        &'a self,
        wallet_id: &'a str,
        mint: &'a str,
        owner_type: &'a str,
        owner_id: &'a str,
    ) -> BalanceQueryFuture<'a, QuoteBalanceView>;
    fn native<'a>(
        &'a self,
        owner_type: &'a str,
        owner_id: &'a str,
    ) -> BalanceQueryFuture<'a, WalletBalancesSnapshot>;
}

pub(crate) struct WalletBalanceService<M, R, P, C> {
    metadata: M,
    runtime: R,
    parser: P,
    clock: C,
}

impl<M, R, P, C> WalletBalanceService<M, R, P, C> {
    pub(crate) fn new(metadata: M, runtime: R, parser: P, clock: C) -> Self {
        Self {
            metadata,
            runtime,
            parser,
            clock,
        }
    }
}

fn visible(wallet: &WalletMetadata, owner_type: &str, owner_id: &str) -> bool {
    wallet.enabled
        && ((wallet.owner_type == owner_type && wallet.owner_id == owner_id)
            || (wallet.owner_type == "system" && wallet.owner_id == "global"))
}

impl<M, R, P, C> WalletBalanceQueries for WalletBalanceService<M, R, P, C>
where
    M: WalletMetadataSource,
    R: WalletBalanceRuntime,
    P: TradeAddressParser + Send + Sync,
    C: Fn() -> u128 + Send + Sync,
{
    fn quote<'a>(
        &'a self,
        wallet_id: &'a str,
        mint: &'a str,
        owner_type: &'a str,
        owner_id: &'a str,
    ) -> BalanceQueryFuture<'a, QuoteBalanceView> {
        Box::pin(async move {
            let wallet = self.metadata.wallet(wallet_id).await.map_err(|error| {
                BalanceQueryError::new(BalanceFailureKind::NotFound, error.to_string())
            })?;
            if !visible(&wallet, owner_type, owner_id) {
                return Err(BalanceQueryError::new(
                    BalanceFailureKind::OwnerMismatch,
                    "wallet owner mismatch",
                ));
            }
            let mint = self.parser.parse(mint).map_err(|_| {
                BalanceQueryError::new(BalanceFailureKind::InvalidMint, "invalid quote mint")
            })?;
            let reader = self.runtime.reader(wallet_id).ok_or_else(|| {
                BalanceQueryError::new(BalanceFailureKind::Unavailable, "wallet is not loaded")
            })?;
            let balance = reader.quote_balance(mint).await.map_err(|error| {
                BalanceQueryError::new(
                    BalanceFailureKind::Backend,
                    crate::sanitize::redact_urls(&error.to_string()),
                )
            })?;
            Ok(QuoteBalanceView {
                wallet_id: wallet_id.into(),
                quote_mint: mint,
                balance,
            })
        })
    }

    fn native<'a>(
        &'a self,
        owner_type: &'a str,
        owner_id: &'a str,
    ) -> BalanceQueryFuture<'a, WalletBalancesSnapshot> {
        Box::pin(async move {
            let visible = self
                .metadata
                .wallets()
                .await
                .map_err(|error| {
                    BalanceQueryError::new(BalanceFailureKind::Unavailable, error.to_string())
                })?
                .into_iter()
                .filter(|wallet| visible(wallet, owner_type, owner_id))
                .collect::<Vec<_>>();
            let addresses = visible
                .iter()
                .map(|wallet| self.parser.parse(&wallet.pubkey))
                .collect::<anyhow::Result<Vec<_>>>()
                .map_err(|error| {
                    BalanceQueryError::new(BalanceFailureKind::InvalidMetadata, error.to_string())
                })?;
            let balances = self
                .runtime
                .default_reader()
                .wallet_balances(&addresses)
                .await
                .and_then(|values| {
                    anyhow::ensure!(
                        values.len() == addresses.len(),
                        "wallet balances unavailable: RPC account count mismatch"
                    );
                    Ok(values)
                });
            let wallets = match balances {
                Ok(values) => visible
                    .into_iter()
                    .zip(values)
                    .map(|(wallet, balance)| WalletBalanceRow {
                        wallet_id: wallet.wallet_id,
                        pubkey: wallet.pubkey,
                        balance_lamports: Some(balance),
                        error: None,
                    })
                    .collect(),
                Err(error) => {
                    let message = error.to_string();
                    visible
                        .into_iter()
                        .map(|wallet| WalletBalanceRow {
                            wallet_id: wallet.wallet_id,
                            pubkey: wallet.pubkey,
                            balance_lamports: None,
                            error: Some(message.clone()),
                        })
                        .collect()
                }
            };
            Ok(WalletBalancesSnapshot {
                fetched_at_ms: (self.clock)(),
                wallets,
            })
        })
    }
}
