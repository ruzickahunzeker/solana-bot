use super::{
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use crate::{
    application::ports::reconciliation::SignatureStatusReader,
    domain::reconciliation::ChainObservation,
};
use std::sync::Arc;

/// A metadata snapshot of at most the original two RPCs, with no SDK client or signer.
pub(crate) struct RpcQuoteStatusReader<T = ReqwestReadTransport> {
    runtime: Arc<SharedReadRuntime<T>>,
    urls: Vec<String>,
}
impl<T: ReadHttpTransport> RpcQuoteStatusReader<T> {
    pub(crate) fn new(runtime: Arc<SharedReadRuntime<T>>, urls: Vec<String>) -> Self {
        Self {
            runtime,
            urls: urls.into_iter().take(2).collect(),
        }
    }
}
impl<T: ReadHttpTransport> SignatureStatusReader for RpcQuoteStatusReader<T> {
    async fn lookup_signature(&self, signature: &str) -> anyhow::Result<Option<ChainObservation>> {
        self.runtime
            .quote_signature_status(&self.urls, signature)
            .await
    }
}
