#[cfg(test)]
use super::read_runtime;
use super::{
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use crate::{
    application::ports::preparation_balance::PreparationBalanceRead,
    domain::{preparation_balance::PreparationBalanceSnapshot, trade::Address},
};
use std::sync::Arc;

pub(crate) struct RpcPreparationBalanceRead<T = ReqwestReadTransport> {
    wallet: Address,
    url: String,
    read_rpc: Arc<SharedReadRuntime<T>>,
}
impl<T: ReadHttpTransport> RpcPreparationBalanceRead<T> {
    pub(crate) fn new(wallet: Address, url: String, read_rpc: Arc<SharedReadRuntime<T>>) -> Self {
        Self {
            wallet,
            url,
            read_rpc,
        }
    }
}
impl<T: ReadHttpTransport> PreparationBalanceRead for RpcPreparationBalanceRead<T> {
    async fn snapshot(&self, mint: Address) -> anyhow::Result<PreparationBalanceSnapshot> {
        self.read_rpc
            .preparation_balance(&self.url, self.wallet, mint)
            .await
    }
}

#[cfg(test)]
#[path = "preparation_balance/legacy.rs"]
mod legacy;
