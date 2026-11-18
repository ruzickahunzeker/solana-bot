use super::{invalid, ParameterReadScope};
use anyhow::{ensure, Result};
use base64::Engine;
use serde_json::{json, Value};
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_client::{
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
    rpc_request::RpcRequest,
    rpc_response::{RpcKeyedAccount, RpcResponseContext},
};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

#[cfg(test)]
pub(crate) mod pump_curve;

#[derive(Clone, Copy)]
enum Kind {
    Account,
    Accounts(usize),
    Epoch,
    ProgramScan,
    TokenBalance,
}

#[derive(Clone)]
pub(crate) struct ParameterQuery {
    kind: Kind,
    params: Value,
}

pub(super) struct PumpConfigSnapshot {
    config: Value,
    context: Value,
    values: [Value; 2],
}

fn pump_config_keys() -> [String; 2] {
    use sol_trade_sdk::instruction::utils::pumpswap::accounts;
    [
        accounts::FEE_CONFIG.to_string(),
        accounts::GLOBAL_ACCOUNT.to_string(),
    ]
}

fn key(value: &Value) -> Result<()> {
    let value = value
        .as_str()
        .ok_or_else(|| invalid("invalid parameter account address"))?;
    ensure!(
        value.len() <= 44,
        invalid("invalid parameter account address")
    );
    Pubkey::from_str(value).map_err(|_| invalid("invalid parameter account address"))?;
    Ok(())
}

fn account_config(value: &mut Value) -> Result<()> {
    let config: RpcAccountInfoConfig = serde_json::from_value(value.clone())
        .map_err(|_| invalid("invalid parameter account configuration"))?;
    ensure!(
        matches!(
            config.encoding,
            Some(UiAccountEncoding::Base64 | UiAccountEncoding::Base64Zstd)
        ) && config.data_slice.is_none()
            && config.min_context_slot.is_none(),
        invalid("unsupported parameter account configuration")
    );
    ensure!(
        value
            .as_object()
            .is_some_and(|object| object.keys().all(|name| matches!(
                name.as_str(),
                "encoding" | "dataSlice" | "commitment" | "minContextSlot"
            ))),
        invalid("unsupported parameter account configuration")
    );
    // Bound decoded data without accepting compressed-response expansion or changing SDK codecs.
    value["encoding"] = json!("base64");
    Ok(())
}

impl ParameterQuery {
    pub(super) fn pump_config_reply(&self, snapshot: &PumpConfigSnapshot) -> Option<Value> {
        if !matches!(self.kind, Kind::Account) || self.params[1] != snapshot.config {
            return None;
        }
        pump_config_keys()
            .iter()
            .position(|key| self.params[0] == *key)
            .map(|index| json!({"context":snapshot.context,"value":snapshot.values[index]}))
    }

    pub(super) fn with_pump_configs(&self, scope: ParameterReadScope) -> Option<Self> {
        if scope.pool != crate::domain::trade::Pool::PumpAmm
            || !matches!(self.kind, Kind::Accounts(4))
            || scope
                .pool_id
                .is_some_and(|id| self.params[0][0] != id.to_string())
        {
            return None;
        }
        let mut query = self.clone();
        let accounts = query.params[0]
            .as_array_mut()
            .expect("validated account list");
        accounts.extend(pump_config_keys().map(Value::String));
        query.kind = Kind::Accounts(6);
        Some(query)
    }

    pub(super) fn split_pump_configs(&self, mut result: Value) -> (Value, PumpConfigSnapshot) {
        let values = result["value"]
            .as_array_mut()
            .expect("validated account result");
        let global = values.pop().expect("validated six-account result");
        let fee = values.pop().expect("validated six-account result");
        let snapshot = PumpConfigSnapshot {
            config: self.params[1].clone(),
            context: result["context"].clone(),
            values: [fee, global],
        };
        (result, snapshot)
    }

    pub(super) fn parse(
        method: RpcRequest,
        mut params: Value,
        scope: ParameterReadScope,
    ) -> Result<Self> {
        let length = params
            .as_array()
            .map(Vec::len)
            .ok_or_else(|| invalid("invalid parameter RPC arguments"))?;
        let kind = match method {
            RpcRequest::GetAccountInfo if length == 2 => {
                key(&params[0])?;
                account_config(&mut params[1])?;
                Kind::Account
            }
            RpcRequest::GetMultipleAccounts if length == 2 => {
                let accounts = params[0]
                    .as_array()
                    .ok_or_else(|| invalid("invalid parameter account list"))?;
                ensure!(
                    (1..=100).contains(&accounts.len()),
                    invalid("parameter account list exceeds limit")
                );
                for account in accounts {
                    key(account)?;
                }
                let count = accounts.len();
                account_config(&mut params[1])?;
                Kind::Accounts(count)
            }
            RpcRequest::GetEpochInfo if length == 1 => {
                serde_json::from_value::<solana_commitment_config::CommitmentConfig>(
                    params[0].clone(),
                )
                .map_err(|_| invalid("invalid parameter epoch configuration"))?;
                ensure!(
                    params[0].as_object().is_some_and(
                        |object| object.len() == 1 && object.contains_key("commitment")
                    ),
                    invalid("invalid parameter epoch configuration")
                );
                Kind::Epoch
            }
            RpcRequest::GetTokenAccountBalance if length == 2 && scope.hop_quote.is_some() => {
                key(&params[0])?;
                serde_json::from_value::<solana_commitment_config::CommitmentConfig>(
                    params[1].clone(),
                )
                .map_err(|_| invalid("invalid hop balance configuration"))?;
                ensure!(
                    params[1].as_object().is_some_and(
                        |object| object.len() == 1 && object.contains_key("commitment")
                    ),
                    invalid("invalid hop balance configuration")
                );
                Kind::TokenBalance
            }
            RpcRequest::GetProgramAccounts if length == 2 => {
                let pump = scope.pump_scan_mint();
                let dbc = scope.dbc_scan_mint();
                use sol_trade_sdk::instruction::utils::{
                    meteora_dbc, pumpswap::accounts::AMM_PROGRAM,
                    pumpswap_types::POOL_DISCRIMINATOR,
                };
                let (program, profiles) = if let Some(quote) = scope.hop_quote {
                    use sol_trade_sdk::instruction::utils::{
                        raydium_cpmm::accounts::RAYDIUM_CPMM,
                        raydium_cpmm_types::POOL_STATE_DISCRIMINATOR,
                    };
                    let wsol = sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
                    ensure!(
                        quote.to_bytes() != wsol.to_bytes(),
                        invalid("SOL hop quote is already WSOL")
                    );
                    let profiles = [(168usize, 200usize), (200, 168)]
                        .map(|(quote_offset, wsol_offset)| {
                            vec![
                                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                    0,
                                    &POOL_STATE_DISCRIMINATOR,
                                )),
                                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                    quote_offset,
                                    &quote.to_bytes(),
                                )),
                                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                    wsol_offset,
                                    wsol.as_ref(),
                                )),
                            ]
                        })
                        .to_vec();
                    (RAYDIUM_CPMM, profiles)
                } else {
                    let mint = pump.or(dbc).ok_or_else(|| {
                        invalid("parameter program scan is not permitted for this loader")
                    })?;
                    let profiles = if pump.is_some() {
                        [252u64, 261, 270, 271, 300, 643]
                            .into_iter()
                            .flat_map(|size| {
                                [43usize, 75].map(|offset| (size, offset, POOL_DISCRIMINATOR))
                            })
                            .collect::<Vec<_>>()
                    } else {
                        vec![(
                            meteora_dbc::VIRTUAL_POOL_ACCOUNT_SIZE,
                            meteora_dbc::BASE_MINT_OFFSET,
                            meteora_dbc::VIRTUAL_POOL_DISCRIMINATOR,
                        )]
                    };
                    let filters = profiles
                        .into_iter()
                        .map(|(size, offset, discriminator)| {
                            vec![
                                RpcFilterType::DataSize(size),
                                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                    0,
                                    &discriminator,
                                )),
                                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                                    offset,
                                    &mint.to_bytes(),
                                )),
                            ]
                        })
                        .collect();
                    (
                        if pump.is_some() {
                            AMM_PROGRAM
                        } else {
                            meteora_dbc::accounts::PROGRAM
                        },
                        filters,
                    )
                };
                ensure!(
                    params[0] == program.to_string(),
                    invalid("parameter program scan has wrong program")
                );
                let config: RpcProgramAccountsConfig = serde_json::from_value(params[1].clone())
                    .map_err(|_| invalid("invalid parameter program scan configuration"))?;
                ensure!(
                    config.account_config.encoding == Some(UiAccountEncoding::Base64)
                        && config.account_config.data_slice.is_none()
                        && config.account_config.min_context_slot.is_none()
                        && config.with_context != Some(true)
                        && config.sort_results.is_none(),
                    invalid("unsupported parameter program scan configuration")
                );
                ensure!(
                    params[1]
                        .as_object()
                        .is_some_and(|object| object.keys().all(|name| matches!(
                            name.as_str(),
                            "filters"
                                | "encoding"
                                | "dataSlice"
                                | "commitment"
                                | "minContextSlot"
                                | "withContext"
                                | "sortResults"
                        ))),
                    invalid("unsupported parameter program scan configuration")
                );
                let allowed = profiles.into_iter().any(|filters| {
                    serde_json::to_value(filters)
                        .is_ok_and(|filters| params[1]["filters"] == filters)
                });
                ensure!(
                    allowed,
                    invalid("parameter program scan filters are not audited")
                );
                Kind::ProgramScan
            }
            _ => return Err(invalid("parameter RPC method is not permitted")),
        };
        Ok(Self { kind, params })
    }

    pub(crate) fn body(&self, id: u64) -> Value {
        let method = match self.kind {
            Kind::Account => "getAccountInfo",
            Kind::Accounts(_) => "getMultipleAccounts",
            Kind::Epoch => "getEpochInfo",
            Kind::ProgramScan => "getProgramAccounts",
            Kind::TokenBalance => "getTokenAccountBalance",
        };
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":self.params})
    }

    pub(crate) fn max_body(&self) -> usize {
        if matches!(self.kind, Kind::ProgramScan) {
            1_048_576
        } else {
            524_288
        }
    }

    pub(crate) fn validate_result(&self, result: &Value) -> Result<()> {
        match self.kind {
            Kind::Account | Kind::Accounts(_) => {
                let context: RpcResponseContext = serde_json::from_value(result["context"].clone())
                    .map_err(|_| invalid("parameter account context is invalid"))?;
                ensure!(
                    context.slot > 0,
                    invalid("parameter account context is invalid")
                );
                if let Kind::Accounts(count) = self.kind {
                    let accounts = result["value"]
                        .as_array()
                        .ok_or_else(|| invalid("parameter account result is invalid"))?;
                    ensure!(
                        accounts.len() == count,
                        invalid("parameter account result count is invalid")
                    );
                    for account in accounts {
                        validate_account(account)?;
                    }
                } else {
                    ensure!(
                        result
                            .as_object()
                            .is_some_and(|object| object.contains_key("value")),
                        invalid("parameter account result is invalid")
                    );
                    validate_account(&result["value"])?;
                }
            }
            Kind::Epoch => {
                serde_json::from_value::<solana_sdk::epoch_info::EpochInfo>(result.clone())
                    .map_err(|_| invalid("parameter epoch result is invalid"))?;
            }
            Kind::TokenBalance => {
                let context: RpcResponseContext = serde_json::from_value(result["context"].clone())
                    .map_err(|_| invalid("invalid hop balance context"))?;
                ensure!(context.slot > 0, invalid("invalid hop balance context"));
                let amount: solana_account_decoder::parse_token::UiTokenAmount =
                    serde_json::from_value(result["value"].clone())
                        .map_err(|_| invalid("invalid hop balance metadata"))?;
                amount
                    .amount
                    .parse::<u64>()
                    .map_err(|_| invalid("invalid hop balance units"))?;
            }
            Kind::ProgramScan => {
                let rows = result
                    .as_array()
                    .ok_or_else(|| invalid("parameter program scan result is invalid"))?;
                ensure!(
                    rows.len() <= 1000,
                    invalid("parameter program scan exceeds account limit")
                );
                for row in rows {
                    let _: RpcKeyedAccount = serde_json::from_value(row.clone())
                        .map_err(|_| invalid("parameter program scan metadata is invalid"))?;
                    key(&row["pubkey"])?;
                    ensure!(
                        !row["account"].is_null(),
                        invalid("parameter program scan account is missing")
                    );
                    validate_account(&row["account"])?;
                }
            }
        }
        Ok(())
    }
}

fn validate_account(value: &Value) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    let account: UiAccount = serde_json::from_value(value.clone())
        .map_err(|_| invalid("parameter account metadata is invalid"))?;
    key(&json!(account.owner))?;
    let UiAccountData::Binary(data, UiAccountEncoding::Base64) = account.data else {
        return Err(invalid(
            "parameter account requires uncompressed binary data",
        ));
    };
    ensure!(
        data.len() <= 87_384,
        invalid("parameter decoded account exceeds limit")
    );
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| invalid("parameter account encoding is invalid"))?;
    ensure!(
        decoded.len() <= 65_536
            && account
                .space
                .is_none_or(|space| space == decoded.len() as u64),
        invalid("parameter decoded account size is invalid")
    );
    Ok(())
}
