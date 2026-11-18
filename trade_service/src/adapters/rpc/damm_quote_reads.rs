use super::{
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use anyhow::{ensure, Result};
use base64::Engine;
use serde_json::{json, Value};
use solana_account_decoder::UiAccount;
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Signature, transaction::Transaction};
use std::{collections::HashSet, future::Future};

mod sealed {
    pub trait Sealed {}
}

/// Only the original fixed account snapshot and unsigned quote probe, never signing.
pub(crate) trait DammQuoteRpc: sealed::Sealed + Send + Sync {
    fn damm_accounts(
        &self,
        keys: &[Pubkey],
        minimum: Option<u64>,
    ) -> impl Future<Output = Result<Response<Vec<Option<UiAccount>>>>> + Send;

    fn damm_probe(
        &self,
        transaction: &Transaction,
        accounts: &[Pubkey],
        minimum: u64,
        inner: bool,
    ) -> impl Future<Output = Result<Response<RpcSimulateTransactionResult>>> + Send;
}

/// Borrowed service-owned runtime and original selected URL; no execution client.
pub(crate) struct DammQuoteReader<'a, T = ReqwestReadTransport> {
    runtime: &'a SharedReadRuntime<T>,
    url: String,
}
impl<'a, T: ReadHttpTransport> DammQuoteReader<'a, T> {
    pub(crate) fn new(runtime: &'a SharedReadRuntime<T>, url: String) -> Self {
        Self { runtime, url }
    }
}
impl<T: ReadHttpTransport> sealed::Sealed for DammQuoteReader<'_, T> {}
impl<T: ReadHttpTransport> DammQuoteRpc for DammQuoteReader<'_, T> {
    async fn damm_accounts(
        &self,
        keys: &[Pubkey],
        minimum: Option<u64>,
    ) -> Result<Response<Vec<Option<UiAccount>>>> {
        self.runtime
            .damm_quote_accounts(&self.url, DammQuoteAccounts::checked(keys, minimum)?)
            .await
    }

    async fn damm_probe(
        &self,
        transaction: &Transaction,
        accounts: &[Pubkey],
        minimum: u64,
        inner: bool,
    ) -> Result<Response<RpcSimulateTransactionResult>> {
        self.runtime
            .damm_quote_probe(
                &self.url,
                DammQuotePacket::checked(transaction, accounts, minimum, inner)?,
            )
            .await
    }
}

pub(crate) async fn simulate_packet(
    rpc: &impl DammQuoteRpc,
    transaction: &Transaction,
    accounts: &[Pubkey],
    minimum: u64,
    inner: bool,
) -> Result<Response<RpcSimulateTransactionResult>> {
    rpc.damm_probe(transaction, accounts, minimum, inner).await
}

fn account_keys(keys: &[Pubkey]) -> Result<Vec<String>> {
    ensure!(
        matches!(keys.len(), 8 | 9) && keys.iter().collect::<HashSet<_>>().len() == keys.len(),
        "DAMM quote requires its eight or nine distinct evidence accounts"
    );
    Ok(keys.iter().map(ToString::to_string).collect())
}

/// Fixed processed/base64 account query, not an arbitrary RPC/config envelope.
pub(crate) struct DammQuoteAccounts {
    keys: Vec<String>,
    minimum: Option<u64>,
}
impl DammQuoteAccounts {
    fn checked(keys: &[Pubkey], minimum: Option<u64>) -> Result<Self> {
        ensure!(
            minimum.is_none_or(|slot| slot > 0),
            "DAMM quote minimum account slot must be nonzero"
        );
        Ok(Self {
            keys: account_keys(keys)?,
            minimum,
        })
    }
    pub(super) fn body(&self, id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"getMultipleAccounts","params":[self.keys,
            {"encoding":"base64","commitment":"processed","dataSlice":null,"minContextSlot":self.minimum}]})
    }
}

/// Bounded zero-signature legacy probe; no encoded input constructor or Clone.
pub(crate) struct DammQuotePacket {
    encoded: String,
    accounts: Vec<String>,
    minimum: u64,
}
impl DammQuotePacket {
    fn checked(
        transaction: &Transaction,
        accounts: &[Pubkey],
        minimum: u64,
        inner: bool,
    ) -> Result<Self> {
        ensure!(
            minimum > 0 && inner,
            "DAMM quote probe requires its slot and inner transfers"
        );
        ensure!(
            transaction.signatures == [Signature::default()]
                && transaction.message.header.num_required_signatures == 1
                && transaction
                    .message
                    .account_keys
                    .first()
                    .is_some_and(|payer| *payer != Pubkey::default())
                && transaction.message.recent_blockhash != Hash::default()
                && bincode::serialized_size(transaction)? <= 1232,
            "DAMM quote probe requires a bounded single-payer zero-signature message"
        );
        Ok(Self {
            encoded: base64::engine::general_purpose::STANDARD
                .encode(bincode::serialize(transaction)?),
            accounts: account_keys(accounts)?,
            minimum,
        })
    }
    pub(super) fn body(&self, id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"simulateTransaction","params":[self.encoded,
            {"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64",
            "accounts":{"encoding":"base64","addresses":self.accounts},"minContextSlot":self.minimum,"innerInstructions":true}]})
    }
}

#[cfg(test)]
impl sealed::Sealed for solana_client::nonblocking::rpc_client::RpcClient {}
#[cfg(test)]
impl DammQuoteRpc for solana_client::nonblocking::rpc_client::RpcClient {
    async fn damm_accounts(
        &self,
        keys: &[Pubkey],
        minimum: Option<u64>,
    ) -> Result<Response<Vec<Option<UiAccount>>>> {
        self.get_multiple_ui_accounts_with_config(
            keys,
            solana_client::rpc_config::RpcAccountInfoConfig {
                encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                commitment: Some(solana_commitment_config::CommitmentConfig::processed()),
                data_slice: None,
                min_context_slot: minimum,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", crate::sanitize::redact_urls(&error.to_string())))
    }
    async fn damm_probe(
        &self,
        transaction: &Transaction,
        accounts: &[Pubkey],
        minimum: u64,
        inner: bool,
    ) -> Result<Response<RpcSimulateTransactionResult>> {
        crate::adapters::sdk::damm_probe::simulate_packet(
            self,
            transaction,
            accounts,
            minimum,
            inner,
        )
        .await
    }
}

#[cfg(test)]
pub(crate) mod tests;
