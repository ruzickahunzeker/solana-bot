use super::{
    parameter_reads::ParameterReadSession,
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use crate::domain::trade::Address;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) struct SolHopReadSession<T = ReqwestReadTransport>(ParameterReadSession<T>);
impl<T: ReadHttpTransport + 'static> SolHopReadSession<T> {
    pub(crate) fn new(
        runtime: Arc<SharedReadRuntime<T>>,
        url: String,
        commitment: CommitmentConfig,
        quote: Address,
        deadline: Instant,
    ) -> Self {
        Self(ParameterReadSession::sol_hop(
            runtime,
            url,
            commitment,
            quote,
            deadline.min(Instant::now() + Duration::from_secs(5)),
        ))
    }
    pub(crate) fn rpc(&self) -> &RpcClient {
        self.0.rpc()
    }
    pub(crate) fn finish<R>(self, result: anyhow::Result<R>) -> anyhow::Result<R> {
        self.0.finish(result)
    }
}
