use super::{SdkTradeClient, SignedWsolTransaction};
use anyhow::Result;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{pubkey::Pubkey, signature::Signature, transaction::Transaction};
use std::sync::Arc;

/// The original selected RPC, without general-purpose request access for its owner.
pub(crate) struct WsolBroadcastLane {
    rpc: Arc<RpcClient>,
}

/// Borrows the unchanged artifact after policy, wallet and snapshot validation.
pub(crate) struct WsolBroadcastPacket<'a> {
    transaction: &'a Transaction,
}

impl WsolBroadcastLane {
    pub(crate) fn from_selected(rpc: Arc<RpcClient>) -> Self {
        Self { rpc }
    }

    pub(crate) fn url(&self) -> String {
        self.rpc.url()
    }

    pub(crate) fn commitment(&self) -> CommitmentConfig {
        self.rpc.commitment()
    }

    async fn send(&self, packet: WsolBroadcastPacket<'_>) -> Result<Signature> {
        // Keep RpcClient's original WSOL preflight, receipt check and transport behavior.
        Ok(self.rpc.send_transaction(packet.transaction).await?)
    }
}

impl<'a> WsolBroadcastPacket<'a> {
    fn checked(signed: &'a SignedWsolTransaction, wallet: &Pubkey) -> Result<Self> {
        signed.validate_snapshot(wallet)?;
        Ok(Self {
            transaction: &signed.transaction,
        })
    }
}

impl SdkTradeClient {
    pub(crate) async fn broadcast_signed_wsol(
        &self,
        signed: &SignedWsolTransaction,
        lane: &WsolBroadcastLane,
    ) -> Result<Signature> {
        self.ensure_broadcast_allowed()?;
        let packet = WsolBroadcastPacket::checked(signed, &self.wallet())?;
        lane.send(packet).await
    }
}

#[cfg(test)]
pub(crate) mod tests;
