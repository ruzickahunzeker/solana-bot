use super::*;
use crate::domain::{trade::Address, wallet::QuoteBalance};
use std::collections::HashSet;

const TOTAL_DEADLINE: Duration = Duration::from_secs(8);
const STEP_DEADLINE: Duration = Duration::from_secs(2);
const MAX_ACCOUNTS: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepError {
    Unavailable,
    Invalid,
}
impl StepError {
    fn error(self) -> anyhow::Error {
        anyhow::anyhow!(match self {
            Self::Unavailable => "quote balance RPC unavailable",
            Self::Invalid => "invalid quote balance RPC response",
        })
    }
}

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn quote_balance(
        &self,
        first_url: &str,
        wallet: Address,
        mint: Address,
    ) -> Result<QuoteBalance> {
        let candidates = self.candidate_indices(&[first_url.to_owned()]);
        let index = *candidates
            .first()
            .ok_or_else(|| anyhow::anyhow!("quote balance RPC endpoint unavailable"))?;
        let deadline = Instant::now() + TOTAL_DEADLINE;
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let _slot = self.slots[Class::QuoteBalance.index()]
                .clone()
                .try_acquire_owned()
                .map_err(|_| anyhow::anyhow!("quote balance quota busy"))?;
            let supply = amount(
                self.quote_step(index, |id| ReadRequest::TokenSupply { id, mint }, deadline)
                    .await
                    .map_err(StepError::error)?,
            )?;
            let decimals = supply.decimals;
            let scan = self
                .quote_step(
                    index,
                    |id| ReadRequest::TokenAccounts { id, wallet, mint },
                    deadline,
                )
                .await;
            let parsed = match scan {
                Ok(value) => scan_balance(value, wallet, mint, decimals)?,
                Err(StepError::Unavailable) => None,
                Err(error) => return Err(error.error()),
            };
            let balance_units = if let Some(total) = parsed {
                total
            } else {
                let mut failed = false;
                let mut found = None;
                for account in atas(wallet, mint) {
                    match self
                        .quote_step(
                            index,
                            |id| ReadRequest::TokenAccountBalance { id, account },
                            deadline,
                        )
                        .await
                    {
                        Ok(value) => {
                            let value = amount(value)?;
                            ensure!(
                                value.decimals == decimals,
                                "quote balance decimals mismatch"
                            );
                            let units = value.units()?;
                            if units > 0 {
                                found = Some(units);
                                break;
                            }
                        }
                        Err(StepError::Unavailable) => failed = true,
                        Err(error) => return Err(error.error()),
                    }
                }
                // Preserve V1: one unavailable ATA and no positive balance is not a known zero.
                if let Some(units) = found {
                    units
                } else {
                    ensure!(!failed, "quote balance ATA lookup unavailable");
                    0
                }
            };
            let value = self
                .quote_step(index, |id| ReadRequest::Balance { id, wallet }, deadline)
                .await
                .map_err(StepError::error)?;
            let balance_lamports = serde_json::from_value::<u64>(value)
                .map_err(|_| anyhow::anyhow!("invalid SOL balance response"))?;
            ensure!(Instant::now() < deadline, "quote balance deadline expired");
            Ok(QuoteBalance {
                decimals,
                balance_units,
                balance_lamports,
            })
        })
        .await
        .map_err(|_| anyhow::anyhow!("quote balance lookup timed out"))?
    }

    async fn quote_step(
        &self,
        index: usize,
        request: impl FnOnce(u64) -> ReadRequest,
        total: Instant,
    ) -> std::result::Result<Value, StepError> {
        if Instant::now() >= total
            || !self
                .available(index, Instant::now())
                .map_err(|_| StepError::Unavailable)?
        {
            return Err(StepError::Unavailable);
        }
        let deadline = (Instant::now() + STEP_DEADLINE).min(total);
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = request(id);
        let max_body = request.max_body();
        let reply = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.read(Class::QuoteBalance, index, request, deadline),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) if error.is::<super::super::read_http::ReadBodyLimit>() => {
                return Err(StepError::Invalid)
            }
            _ => return Err(StepError::Unavailable),
        };
        if !reply.status.is_success() {
            return Err(StepError::Unavailable);
        }
        if reply.body.len() > max_body {
            return Err(StepError::Invalid);
        }
        let value = decode_envelope(id, &reply.body)?;
        if Instant::now() >= deadline {
            return Err(StepError::Unavailable);
        }
        Ok(value)
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Contextual>,
    error: Option<RpcFault>,
}
#[derive(Deserialize)]
struct RpcFault {
    #[serde(rename = "code")]
    _code: i64,
    #[serde(rename = "message")]
    _message: String,
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
fn decode_envelope(id: u64, bytes: &[u8]) -> std::result::Result<Value, StepError> {
    let reply: Envelope = serde_json::from_slice(bytes).map_err(|_| StepError::Invalid)?;
    if reply.jsonrpc != "2.0" || reply.id != id {
        return Err(StepError::Invalid);
    }
    match (reply.result, reply.error) {
        (Some(result), None) => {
            let _slot = result.context.slot;
            Ok(result.value)
        }
        (None, Some(_)) => Err(StepError::Unavailable),
        _ => Err(StepError::Invalid),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenAmount {
    amount: String,
    decimals: u8,
    #[serde(rename = "uiAmount")]
    _ui_amount: Option<f64>,
    #[serde(rename = "uiAmountString")]
    _ui_amount_string: String,
}
impl TokenAmount {
    fn units(&self) -> Result<u64> {
        self.amount
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid token amount"))
    }
}
fn amount(value: Value) -> Result<TokenAmount> {
    let value: TokenAmount = serde_json::from_value(value)
        .map_err(|_| anyhow::anyhow!("invalid token amount metadata"))?;
    value.units()?;
    Ok(value)
}
#[derive(Deserialize)]
struct KeyedAccount {
    pubkey: String,
    account: Account,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    owner: String,
    executable: bool,
    #[serde(rename = "lamports")]
    _lamports: u64,
    #[serde(rename = "rentEpoch")]
    _rent_epoch: u64,
    #[serde(rename = "space")]
    _space: Option<u64>,
    data: Value,
}
fn address(value: &str) -> Result<Address> {
    ensure!(value.len() <= 44, "invalid token account address");
    let mut bytes = [0u8; 32];
    let length = bs58::decode(value)
        .onto(&mut bytes[..])
        .map_err(|_| anyhow::anyhow!("invalid token account address"))?;
    ensure!(length == 32, "invalid token account address");
    Ok(Address::from_bytes(bytes))
}
fn program(label: &str) -> Option<Address> {
    match label {
        "spl-token" => Some(Address::from_bytes(spl_token_interface::ID.to_bytes())),
        "spl-token-2022" => Some(Address::from_bytes(spl_token_2022_interface::ID.to_bytes())),
        _ => None,
    }
}
fn scan_balance(value: Value, wallet: Address, mint: Address, decimals: u8) -> Result<Option<u64>> {
    inspect_scan(value, wallet, mint, Some(decimals), false)
        .map(|value| value.map(|value| value.units))
}

pub(crate) struct PreparationScan {
    pub(crate) units: u64,
    pub(crate) program: Option<Address>,
    pub(crate) decimals: Option<u8>,
}

pub(crate) fn preparation_scan(
    value: Value,
    wallet: Address,
    mint: Address,
) -> Result<Option<PreparationScan>> {
    inspect_scan(value, wallet, mint, None, true)
}

fn inspect_scan(
    value: Value,
    wallet: Address,
    mint: Address,
    mut decimals: Option<u8>,
    preparation: bool,
) -> Result<Option<PreparationScan>> {
    let values = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid token account scan"))?;
    ensure!(
        values.len() <= MAX_ACCOUNTS,
        "token account scan exceeds account limit"
    );
    let accounts: Vec<KeyedAccount> = serde_json::from_value(value)
        .map_err(|_| anyhow::anyhow!("invalid token account metadata"))?;
    if accounts.is_empty() {
        return Ok(Some(PreparationScan {
            units: 0,
            program: None,
            decimals,
        }));
    }
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut parsed = 0;
    let mut unparsed = 0;
    let mut token_program = None;
    for keyed in accounts {
        let account_address = address(&keyed.pubkey)?;
        ensure!(
            seen.insert(account_address.to_bytes()),
            "duplicate token account in scan"
        );
        let account = keyed.account;
        let owner = address(&account.owner)?;
        ensure!(
            !account.executable
                && [
                    program("spl-token").unwrap(),
                    program("spl-token-2022").unwrap()
                ]
                .contains(&owner),
            "invalid token account program"
        );
        if let Some(data) = account.data.as_object() {
            ensure!(
                data.get("space").and_then(Value::as_u64).is_some() && data.contains_key("parsed"),
                "invalid parsed token account metadata"
            );
            let label = data
                .get("program")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("invalid parsed token account"))?;
            let Some(expected) = program(label) else {
                unparsed += 1;
                continue;
            };
            ensure!(owner == expected, "token account program label mismatch");
            let body = data
                .get("parsed")
                .ok_or_else(|| anyhow::anyhow!("invalid parsed token account"))?;
            ensure!(
                body.get("type").and_then(Value::as_str) == Some("account"),
                "unexpected parsed token account type"
            );
            let info = body
                .get("info")
                .ok_or_else(|| anyhow::anyhow!("missing token account info"))?;
            let info_address = |field: &str| -> Result<Address> {
                address(
                    info.get(field)
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("missing token account identity"))?,
                )
            };
            ensure!(
                info_address("owner")? == wallet && info_address("mint")? == mint,
                "token account wallet or mint mismatch"
            );
            ensure!(
                matches!(
                    info.get("state").and_then(Value::as_str),
                    Some("initialized" | "frozen")
                ),
                "invalid token account state"
            );
            if preparation {
                ensure!(
                    info.get("state").and_then(Value::as_str) == Some("initialized"),
                    "frozen preparation token account is unsupported"
                );
                ensure!(
                    token_program.is_none_or(|program| program == owner),
                    "preparation token programs disagree"
                );
            }
            token_program = Some(owner);
            let value = amount(
                info.get("tokenAmount")
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("missing token amount in RPC response"))?,
            )?;
            ensure!(
                decimals.is_none_or(|expected| value.decimals == expected),
                "quote balance decimals mismatch"
            );
            decimals = Some(value.decimals);
            let units = value.units()?;
            if preparation {
                let ata = atas(wallet, mint)[usize::from(owner != program("spl-token").unwrap())];
                ensure!(
                    units == 0 || account_address == ata,
                    "noncanonical positive token holding is not supported by the SDK input account"
                );
                total = total
                    .checked_add(units)
                    .ok_or_else(|| anyhow::anyhow!("preparation token balance overflow"))?;
            } else {
                total = total.saturating_add(units);
            }
            parsed += 1;
        } else {
            let binary = account.data.as_str().is_some()
                || account.data.as_array().is_some_and(|data| {
                    data.len() == 2
                        && data[0].is_string()
                        && matches!(data[1].as_str(), Some("base64" | "base64+zstd" | "base58"))
                });
            ensure!(binary, "invalid unparsed token account");
            unparsed += 1;
        }
    }
    ensure!(
        parsed == 0 || unparsed == 0,
        "incomplete token scan; refusing partial balance"
    );
    Ok(if parsed > 0 {
        Some(PreparationScan {
            units: total,
            program: token_program,
            decimals,
        })
    } else {
        None
    })
}
fn atas(wallet: Address, mint: Address) -> [Address; 2] {
    let wallet = solana_sdk::pubkey::Pubkey::new_from_array(wallet.to_bytes());
    let mint = solana_sdk::pubkey::Pubkey::new_from_array(mint.to_bytes());
    [spl_token_interface::ID, spl_token_2022_interface::ID].map(|program| {
        Address::from_bytes(spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&wallet, &mint, &program).to_bytes())
    })
}

#[cfg(test)]
mod tests;
