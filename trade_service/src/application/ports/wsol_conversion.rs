use std::future::Future;

use crate::{
    application::wsol_submission_types::{
        WsolQuoteSnapshot, WsolSignedOperation, WsolSubmissionInput, WsolSubmissionRecord,
    },
    domain::{trade::Address, wsol_progress::WsolBroadcastOutcome},
};

pub(crate) use crate::domain::wallet::WalletMetadata as WsolWalletMetadata;

pub(crate) trait WsolArtifact: Send + Sync {
    fn signature(&self) -> &str;
    fn signed_bytes(&self) -> &[u8];
}

pub(crate) trait WsolExecutionWallet: Send + Sync {
    type Prepared: Send + Sync;
    type Signed: WsolArtifact;
    fn pubkey(&self) -> Address;
    fn prepare(
        &self,
        quote: &WsolQuoteSnapshot,
    ) -> impl Future<Output = anyhow::Result<Self::Prepared>> + Send;
    fn simulate_unsigned(
        &self,
        prepared: &Self::Prepared,
        quote: &WsolQuoteSnapshot,
    ) -> impl Future<Output = anyhow::Result<Option<u64>>> + Send;
    fn sign(
        &self,
        prepared: Self::Prepared,
        quote: &WsolQuoteSnapshot,
    ) -> anyhow::Result<Self::Signed>;
    fn simulate_signed(
        &self,
        signed: &Self::Signed,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
    fn broadcast(
        &self,
        signed: &Self::Signed,
    ) -> impl Future<Output = anyhow::Result<String>> + Send;
}

pub(crate) trait WsolRuntime: Send + Sync {
    type Wallet: WsolExecutionWallet;
    fn metadata(
        &self,
        wallet_id: &str,
    ) -> impl Future<Output = anyhow::Result<WsolWalletMetadata>> + Send;
    fn resolve(&self, wallet_id: &str) -> Option<Self::Wallet>;
}

pub(crate) trait WsolJournal: Send + Sync {
    fn quote(
        &self,
        quote_id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WsolQuoteSnapshot>>> + Send;
    fn reserve(
        &self,
        input: &WsolSubmissionInput,
        quote: &WsolQuoteSnapshot,
        now: u64,
    ) -> impl Future<Output = anyhow::Result<(WsolSubmissionRecord, bool)>> + Send;
    fn record_signed(
        &self,
        operation: WsolSignedOperation<'_>,
        now: u64,
    ) -> impl Future<Output = anyhow::Result<WsolSubmissionRecord>> + Send;
    fn record_simulation_failure(
        &self,
        operation: WsolSignedOperation<'_>,
        error: &str,
        now: u64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn fail_before_sign(
        &self,
        expected: &WsolSubmissionRecord,
        error: &str,
        now: u64,
    ) -> impl Future<Output = anyhow::Result<WsolSubmissionRecord>> + Send;
    fn record_broadcast(
        &self,
        operation: WsolSignedOperation<'_>,
        status: WsolBroadcastOutcome,
        error: Option<&str>,
        now: u64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn by_id(
        &self,
        id: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WsolSubmissionRecord>>> + Send;
    fn by_key(
        &self,
        key: &str,
    ) -> impl Future<Output = anyhow::Result<Option<WsolSubmissionRecord>>> + Send;
}
