use super::{
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use anyhow::{ensure, Result};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use solana_client::rpc_response::Response;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{hash::Hash, message::Message, pubkey::Pubkey};
use std::time::{Duration, Instant};
use tokio::sync::OwnedSemaphorePermit;

#[derive(Clone)]
enum Kind {
    Balance(Pubkey, CommitmentConfig),
    Hash(CommitmentConfig),
    Fee(String, CommitmentConfig),
    Rent,
}

#[derive(Clone)]
pub(crate) struct SolTransferQuery(Kind);
impl SolTransferQuery {
    pub(super) fn body(&self, id: u64) -> Value {
        let (method, params) = match &self.0 {
            Kind::Balance(owner, commitment) => {
                ("getBalance", json!([owner.to_string(), commitment]))
            }
            Kind::Hash(commitment) => ("getLatestBlockhash", json!([commitment])),
            Kind::Fee(message, commitment) => ("getFeeForMessage", json!([message, commitment])),
            Kind::Rent => ("getMinimumBalanceForRentExemption", json!([0])),
        };
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
    }
}

#[derive(Clone, Copy)]
enum Stage {
    New,
    Balance,
    Hash(Hash),
    Fee,
    Closed,
}

/// One original lane's five-second preparation scope, without a signer or send RPC.
pub(crate) struct SolTransferReadSession<'a, T = ReqwestReadTransport> {
    runtime: &'a SharedReadRuntime<T>,
    url: String,
    source: Pubkey,
    destination: Pubkey,
    commitment: CommitmentConfig,
    deadline: Instant,
    stage: Stage,
    _permit: OwnedSemaphorePermit,
}

impl<'a, T: ReadHttpTransport> SolTransferReadSession<'a, T> {
    pub(crate) fn new(
        runtime: &'a SharedReadRuntime<T>,
        url: &str,
        commitment: CommitmentConfig,
        source: Pubkey,
        destination: Pubkey,
    ) -> Result<Self> {
        ensure!(
            source != destination,
            "source and destination wallets must be different"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let permit = runtime.sol_transfer_slot(url)?;
        Ok(Self {
            runtime,
            url: url.into(),
            source,
            destination,
            commitment,
            deadline,
            stage: Stage::New,
            _permit: permit,
        })
    }

    async fn read(&self, kind: Kind) -> Result<Value> {
        self.runtime
            .sol_transfer_step(&self.url, SolTransferQuery(kind), self.deadline)
            .await
    }

    fn contextual<V: serde::de::DeserializeOwned>(&self, value: Value) -> Result<V> {
        let response: Response<V> = serde_json::from_value(value)
            .map_err(|_| anyhow::anyhow!("invalid SOL transfer preparation response"))?;
        ensure!(
            Instant::now() < self.deadline,
            "SOL transfer preparation expired after decoding"
        );
        Ok(response.value)
    }

    async fn balance(&mut self) -> Result<u64> {
        ensure!(
            matches!(self.stage, Stage::New),
            "SOL transfer balance already attempted or out of order"
        );
        self.stage = Stage::Closed;
        let value = self
            .read(Kind::Balance(self.source, self.commitment))
            .await?;
        let balance = self.contextual(value)?;
        self.stage = Stage::Balance;
        Ok(balance)
    }

    async fn blockhash(&mut self) -> Result<Hash> {
        ensure!(
            matches!(self.stage, Stage::Balance),
            "SOL transfer blockhash already attempted or out of order"
        );
        self.stage = Stage::Closed;
        let value = self.read(Kind::Hash(self.commitment)).await?;
        let block: BlockhashValue = self.contextual(value)?;
        ensure!(
            block.blockhash.len() <= 44,
            "invalid SOL transfer blockhash"
        );
        let hash = block
            .blockhash
            .parse::<Hash>()
            .map_err(|_| anyhow::anyhow!("invalid SOL transfer blockhash"))?;
        self.stage = Stage::Hash(hash);
        Ok(hash)
    }

    async fn fee(&mut self) -> Result<u64> {
        let Stage::Hash(hash) = self.stage else {
            anyhow::bail!("SOL transfer fee requires its original blockhash");
        };
        self.stage = Stage::Closed;
        // Same original one-lamport estimate, never caller-provided instructions or wire bytes.
        let message = Message::new_with_blockhash(
            &[solana_system_interface::instruction::transfer(
                &self.source,
                &self.destination,
                1,
            )],
            Some(&self.source),
            &hash,
        );
        let value = self
            .read(Kind::Fee(
                base64::engine::general_purpose::STANDARD.encode(message.serialize()),
                self.commitment,
            ))
            .await?;
        let fee: Option<u64> = self.contextual(value)?;
        let fee = fee.ok_or_else(|| anyhow::anyhow!("SOL transfer fee unavailable"))?;
        self.stage = Stage::Fee;
        Ok(fee)
    }

    async fn rent(&mut self) -> Result<u64> {
        ensure!(
            matches!(self.stage, Stage::Fee),
            "SOL transfer rent requires its original fee"
        );
        self.stage = Stage::Closed;
        let value = self.read(Kind::Rent).await?;
        let rent = value
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("invalid SOL transfer rent"))?;
        ensure!(
            Instant::now() < self.deadline,
            "SOL transfer preparation expired after rent decoding"
        );
        Ok(rent)
    }

    pub(crate) async fn snapshot(mut self) -> Result<(u64, Hash, u64, u64)> {
        let balance = self.balance().await?;
        let hash = self.blockhash().await?;
        let fee = self.fee().await?;
        let rent = self.rent().await?;
        Ok((balance, hash, fee, rent))
    }
}

#[derive(Deserialize)]
struct BlockhashValue {
    blockhash: String,
    #[serde(rename = "lastValidBlockHeight")]
    _height: u64,
}

#[cfg(test)]
mod tests;
