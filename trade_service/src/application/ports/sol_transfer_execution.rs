use std::future::Future;

use crate::{
    application::ports::sol_transfer_journal::SolTransferJournal,
    domain::{
        sol_transfer::{SolTransferAmount, SolTransferSnapshot},
        submission::SignedSubmission,
        trade::Address,
    },
};

pub(crate) struct TransferWalletMetadata {
    pub owner_type: String,
    pub owner_id: String,
    pub pubkey: String,
    pub enabled: bool,
}

pub(crate) trait SolTransferArtifact: Send + Sync {
    fn submission(&self) -> &SignedSubmission;
    fn snapshot(&self) -> &SolTransferSnapshot;
}

pub(crate) trait SolTransferExecutionWallet: Send + Sync {
    type Prepared: Send;
    type Signed: SolTransferArtifact;
    fn pubkey(&self) -> Address;
    fn prepare(
        &self,
        destination: Address,
        amount: SolTransferAmount,
    ) -> impl Future<Output = anyhow::Result<Self::Prepared>> + Send;
    fn prepared_snapshot<'a>(&self, prepared: &'a Self::Prepared) -> &'a SolTransferSnapshot;
    fn sign(&self, prepared: Self::Prepared) -> anyhow::Result<Self::Signed>;
    fn broadcast(
        &self,
        signed: &Self::Signed,
    ) -> impl Future<Output = anyhow::Result<String>> + Send;
    fn confirm(&self, signed: &Self::Signed) -> impl Future<Output = bool> + Send;
}

pub(crate) trait SolTransferRuntime: Send + Sync {
    type Wallet: SolTransferExecutionWallet;
    type Journal: SolTransferJournal + Send;
    fn metadata(
        &self,
        wallet_id: &str,
    ) -> impl Future<Output = anyhow::Result<TransferWalletMetadata>> + Send;
    fn journal(&self) -> Option<Self::Journal>;
    fn resolve(&self, wallet_id: &str) -> Option<Self::Wallet>;
}
