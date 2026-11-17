use super::{
    parameter_reads::ParameterReadSession,
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use crate::domain::trade::{Address, Pool};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use std::{sync::Arc, time::Instant};

#[derive(Clone, Copy)]
pub(crate) enum DiscoveryProtocol {
    Pump,
    Dbc,
    PumpSwap,
}

/// One protocol branch; a failed sibling cannot erase this branch's own evidence.
pub(crate) struct DiscoveryReadSession<T = ReqwestReadTransport>(ParameterReadSession<T>);

impl<T: ReadHttpTransport + 'static> DiscoveryReadSession<T> {
    pub(crate) fn new(
        runtime: Arc<SharedReadRuntime<T>>,
        url: String,
        commitment: CommitmentConfig,
        protocol: DiscoveryProtocol,
        mint: Address,
        deadline: Instant,
    ) -> Self {
        let pool = match protocol {
            DiscoveryProtocol::Pump => Pool::Pump,
            DiscoveryProtocol::Dbc => Pool::MeteoraDbc,
            DiscoveryProtocol::PumpSwap => Pool::PumpAmm,
        };
        let deadline = deadline.min(Instant::now() + std::time::Duration::from_secs(3));
        Self(ParameterReadSession::discovery(
            runtime, url, commitment, pool, mint, deadline,
        ))
    }

    pub(crate) fn rpc(&self) -> &RpcClient {
        self.0.rpc()
    }

    pub(crate) fn finish<R>(self, result: anyhow::Result<R>) -> anyhow::Result<R> {
        self.0.finish(result)
    }
}
