use std::sync::Arc;

use crate::{application::ports::blockhash::LatestBlockhashReader, domain::blockhash::Blockhash};

use super::{read_http::ReadHttpTransport, read_runtime::SharedReadRuntime};

pub(crate) struct RpcBlockhashReader<T = super::read_http::ReqwestReadTransport> {
    runtime: Arc<SharedReadRuntime<T>>,
}

impl<T: ReadHttpTransport> RpcBlockhashReader<T> {
    pub(crate) fn new(runtime: Arc<SharedReadRuntime<T>>) -> Self {
        Self { runtime }
    }
}

impl<T: ReadHttpTransport> LatestBlockhashReader for RpcBlockhashReader<T> {
    async fn latest_blockhash(&self) -> anyhow::Result<Blockhash> {
        self.runtime.latest_blockhash().await
    }
}

#[cfg(test)]
mod legacy_tests;
