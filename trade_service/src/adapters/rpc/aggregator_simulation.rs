use super::{
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use anyhow::{ensure, Result};
use base64::Engine;
use serde_json::{json, Value};
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};
use solana_sdk::{
    account::Account, message::VersionedMessage, pubkey::Pubkey, signature::Signature,
    transaction::VersionedTransaction,
};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};
use tokio::sync::OwnedSemaphorePermit;

#[derive(Clone)]
pub(crate) struct UnsignedAggregatorSimulation {
    packet: String,
    accounts: [String; 2],
    loaded_writable: Vec<String>,
    loaded_readonly: Vec<String>,
}
impl UnsignedAggregatorSimulation {
    pub(crate) fn new(
        transaction: &VersionedTransaction,
        wallet: Pubkey,
        target: Pubkey,
        target_index: u8,
        resolved_keys: &[Pubkey],
    ) -> Result<Self> {
        transaction
            .sanitize()
            .map_err(|_| anyhow::anyhow!("invalid aggregator simulation transaction"))?;
        ensure!(
            transaction.signatures == [Signature::default()]
                && transaction.message.header().num_required_signatures == 1
                && transaction.message.static_account_keys().first() == Some(&wallet)
                && matches!(
                    transaction.message,
                    VersionedMessage::Legacy(_) | VersionedMessage::V0(_)
                ),
            "aggregator simulation requires a single-payer zero-signature candidate"
        );
        let bytes = bincode::serialize(transaction)?;
        ensure!(
            bytes.len() <= 1232,
            "aggregator simulation transaction exceeds packet size"
        );
        let static_keys = transaction.message.static_account_keys();
        let lookups = transaction.message.address_table_lookups().unwrap_or(&[]);
        ensure!(
            lookups.len() <= 4 && (1..=32).contains(&transaction.message.instructions().len()),
            "aggregator simulation instruction or lookup count is invalid"
        );
        let writable = lookups
            .iter()
            .map(|lookup| lookup.writable_indexes.len())
            .sum::<usize>();
        let readonly = lookups
            .iter()
            .map(|lookup| lookup.readonly_indexes.len())
            .sum::<usize>();
        ensure!(
            resolved_keys.len() <= 256
                && resolved_keys.len() == static_keys.len() + writable + readonly
                && resolved_keys.starts_with(static_keys)
                && resolved_keys.iter().copied().collect::<HashSet<_>>().len()
                    == resolved_keys.len()
                && resolved_keys.get(usize::from(target_index)) == Some(&target)
                && transaction
                    .message
                    .is_maybe_writable_with_reserved_addresses(
                        usize::from(target_index),
                        None::<&HashSet<Pubkey>>
                    ),
            "aggregator simulation account binding is invalid"
        );
        Ok(Self {
            packet: base64::engine::general_purpose::STANDARD.encode(bytes),
            accounts: [wallet.to_string(), target.to_string()],
            loaded_writable: resolved_keys[static_keys.len()..static_keys.len() + writable]
                .iter()
                .map(ToString::to_string)
                .collect(),
            loaded_readonly: resolved_keys[static_keys.len() + writable..]
                .iter()
                .map(ToString::to_string)
                .collect(),
        })
    }

    pub(super) fn baseline_body(&self, id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"getMultipleAccounts","params":[self.accounts,{"encoding":"base64","commitment":"processed"}]})
    }
    pub(super) fn simulation_body(&self, id: u64, minimum_slot: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"simulateTransaction","params":[self.packet,{"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64","accounts":{"encoding":"base64","addresses":self.accounts},"minContextSlot":minimum_slot,"innerInstructions":true}]})
    }
    pub(super) fn check_simulation(
        &self,
        response: &Response<RpcSimulateTransactionResult>,
        minimum_slot: u64,
    ) -> Result<()> {
        ensure!(
            response.context.slot >= minimum_slot && minimum_slot > 0,
            "aggregator simulation context precedes its baseline"
        );
        ensure!(
            response.value.replacement_blockhash.is_none(),
            "aggregator simulation changed its blockhash"
        );
        if let Some(loaded) = &response.value.loaded_addresses {
            ensure!(
                loaded.writable == self.loaded_writable && loaded.readonly == self.loaded_readonly,
                "aggregator simulation loaded accounts differ from its message"
            );
        }
        if let Some(accounts) = &response.value.accounts {
            ensure!(
                accounts.len() == 2,
                "aggregator simulation account count is invalid"
            );
            for account in accounts.iter().flatten() {
                super::read_runtime::aggregator_accounts::checked_account(account.clone())?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Stage {
    New,
    Baseline(u64),
    Closed,
}

/// One fixed baseline/simulation sequence. No signer, execution RPC or detached task.
pub(crate) struct AggregatorSimulationReadSession<'a, T = ReqwestReadTransport> {
    runtime: &'a SharedReadRuntime<T>,
    url: String,
    plan: UnsignedAggregatorSimulation,
    deadline: Instant,
    stage: Stage,
    _permit: OwnedSemaphorePermit,
}
impl<'a, T: ReadHttpTransport> AggregatorSimulationReadSession<'a, T> {
    pub(crate) fn new(
        runtime: &'a SharedReadRuntime<T>,
        url: &str,
        plan: UnsignedAggregatorSimulation,
    ) -> Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let permit = runtime.aggregator_simulation_slot(url)?;
        Ok(Self {
            runtime,
            url: url.to_owned(),
            plan,
            deadline,
            stage: Stage::New,
            _permit: permit,
        })
    }
    pub(crate) async fn baseline(&mut self) -> Result<Response<Vec<Option<Account>>>> {
        ensure!(
            matches!(self.stage, Stage::New),
            "aggregator simulation baseline was already attempted"
        );
        self.stage = Stage::Closed;
        let value = self
            .runtime
            .aggregator_simulation_step(&self.url, &self.plan, None, self.deadline)
            .await?;
        let baseline: Response<Vec<Option<solana_account_decoder::UiAccount>>> =
            serde_json::from_value(value)
                .map_err(|_| anyhow::anyhow!("invalid aggregator baseline response"))?;
        ensure!(
            baseline.context.slot > 0 && baseline.value.len() == 2,
            "aggregator baseline context or count is invalid"
        );
        let accounts = baseline
            .value
            .into_iter()
            .map(|account| {
                account
                    .map(super::read_runtime::aggregator_accounts::checked_account)
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            Instant::now() < self.deadline,
            "aggregator baseline scope expired after decoding"
        );
        self.stage = Stage::Baseline(baseline.context.slot);
        Ok(Response {
            context: baseline.context,
            value: accounts,
        })
    }
    pub(crate) async fn simulate(&mut self) -> Result<Response<RpcSimulateTransactionResult>> {
        let Stage::Baseline(slot) = self.stage else {
            anyhow::bail!("aggregator simulation requires its successful baseline")
        };
        self.stage = Stage::Closed;
        let value = self
            .runtime
            .aggregator_simulation_step(&self.url, &self.plan, Some(slot), self.deadline)
            .await?;
        let response: Response<RpcSimulateTransactionResult> = serde_json::from_value(value)
            .map_err(|_| anyhow::anyhow!("invalid aggregator simulation response"))?;
        self.plan.check_simulation(&response, slot)?;
        ensure!(
            Instant::now() < self.deadline,
            "aggregator simulation scope expired after decoding"
        );
        Ok(response)
    }
}

#[cfg(test)]
pub(crate) mod tests;
