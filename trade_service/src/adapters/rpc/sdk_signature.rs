use anyhow::{bail, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use std::{sync::Arc, time::Duration};

use super::signature_status::RpcSignatureStatusReader;
use crate::{
    application::ports::reconciliation::SignatureStatusReader,
    domain::reconciliation::ChainObservation,
};

/// Only the original signature and selected RPC are queried; timeout is not evidence of failure.
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
