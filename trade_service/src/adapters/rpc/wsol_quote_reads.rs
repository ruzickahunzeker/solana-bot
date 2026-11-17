use super::{
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use anyhow::{ensure, Result};
use base64::Engine;
use serde_json::{json, Value};
use solana_client::rpc_response::Response;
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::{account::Account, hash::Hash, message::Message, pubkey::Pubkey};
use std::time::{Duration, Instant};
use tokio::sync::OwnedSemaphorePermit;

#[derive(Clone)]
enum Kind {
    Balance(Pubkey, CommitmentConfig),
    Ata(Pubkey),
    Rent,
    Hash(CommitmentConfig),
    Fee(String, CommitmentConfig),
}
#[derive(Clone)]
pub(crate) struct WsolQuoteQuery(Kind);
impl WsolQuoteQuery {
    pub(super) fn body(&self, id: u64) -> Value {
        let (method, params) = match &self.0 {
            Kind::Balance(owner, commitment) => {
                ("getBalance", json!([owner.to_string(), commitment]))
            }
            Kind::Ata(ata) => (
                "getAccountInfo",
                json!([ata.to_string(),{"encoding":"base64","dataSlice":null,"commitment":"processed","minContextSlot":null}]),
            ),
            Kind::Rent => (
                "getMinimumBalanceForRentExemption",
                json!([spl_token_interface::state::Account::LEN]),
            ),
            Kind::Hash(commitment) => ("getLatestBlockhash", json!([commitment])),
            Kind::Fee(message, commitment) => ("getFeeForMessage", json!([message, commitment])),
        };
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
    }
}
#[derive(Clone, Copy)]
enum Stage {
    New,
    Balance,
    Ata,
    Rent,
    Hash(Hash),
    Closed,
}

pub(crate) struct WsolQuoteReadSession<'a, T = ReqwestReadTransport> {
    runtime: &'a SharedReadRuntime<T>,
    url: String,
    owner: Pubkey,
    ata: Pubkey,
    commitment: CommitmentConfig,
    deadline: Instant,
    stage: Stage,
    _permit: OwnedSemaphorePermit,
}
impl<'a, T: ReadHttpTransport> WsolQuoteReadSession<'a, T> {
    pub(crate) fn new(
        runtime: &'a SharedReadRuntime<T>,
        url: &str,
        commitment: CommitmentConfig,
        owner: Pubkey,
    ) -> Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let permit = runtime.wsol_quote_slot(url)?;
        let mint = crate::domain::trade::WSOL_MINT.parse::<Pubkey>()?;
        let ata=spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&owner,&mint,&spl_token_interface::ID);
        Ok(Self {
            runtime,
            url: url.into(),
            owner,
            ata,
            commitment,
            deadline,
            stage: Stage::New,
            _permit: permit,
        })
    }
    async fn read(&self, kind: Kind) -> Result<Value> {
        self.runtime
            .wsol_quote_step(&self.url, WsolQuoteQuery(kind), self.deadline)
            .await
    }
    fn contextual<TValue: serde::de::DeserializeOwned>(&self, value: Value) -> Result<TValue> {
        let response: Response<TValue> = serde_json::from_value(value)
            .map_err(|_| anyhow::anyhow!("invalid WSOL quote response"))?;
        ensure!(
            response.context.slot > 0 && Instant::now() < self.deadline,
            "invalid or expired WSOL quote context"
        );
        Ok(response.value)
    }
    pub(crate) async fn balance(&mut self) -> Result<u64> {
        ensure!(
            matches!(self.stage, Stage::New),
            "WSOL balance already attempted or out of order"
        );
        self.stage = Stage::Closed;
        let value = self
            .read(Kind::Balance(self.owner, self.commitment))
            .await?;
        let balance = self.contextual(value)?;
        self.stage = Stage::Balance;
        Ok(balance)
    }
    pub(crate) async fn ata(&mut self) -> Result<Option<Account>> {
        ensure!(
            matches!(self.stage, Stage::Balance),
            "WSOL ATA already attempted or out of order"
        );
        self.stage = Stage::Closed;
        let value = self.read(Kind::Ata(self.ata)).await?;
        let account: Option<solana_account_decoder::UiAccount> = self.contextual(value)?;
        let account = account
            .map(super::read_runtime::aggregator_accounts::checked_account)
            .transpose()?;
        ensure!(
            Instant::now() < self.deadline,
            "WSOL account scope expired after decoding"
        );
        self.stage = Stage::Ata;
        Ok(account)
    }
    pub(crate) async fn rent(&mut self) -> Result<u64> {
        ensure!(
            matches!(self.stage, Stage::Ata),
            "WSOL rent already attempted or out of order"
        );
        self.stage = Stage::Closed;
        let value = self.read(Kind::Rent).await?;
        let rent = value
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("invalid WSOL quote rent"))?;
        self.stage = Stage::Rent;
        Ok(rent)
    }
    pub(crate) async fn blockhash(&mut self) -> Result<Hash> {
        ensure!(
            matches!(self.stage, Stage::Rent),
            "WSOL blockhash already attempted or out of order"
        );
        self.stage = Stage::Closed;
        let value = self.read(Kind::Hash(self.commitment)).await?;
        let value: BlockhashValue = self.contextual(value)?;
        ensure!(value.blockhash.len() <= 44, "invalid WSOL quote blockhash");
        let hash = value
            .blockhash
            .parse::<Hash>()
            .map_err(|_| anyhow::anyhow!("invalid WSOL quote blockhash"))?;
        self.stage = Stage::Hash(hash);
        Ok(hash)
    }
    pub(crate) async fn fee(&mut self, message: &Message) -> Result<u64> {
        let Stage::Hash(hash) = self.stage else {
            anyhow::bail!("WSOL fee requires its original blockhash")
        };
        self.stage = Stage::Closed;
        ensure!(
            message.account_keys.first() == Some(&self.owner)
                && message.recent_blockhash == hash
                && (1..=2).contains(&message.header.num_required_signatures)
                && message.account_keys.contains(&self.ata),
            "WSOL quote fee message binding is invalid"
        );
        ensure!(
            bincode::serialized_size(message)? <= 1232,
            "WSOL fee message exceeds limit"
        );
        let bytes = message.serialize();
        let value = self
            .read(Kind::Fee(
                base64::engine::general_purpose::STANDARD.encode(bytes),
                self.commitment,
            ))
            .await?;
        let fee: Option<u64> = self.contextual(value)?;
        fee.ok_or_else(|| anyhow::anyhow!("WSOL quote fee is unavailable"))
    }
}
#[derive(serde::Deserialize)]
struct BlockhashValue {
    blockhash: String,
    #[serde(rename = "lastValidBlockHeight")]
    _last_valid_block_height: u64,
}

#[cfg(test)]
pub(crate) mod tests;
