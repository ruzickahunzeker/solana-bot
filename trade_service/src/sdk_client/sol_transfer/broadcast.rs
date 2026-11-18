use super::SignedSolTransfer;
use anyhow::{Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use std::{sync::Arc, time::Duration};

/// Retains the original selected RPC without exposing its general-purpose handle.
pub(super) struct SolTransferBroadcastLane {
    rpc: Arc<RpcClient>,
}

impl SolTransferBroadcastLane {
    pub(super) fn from_selected(rpc: Arc<RpcClient>) -> Self {
        Self { rpc }
    }

    pub(super) fn url(&self) -> String {
        self.rpc.url()
    }

    pub(super) fn commitment(&self) -> CommitmentConfig {
        self.rpc.commitment()
    }

    #[cfg(test)]
    pub(super) fn is_selected(&self, rpc: &Arc<RpcClient>) -> bool {
        Arc::ptr_eq(&self.rpc, rpc)
    }

    #[cfg(test)]
    pub(super) async fn confirm_original(&self, signature: &Signature, timeout: Duration) -> bool {
        matches!(
            tokio::time::timeout(timeout, self.rpc.confirm_transaction(signature)).await,
            Ok(Ok(true))
        )
    }
}

pub(super) async fn broadcast_signed(
    signed: &SignedSolTransfer,
    wallet: &Pubkey,
    timeout: Duration,
) -> Result<Signature> {
    signed.ensure_source(wallet)?;
    let snapshot = signed.snapshot();
    let balance = snapshot.balance_before_lamports;
    let amount = snapshot.amount_lamports;
    let fee = snapshot.fee_lamports;
    let reserve = snapshot.reserve_lamports;
    // The F2-C application compares this receipt with submission().signature after durable recording.
    tokio::time::timeout(timeout, signed.prepared.broadcast.rpc.send_transaction(&signed.transaction))
        .await.context("SOL transfer submission timed out")?.with_context(|| format!(
            "SOL transfer submission failed: balance={balance}, amount={amount}, fee={fee}, reserve={reserve} lamports",
        ))
}
