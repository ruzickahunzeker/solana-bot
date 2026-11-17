use anyhow::Result;
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use std::{future::Future, time::Duration};

pub(super) const MAX_BODY: usize = 65_536;
pub(super) const MAX_TOKEN_BODY: usize = 524_288;

#[derive(Debug)]
pub(crate) struct ReadBodyLimit(&'static str);
impl std::fmt::Display for ReadBodyLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} RPC body exceeds limit", self.0)
    }
}
impl std::error::Error for ReadBodyLimit {}

pub(crate) struct HttpReply {
    pub(super) status: StatusCode,
    pub(super) retry_after: Option<u64>,
    pub(super) body: Vec<u8>,
}

pub(crate) enum ReadRequest {
    Parameters {
        id: u64,
        query: super::parameter_reads::request::ParameterQuery,
    },
    SignatureStatus {
        id: u64,
        signature: String,
    },
    LatestBlockhash {
        id: u64,
    },
    NativeBalances {
        id: u64,
        wallets: Vec<crate::domain::trade::Address>,
    },
    TokenSupply {
        id: u64,
        mint: crate::domain::trade::Address,
    },
    TokenAccounts {
        id: u64,
        wallet: crate::domain::trade::Address,
        mint: crate::domain::trade::Address,
    },
    TokenAccountBalance {
        id: u64,
        account: crate::domain::trade::Address,
    },
    Balance {
        id: u64,
        wallet: crate::domain::trade::Address,
    },
    PreparationAccounts {
        id: u64,
        accounts: [crate::domain::trade::Address; 3],
        minimum_slot: Option<u64>,
    },
}

impl ReadRequest {
    pub(super) fn body(&self) -> Value {
        match self {
            Self::Parameters { id, query } => query.body(*id),
            Self::SignatureStatus { id, signature } => {
                json!({"jsonrpc":"2.0","id":id,"method":"getSignatureStatuses","params":[[signature],{"searchTransactionHistory":true}]})
            }
            Self::LatestBlockhash { id } => {
                json!({"jsonrpc":"2.0","id":id,"method":"getLatestBlockhash","params":[{"commitment":"processed"}]})
            }
            Self::NativeBalances { id, wallets } => {
                let wallets = wallets.iter().map(ToString::to_string).collect::<Vec<_>>();
                json!({"jsonrpc":"2.0","id":id,"method":"getMultipleAccounts","params":[wallets,{"commitment":"processed","encoding":"base64","dataSlice":{"offset":0,"length":0}}]})
            }
            Self::TokenSupply { id, mint } => {
                json!({"jsonrpc":"2.0","id":id,"method":"getTokenSupply","params":[mint.to_string(),{"commitment":"processed"}]})
            }
            Self::TokenAccounts { id, wallet, mint } => {
                json!({"jsonrpc":"2.0","id":id,"method":"getTokenAccountsByOwner","params":[wallet.to_string(),{"mint":mint.to_string()},{"encoding":"jsonParsed","dataSlice":null,"commitment":"processed","minContextSlot":null}]})
            }
            Self::TokenAccountBalance { id, account } => {
                json!({"jsonrpc":"2.0","id":id,"method":"getTokenAccountBalance","params":[account.to_string(),{"commitment":"processed"}]})
            }
            Self::Balance { id, wallet } => {
                json!({"jsonrpc":"2.0","id":id,"method":"getBalance","params":[wallet.to_string(),{"commitment":"processed"}]})
            }
            Self::PreparationAccounts {
                id,
                accounts,
                minimum_slot,
            } => {
                json!({"jsonrpc":"2.0","id":id,"method":"getMultipleAccounts","params":[accounts.map(|account|account.to_string()),{"encoding":"base64","dataSlice":null,"commitment":"processed","minContextSlot":minimum_slot}]})
            }
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Parameters { .. } => "protocol parameters",
            Self::SignatureStatus { .. } => "observation",
            Self::LatestBlockhash { .. } => "blockhash",
            Self::NativeBalances { .. } => "native balances",
            Self::PreparationAccounts { .. } => "preparation balance",
            Self::TokenSupply { .. }
            | Self::TokenAccounts { .. }
            | Self::TokenAccountBalance { .. }
            | Self::Balance { .. } => "quote balance",
        }
    }

    pub(super) fn max_body(&self) -> usize {
        if let Self::Parameters { query, .. } = self {
            return query.max_body();
        }
        if matches!(self, Self::TokenAccounts { .. }) {
            MAX_TOKEN_BODY
        } else {
            MAX_BODY
        }
    }
}

pub(crate) trait ReadHttpTransport: Send + Sync {
    fn fetch(
        &self,
        url: &Url,
        request: &ReadRequest,
    ) -> impl Future<Output = Result<HttpReply>> + Send;
}

pub(crate) struct ReqwestReadTransport {
    client: reqwest::Client,
}

impl ReqwestReadTransport {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .build()
                .map_err(|_| anyhow::anyhow!("cannot initialize shared read HTTP transport"))?,
        })
    }
}

impl ReadHttpTransport for ReqwestReadTransport {
    async fn fetch(&self, url: &Url, request: &ReadRequest) -> Result<HttpReply> {
        let label = request.label();
        let max_body = request.max_body();
        let mut response = self
            .client
            .post(url.clone())
            .json(&request.body())
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("{label} HTTP transport unavailable"))?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if !status.is_success() {
            return Ok(HttpReply {
                status,
                retry_after,
                body: vec![],
            });
        }
        if response
            .content_length()
            .is_some_and(|len| len > max_body as u64)
        {
            return Err(ReadBodyLimit(label).into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("{label} RPC body unavailable"))?
        {
            if body.len().saturating_add(chunk.len()) > max_body {
                return Err(ReadBodyLimit(label).into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpReply {
            status,
            retry_after,
            body,
        })
    }
}
