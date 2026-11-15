use anyhow::{bail, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::{
    application::ports::blockhash::LatestBlockhashReader, domain::blockhash::Blockhash,
    sanitize::redact_urls,
};

pub(crate) struct RpcBlockhashReader {
    rpcs: Vec<Arc<RpcClient>>,
}

impl RpcBlockhashReader {
    pub fn new(rpcs: Vec<Arc<RpcClient>>) -> Self {
        Self { rpcs }
    }
}

impl LatestBlockhashReader for RpcBlockhashReader {
    async fn latest_blockhash(&self) -> Result<Blockhash> {
        for (idx, rpc) in self.rpcs.iter().enumerate() {
            match rpc.get_latest_blockhash().await {
                Ok(hash) => {
                    debug!(rpc_lane = idx, "blockhash refreshed");
                    return Ok(Blockhash::from_bytes(hash.to_bytes()));
                }
                Err(error) => {
                    let error = redact_urls(&error.to_string());
                    warn!(rpc_lane=idx,error=%error,"blockhash refresh failed");
                }
            }
        }
        bail!("all RPC lanes failed to refresh blockhash")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_client::rpc_request::RpcRequest;
    use std::collections::HashMap;

    fn success(byte: u8) -> Arc<RpcClient> {
        let hash = solana_sdk::hash::Hash::new_from_array([byte; 32]);
        let mocks = HashMap::from([(
            RpcRequest::GetLatestBlockhash,
            serde_json::json!({"context":{"slot":42},"value":{"blockhash":hash.to_string(),"lastValidBlockHeight":99}}),
        )]);
        Arc::new(RpcClient::new_mock_with_mocks("fails".into(), mocks))
    }

    #[tokio::test]
    async fn primary_success_stops_before_backup_and_returns_owned_bytes() {
        let backup = success(2);
        let reader = RpcBlockhashReader::new(vec![success(1), backup.clone()]);
        assert_eq!(reader.latest_blockhash().await.unwrap().to_bytes(), [1; 32]);
        // Mock entries are consumed once; backup still has its original response.
        assert_eq!(
            backup.get_latest_blockhash().await.unwrap().to_bytes(),
            [2; 32]
        );
    }

    #[tokio::test]
    async fn failed_primary_uses_backup_in_existing_order() {
        let reader = RpcBlockhashReader::new(vec![
            Arc::new(RpcClient::new_mock("fails".into())),
            success(3),
        ]);
        assert_eq!(reader.latest_blockhash().await.unwrap().to_bytes(), [3; 32]);
    }

    #[tokio::test]
    async fn empty_or_failed_lanes_do_not_invent_a_hash() {
        for rpcs in [vec![], vec![Arc::new(RpcClient::new_mock("fails".into()))]] {
            assert_eq!(
                RpcBlockhashReader::new(rpcs)
                    .latest_blockhash()
                    .await
                    .unwrap_err()
                    .to_string(),
                "all RPC lanes failed to refresh blockhash"
            );
        }
    }
}
