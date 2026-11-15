use super::*;
use crate::{
    adapters::sdk::preparation_assets,
    domain::{
        preparation_balance::{
            BalanceFailureKind, PreparationBalanceFailure, PreparationBalanceSnapshot,
            PreparationInput,
        },
        trade::Address,
    },
};
use solana_account_decoder::UiAccount;

const TOTAL: Duration = Duration::from_secs(4);
const STEP: Duration = Duration::from_secs(2);

fn invalid(message: &'static str) -> anyhow::Error {
    PreparationBalanceFailure::new(BalanceFailureKind::Invalid, message).into()
}
fn unavailable(message: &'static str) -> anyhow::Error {
    PreparationBalanceFailure::new(BalanceFailureKind::Unavailable, message).into()
}
fn retryable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<PreparationBalanceFailure>()
        .is_some_and(|failure| failure.allows_lane_fallback())
}

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn preparation_balance(
        &self,
        url: &str,
        wallet: Address,
        mint: Address,
    ) -> Result<PreparationBalanceSnapshot> {
        let started = Instant::now();
        let deadline = started + TOTAL;
        let index = *self
            .candidate_indices(&[url.to_owned()])
            .first()
            .ok_or_else(|| unavailable("preparation RPC endpoint is not registered"))?;
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let _slot = self.slots[Class::PreparationBalance.index()]
                .clone()
                .try_acquire_owned()
                .map_err(|_| unavailable("preparation RPC quota busy"))?;
            let (scan, minimum_slot) = match self
                .preparation_step(
                    index,
                    |id| ReadRequest::TokenAccounts { id, wallet, mint },
                    deadline,
                )
                .await
            {
                Ok((slot, value)) => {
                    let scan = preparation_scan(value, wallet, mint).map_err(|_| {
                        invalid(
                            "preparation scan cannot verify wallet, mint, state or input account",
                        )
                    })?;
                    (scan, Some(slot))
                }
                Err(error) if retryable(&error) => (None, None),
                Err(error) => return Err(error),
            };
            let accounts = preparation_assets::input_keys(wallet, mint)
                .map(|key| Address::from_bytes(key.to_bytes()));
            let (slot, value) = self
                .preparation_step(
                    index,
                    |id| ReadRequest::PreparationAccounts {
                        id,
                        accounts,
                        minimum_slot,
                    },
                    deadline,
                )
                .await?;
            if minimum_slot.is_some_and(|minimum| slot < minimum) {
                return Err(invalid("preparation raw account slot is invalid"));
            }
            let values: Vec<Option<UiAccount>> = serde_json::from_value(value)
                .map_err(|_| invalid("preparation raw account metadata is invalid"))?;
            let raw = preparation_assets::inspect(wallet, mint, &values).map_err(|_| {
                invalid("preparation raw mint or input account evidence is invalid")
            })?;
            if scan.is_some_and(|scan| {
                scan.units != raw.units
                    || scan.program.is_some_and(|program| program != raw.program)
                    || scan
                        .decimals
                        .is_some_and(|decimals| decimals != raw.decimals)
            }) {
                return Err(invalid("preparation scan and raw input evidence disagree"));
            }
            PreparationBalanceSnapshot::from_account(
                wallet,
                mint,
                raw.units,
                PreparationInput {
                    account: raw.account,
                    program: raw.program,
                    decimals: raw.decimals,
                },
                slot,
                started,
            )
        })
        .await
        .map_err(|_| unavailable("preparation balance read timed out"))?
    }

    async fn preparation_step(
        &self,
        index: usize,
        request: impl FnOnce(u64) -> ReadRequest,
        total: Instant,
    ) -> Result<(u64, Value)> {
        if Instant::now() >= total
            || !self
                .available(index, Instant::now())
                .map_err(|_| unavailable("preparation RPC budget unavailable"))?
        {
            return Err(unavailable(
                "preparation RPC endpoint cooling down or unavailable",
            ));
        }
        let deadline = (Instant::now() + STEP).min(total);
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = request(id);
        let max_body = request.max_body();
        let reply = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.read(Class::PreparationBalance, index, request, deadline),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) if error.is::<super::super::read_http::ReadBodyLimit>() => {
                return Err(invalid("preparation RPC body exceeds limit"))
            }
            _ => return Err(unavailable("preparation RPC read unavailable")),
        };
        if !reply.status.is_success() {
            return Err(unavailable("preparation RPC HTTP unavailable"));
        }
        if reply.body.len() > max_body {
            return Err(invalid("preparation RPC body exceeds limit"));
        }
        let value = decode(id, &reply.body)?;
        if Instant::now() >= deadline {
            return Err(unavailable("preparation RPC deadline expired"));
        }
        Ok(value)
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
    value: Value,
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
fn decode(id: u64, body: &[u8]) -> Result<(u64, Value)> {
    let response: Envelope =
        serde_json::from_slice(body).map_err(|_| invalid("preparation RPC envelope is invalid"))?;
    if response.jsonrpc != "2.0" || response.id != id {
        return Err(invalid("preparation RPC identity is invalid"));
    }
    match (response.result, response.error) {
        (Some(result), None) if result.context.slot > 0 => Ok((result.context.slot, result.value)),
        (None, Some(_)) => Err(unavailable("preparation RPC error")),
        _ => Err(invalid("preparation RPC result or context is invalid")),
    }
}

#[cfg(test)]
pub(crate) mod tests;
