use crate::{
    adapters::rpc::{read_http::ReadHttpTransport, read_runtime::SharedReadRuntime},
    domain::reconciliation::ChainObservation,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;

pub(super) async fn lookup_selected<T: ReadHttpTransport>(
    runtime: &SharedReadRuntime<T>,
    rpcs: Vec<Arc<RpcClient>>,
    signature: &str,
) -> anyhow::Result<Option<ChainObservation>> {
    let urls = rpcs
        .into_iter()
        .take(2)
        .map(|rpc| rpc.url())
        .collect::<Vec<_>>();
    runtime.quote_signature_status(&urls, signature).await
}

#[cfg(test)]
mod tests;
