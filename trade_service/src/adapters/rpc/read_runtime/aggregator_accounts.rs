use super::*;
use crate::{
    adapters::rpc::parameter_reads::{invalid, unavailable},
    domain::trade::Address,
};
use base64::Engine;
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::account::Account;

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn aggregator_account(
        &self,
        url: &str,
        account: Address,
        commitment: CommitmentConfig,
    ) -> Result<Account> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let _slot = self.slots[Class::AggregatorAccounts.index()]
            .clone()
            .try_acquire_owned()
            .map_err(|_| unavailable("aggregator account RPC quota busy"))?;
        let index = *self
            .candidate_indices(&[url.to_owned()])
            .first()
            .ok_or_else(|| unavailable("aggregator account RPC endpoint is not registered"))?;
        ensure!(
            self.available(index, Instant::now())?,
            unavailable("aggregator account RPC endpoint cooling down or unavailable")
        );
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::AggregatorAccount {
            id,
            account,
            commitment,
        };
        let maximum = request.max_body();
        let step_deadline = (Instant::now() + Duration::from_secs(2)).min(deadline);
        let reply = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(step_deadline),
            self.read(Class::AggregatorAccounts, index, request, step_deadline),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) if error.is::<super::super::read_http::ReadBodyLimit>() => {
                return Err(invalid("aggregator account RPC body exceeds limit"));
            }
            _ => return Err(unavailable("aggregator account RPC read unavailable")),
        };
        ensure!(
            reply.status.is_success(),
            unavailable("aggregator account RPC HTTP unavailable")
        );
        ensure!(
            reply.body.len() <= maximum,
            invalid("aggregator account RPC body exceeds limit")
        );
        let account = decode(id, &reply.body)?;
        ensure!(
            Instant::now() < step_deadline,
            unavailable("aggregator account RPC deadline expired")
        );
        Ok(account)
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Contextual>,
    error: Option<Fault>,
}
#[derive(Deserialize)]
struct Contextual {
    context: Context,
    value: Option<UiAccount>,
}
#[derive(Deserialize)]
struct Context {
    slot: u64,
}
#[derive(Deserialize)]
struct Fault {
    #[serde(rename = "code")]
    _code: i64,
    #[serde(rename = "message")]
    _message: String,
}

fn decode(id: u64, body: &[u8]) -> Result<Account> {
    let envelope: Envelope = serde_json::from_slice(body)
        .map_err(|_| invalid("aggregator account RPC envelope is invalid"))?;
    ensure!(
        envelope.jsonrpc == "2.0" && envelope.id == id,
        invalid("aggregator account RPC identity is invalid")
    );
    let result = match (envelope.result, envelope.error) {
        (Some(value), None) => value,
        (None, Some(_)) => return Err(unavailable("aggregator account RPC error")),
        _ => return Err(invalid("aggregator account RPC result is invalid")),
    };
    ensure!(
        result.context.slot > 0,
        invalid("aggregator account RPC context is invalid")
    );
    let ui = result
        .value
        .ok_or_else(|| invalid("aggregator account is missing"))?;
    ensure!(
        ui.owner.len() <= 44,
        invalid("aggregator account owner is invalid")
    );
    let owner = ui
        .owner
        .parse()
        .map_err(|_| invalid("aggregator account owner is invalid"))?;
    let UiAccountData::Binary(encoded, UiAccountEncoding::Base64) = ui.data else {
        return Err(invalid("aggregator account encoding is invalid"));
    };
    let data = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| invalid("aggregator account data is invalid"))?;
    ensure!(
        data.len() <= 65_536 && ui.space.is_none_or(|space| space == data.len() as u64),
        invalid("aggregator account size is invalid")
    );
    Ok(Account {
        lamports: ui.lamports,
        owner,
        data,
        executable: ui.executable,
        rent_epoch: ui.rent_epoch,
    })
}

#[cfg(test)]
pub(crate) mod tests;
