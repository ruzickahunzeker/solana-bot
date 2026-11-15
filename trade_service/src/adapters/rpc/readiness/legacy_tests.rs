use solana_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;

use crate::domain::readiness::ReadinessWalletMetadata;

pub(crate) struct RpcReadinessProbe {
    metadata: ReadinessWalletMetadata,
    rpcs: Vec<Arc<RpcClient>>,
}

impl RpcReadinessProbe {
    pub fn new(metadata: ReadinessWalletMetadata, rpcs: Vec<Arc<RpcClient>>) -> Self {
        Self { metadata, rpcs }
    }

    pub async fn ping(&self) -> bool {
        for rpc in &self.rpcs {
            if rpc.get_latest_blockhash().await.is_ok() {
                return true;
            }
        }
        false
    }

    pub fn into_metadata(self) -> ReadinessWalletMetadata {
        self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::trade::Address;
    use solana_client::rpc_request::RpcRequest;
    use std::collections::HashMap;

    fn metadata() -> ReadinessWalletMetadata {
        ReadinessWalletMetadata {
            wallet: Address::from_bytes([1; 32]),
            rpc_lanes: 3,
            swqos_lanes: vec!["Temporal".into(), "Default".into()],
            active_swqos_lanes: vec!["Temporal".into()],
            mev_protection: false,
            require_mev_submit: true,
        }
    }

    fn success() -> Arc<RpcClient> {
        let hash = solana_sdk::hash::Hash::new_from_array([7; 32]);
        Arc::new(RpcClient::new_mock_with_mocks(
            "fails".into(),
            HashMap::from([(
                RpcRequest::GetLatestBlockhash,
                serde_json::json!({"context":{"slot":42},"value":{"blockhash":hash.to_string(),"lastValidBlockHeight":99}}),
            )]),
        ))
    }

    #[tokio::test]
    async fn successful_probe_stops_before_later_lane_and_preserves_metadata() {
        let backup = success();
        let probe = RpcReadinessProbe::new(metadata(), vec![success(), backup.clone()]);
        assert!(probe.ping().await);
        assert!(backup.get_latest_blockhash().await.is_ok());
        let metadata = probe.into_metadata();
        assert_eq!(metadata.wallet.to_bytes(), [1; 32]);
        assert_eq!(metadata.rpc_lanes, 3);
        assert_eq!(metadata.active_swqos_lanes, vec!["Temporal"]);
        assert_eq!(metadata.swqos_lanes, vec!["Temporal", "Default"]);
        assert!(!metadata.mev_protection);
        assert!(metadata.require_mev_submit);
    }

    #[tokio::test]
    async fn failed_probe_uses_next_supplied_execution_lane() {
        let probe = RpcReadinessProbe::new(
            metadata(),
            vec![Arc::new(RpcClient::new_mock("fails".into())), success()],
        );
        assert!(probe.ping().await);
    }

    #[tokio::test]
    async fn empty_and_failed_probes_return_false_without_a_wallet_or_signer() {
        for rpcs in [vec![], vec![Arc::new(RpcClient::new_mock("fails".into()))]] {
            assert!(!RpcReadinessProbe::new(metadata(), rpcs).ping().await);
        }
    }
}
