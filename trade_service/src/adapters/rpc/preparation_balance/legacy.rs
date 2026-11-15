use crate::{
    adapters::sdk::preparation_assets,
    application::ports::preparation_balance::PreparationBalanceRead,
    domain::{
        preparation_balance::{
            reject_balance, BalanceFailureKind, PreparationBalanceFailure,
            PreparationBalanceSnapshot, PreparationInput,
        },
        trade::Address,
    },
};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    nonblocking::rpc_client::RpcClient,
    rpc_config::RpcAccountInfoConfig,
    rpc_request::{RpcError, TokenAccountsFilter},
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) struct RpcPreparationBalanceRead {
    wallet: Address,
    rpc: Arc<RpcClient>,
}
impl RpcPreparationBalanceRead {
    pub(crate) fn new(wallet: Address, rpc: Arc<RpcClient>) -> Self {
        Self { wallet, rpc }
    }
}
impl PreparationBalanceRead for RpcPreparationBalanceRead {
    async fn snapshot(&self, mint: Address) -> anyhow::Result<PreparationBalanceSnapshot> {
        let started = Instant::now();
        tokio::time::timeout(Duration::from_secs(4), self.read(mint, started))
            .await
            .map_err(|_| {
                PreparationBalanceFailure::new(
                    BalanceFailureKind::Unavailable,
                    "preparation balance read timed out",
                )
            })?
    }
}

impl RpcPreparationBalanceRead {
    async fn read(
        &self,
        mint: Address,
        started: Instant,
    ) -> anyhow::Result<PreparationBalanceSnapshot> {
        let scan_response = self
            .rpc
            .get_token_accounts_by_owner_with_commitment(
                &Pubkey::new_from_array(self.wallet.to_bytes()),
                TokenAccountsFilter::Mint(Pubkey::new_from_array(mint.to_bytes())),
                CommitmentConfig::processed(),
            )
            .await;
        let (scan, minimum_slot) = match scan_response {
            Ok(response) => {
                if response.context.slot == 0 {
                    return Err(reject_balance("preparation scan slot is invalid"));
                }
                if response.value.len() > 1000 {
                    return Err(reject_balance(
                        "preparation token scan exceeds account limit",
                    ));
                }
                let value = serde_json::to_value(response.value)
                    .map_err(|_| reject_balance("preparation scan metadata is invalid"))?;
                let scan = super::read_runtime::preparation_scan(value, self.wallet, mint)
                    .map_err(|_| {
                        reject_balance(
                            "preparation scan cannot verify wallet, mint, state or input account",
                        )
                    })?;
                (scan, Some(response.context.slot))
            }
            Err(error) => {
                let failure = classify_read(error);
                if !failure.allows_lane_fallback() {
                    return Err(failure.into());
                }
                (None, None)
            }
        };
        let response = self
            .rpc
            .get_multiple_ui_accounts_with_config(
                &preparation_assets::input_keys(self.wallet, mint),
                RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    commitment: Some(CommitmentConfig::processed()),
                    data_slice: None,
                    min_context_slot: minimum_slot,
                },
            )
            .await
            .map_err(classify_read)?;
        if response.context.slot == 0
            || minimum_slot.is_some_and(|slot| response.context.slot < slot)
        {
            return Err(reject_balance("preparation raw account slot is invalid"));
        }
        let raw =
            preparation_assets::inspect(self.wallet, mint, &response.value).map_err(|_| {
                reject_balance("preparation raw mint or input account evidence is invalid")
            })?;
        if scan.is_some_and(|scan| {
            scan.units != raw.units
                || scan.program.is_some_and(|program| program != raw.program)
                || scan
                    .decimals
                    .is_some_and(|decimals| decimals != raw.decimals)
        }) {
            return Err(reject_balance(
                "preparation scan and raw input evidence disagree",
            ));
        }
        PreparationBalanceSnapshot::from_account(
            self.wallet,
            mint,
            raw.units,
            PreparationInput {
                account: raw.account,
                program: raw.program,
                decimals: raw.decimals,
            },
            response.context.slot,
            started,
        )
    }
}

fn classify_read(error: ClientError) -> PreparationBalanceFailure {
    let unavailable = matches!(
        error.kind(),
        ClientErrorKind::Io(_)
            | ClientErrorKind::Reqwest(_)
            | ClientErrorKind::Custom(_)
            | ClientErrorKind::RpcError(
                RpcError::RpcRequestError(_) | RpcError::RpcResponseError { .. }
            )
    );
    PreparationBalanceFailure::new(
        if unavailable {
            BalanceFailureKind::Unavailable
        } else {
            BalanceFailureKind::Invalid
        },
        if unavailable {
            "preparation balance read unavailable"
        } else {
            "preparation balance response is invalid"
        },
    )
}

#[cfg(test)]
#[path = "legacy_tests.rs"]
mod tests;
