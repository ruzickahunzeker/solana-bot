use anyhow::{bail, Result};
#[cfg(test)]
use solana_client::nonblocking::rpc_client::RpcClient;
#[cfg(test)]
use std::{sync::Arc, time::Duration};

#[cfg(test)]
use super::signature_status::RpcSignatureStatusReader;
use super::{read_http::ReadHttpTransport, read_runtime::SharedReadRuntime};
#[cfg(test)]
use crate::application::ports::reconciliation::SignatureStatusReader;
use crate::domain::reconciliation::ChainObservation;

/// Observes only the captured original signature; no SDK client or send capability.
pub(crate) async fn observe_sdk_signature_shared<T: ReadHttpTransport>(
    runtime: &SharedReadRuntime<T>,
    url: &str,
    signature: &str,
    budget_ms: u64,
) -> Result<Option<ChainObservation>> {
    use std::time::{Duration, Instant};
    if budget_ms == 0 {
        return Ok(None);
    }
    let urls = [url.to_string()];
    let budget = Duration::from_millis(budget_ms);
    let started = Instant::now();
    let result = tokio::time::timeout(budget, async {
        loop {
            if started.elapsed() >= budget {
                return Ok(None);
            }
            if let Some(observation) = runtime.quote_signature_status(&urls, signature).await? {
                if observation.slot == 0 {
                    bail!("SDK signature status RPC returned a zero transaction slot");
                }
                return Ok(Some(observation));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    // Tokio can poll a ready response after a blocked executor has exceeded this caller's budget.
    if started.elapsed() >= budget {
        return Ok(None);
    }
    match result {
        Ok(result) => result,
        Err(_) => Ok(None),
    }
}

/// Only the original signature and selected RPC are queried; timeout is not evidence of failure.
#[cfg(test)]
pub(crate) async fn observe_sdk_signature(
    rpc: Arc<RpcClient>,
    signature: &str,
    budget_ms: u64,
) -> Result<Option<ChainObservation>> {
    if budget_ms == 0 {
        return Ok(None);
    }
    let reader = RpcSignatureStatusReader { rpcs: vec![rpc] };
    match tokio::time::timeout(Duration::from_millis(budget_ms), async {
        loop {
            if let Some(observation) = reader.lookup_signature(signature).await? {
                if observation.slot == 0 {
                    bail!("SDK signature status RPC returned a zero transaction slot");
                }
                return Ok(Some(observation));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    {
        Ok(result) => result,
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests;
