use super::{
    read_http::{ReadHttpTransport, ReqwestReadTransport},
    read_runtime::SharedReadRuntime,
};
use crate::domain::{
    parameter_read::{ParameterReadFailure, ParameterReadFailureKind},
    trade::{Address, Pool},
};
use anyhow::Result;
use serde_json::Value;
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_commitment_config::CommitmentConfig;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{watch, OwnedSemaphorePermit};

pub(crate) mod request;

pub(crate) fn invalid(message: &'static str) -> anyhow::Error {
    ParameterReadFailure::new(ParameterReadFailureKind::Invalid, message).into()
}
pub(crate) fn unavailable(message: &'static str) -> anyhow::Error {
    ParameterReadFailure::new(ParameterReadFailureKind::Unavailable, message).into()
}

#[derive(Clone, Copy)]
pub(crate) struct ParameterReadScope {
    pool: Pool,
    mint: Address,
    pool_id: Option<Address>,
    discovery: bool,
    hop_quote: Option<Address>,
}
impl ParameterReadScope {
    pub(crate) fn new(pool: Pool, mint: Address, pool_id: Option<Address>) -> Self {
        Self {
            pool,
            mint,
            pool_id,
            discovery: false,
            hop_quote: None,
        }
    }
    fn pump_scan_mint(self) -> Option<Address> {
        (self.pool == Pool::PumpAmm && self.pool_id.is_none()).then_some(self.mint)
    }
    fn dbc_scan_mint(self) -> Option<Address> {
        (self.discovery && self.pool == Pool::MeteoraDbc).then_some(self.mint)
    }
}

#[derive(Clone, Copy)]
enum Profile {
    Parameters,
    Discovery,
    SolHop,
}

struct State {
    deadline: Instant,
    scope: ParameterReadScope,
    profile: Profile,
    serial: tokio::sync::Mutex<()>,
    permit: Mutex<Option<OwnedSemaphorePermit>>,
    fault: Mutex<Option<ParameterReadFailure>>,
    calls: AtomicUsize,
    active: AtomicUsize,
    pump_configs: Mutex<Option<request::PumpConfigSnapshot>>,
    closed: watch::Sender<bool>,
}
impl State {
    fn release_if_closed(&self) {
        if *self.closed.borrow() && self.active.load(Ordering::SeqCst) == 0 {
            if let Ok(mut permit) = self.permit.lock() {
                permit.take();
            }
            if let Ok(mut snapshot) = self.pump_configs.lock() {
                snapshot.take();
            }
        }
    }
    fn close(&self) {
        self.closed.send_replace(true);
        self.release_if_closed();
    }
    fn fail(&self, error: &anyhow::Error) {
        let failure = error
            .downcast_ref::<ParameterReadFailure>()
            .copied()
            .unwrap_or_else(|| {
                ParameterReadFailure::new(
                    ParameterReadFailureKind::Unavailable,
                    "parameter RPC session unavailable",
                )
            });
        if let Ok(mut fault) = self.fault.lock() {
            if fault.is_none() || !failure.allows_lane_fallback() {
                *fault = Some(failure);
            }
        }
    }
    fn check(&self) -> Result<()> {
        if let Some(fault) = *self
            .fault
            .lock()
            .map_err(|_| unavailable("parameter RPC session unavailable"))?
        {
            return Err(fault.into());
        }
        if *self.closed.borrow() {
            return Err(unavailable("parameter RPC session closed"));
        }
        if Instant::now() >= self.deadline {
            return Err(unavailable("parameter RPC session deadline expired"));
        }
        Ok(())
    }
}

struct RequestGuard<'a> {
    state: &'a State,
    completed: bool,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.state
                .fail(&unavailable("parameter RPC request cancelled"));
        }
        self.state.active.fetch_sub(1, Ordering::SeqCst);
        self.state.release_if_closed();
    }
}

struct Sender<T> {
    runtime: Arc<SharedReadRuntime<T>>,
    url: String,
    state: Arc<State>,
}

/// A keyless loader scope, not the execution client's RPC or signing/submission authority.
pub(crate) struct ParameterReadSession<T = ReqwestReadTransport> {
    rpc: RpcClient,
    state: Arc<State>,
    _transport: std::marker::PhantomData<T>,
}
impl<T: ReadHttpTransport + 'static> ParameterReadSession<T> {
    pub(crate) fn new(
        runtime: Arc<SharedReadRuntime<T>>,
        url: String,
        commitment: CommitmentConfig,
        scope: ParameterReadScope,
    ) -> Self {
        Self::scoped(
            runtime,
            url,
            commitment,
            scope,
            Profile::Parameters,
            Instant::now() + Duration::from_secs(8),
        )
    }
    pub(super) fn discovery(
        runtime: Arc<SharedReadRuntime<T>>,
        url: String,
        commitment: CommitmentConfig,
        pool: Pool,
        mint: Address,
        deadline: Instant,
    ) -> Self {
        let mut scope = ParameterReadScope::new(pool, mint, None);
        scope.discovery = true;
        Self::scoped(
            runtime,
            url,
            commitment,
            scope,
            Profile::Discovery,
            deadline,
        )
    }
    pub(super) fn sol_hop(
        runtime: Arc<SharedReadRuntime<T>>,
        url: String,
        commitment: CommitmentConfig,
        quote: Address,
        deadline: Instant,
    ) -> Self {
        let mut scope = ParameterReadScope::new(Pool::RaydiumCpmm, quote, None);
        scope.hop_quote = Some(quote);
        Self::scoped(runtime, url, commitment, scope, Profile::SolHop, deadline)
    }
    fn scoped(
        runtime: Arc<SharedReadRuntime<T>>,
        url: String,
        commitment: CommitmentConfig,
        scope: ParameterReadScope,
        profile: Profile,
        deadline: Instant,
    ) -> Self {
        let (closed, _) = watch::channel(false);
        let state = Arc::new(State {
            deadline,
            scope,
            profile,
            serial: tokio::sync::Mutex::new(()),
            permit: Mutex::new(None),
            fault: Mutex::new(None),
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            pump_configs: Mutex::new(None),
            closed,
        });
        let rpc = RpcClient::new_sender(
            Sender {
                runtime,
                url,
                state: state.clone(),
            },
            RpcClientConfig {
                commitment_config: commitment,
                ..RpcClientConfig::default()
            },
        );
        Self {
            rpc,
            state,
            _transport: std::marker::PhantomData,
        }
    }
    pub(crate) fn rpc(&self) -> &RpcClient {
        &self.rpc
    }
    pub(crate) fn finish<R>(self, result: Result<R>) -> Result<R> {
        // Some SDK optional config readers erase transport errors. They cannot erase this scope's fault.
        let checked = self.state.check();
        self.state.close();
        checked?;
        result
    }
}
impl<T> Drop for ParameterReadSession<T> {
    fn drop(&mut self) {
        self.state.close();
    }
}

impl<T: ReadHttpTransport + 'static> Sender<T> {
    async fn request(&self, method: RpcRequest, params: Value) -> Result<Value> {
        self.state.check()?;
        if self.state.calls.fetch_add(1, Ordering::Relaxed) >= 32 {
            return Err(invalid("parameter RPC session request count exceeds limit"));
        }
        let query = request::ParameterQuery::parse(method, params, self.state.scope)?;
        let mut cancelled = self.state.closed.subscribe();
        self.state.active.fetch_add(1, Ordering::SeqCst);
        let mut guard = RequestGuard {
            state: &self.state,
            completed: false,
        };
        let result = tokio::select! {
            biased;
            _ = cancelled.changed() => Err(unavailable("parameter RPC session closed")),
            result = tokio::time::timeout_at(tokio::time::Instant::from_std(self.state.deadline), async {
                let _serial = self.state.serial.lock().await;
                self.state.check()?;
                {
                    let snapshot = self.state.pump_configs.lock().map_err(|_| unavailable("parameter snapshot unavailable"))?;
                    if let Some(reply) = snapshot.as_ref().and_then(|snapshot| query.pump_config_reply(snapshot)) {
                        return Ok(reply);
                    }
                }
                {
                    let mut permit = self.state.permit.lock().map_err(|_| unavailable("parameter RPC quota unavailable"))?;
                    if permit.is_none() {
                        *permit = Some(match self.state.profile {
                            Profile::Parameters => self.runtime.parameter_slot()?,
                            Profile::Discovery => self.runtime.discovery_slot()?,
                            Profile::SolHop => self.runtime.sol_hop_slot()?,
                        });
                    }
                }
                self.state.check()?;
                // The pinned SDK gives both optional config reads only 180 ms. Include their
                // raw accounts in its coherent pool snapshot, then feed the unchanged decoders.
                if let Some(expanded) = query.with_pump_configs(self.state.scope) {
                    let result = self.step(expanded.clone()).await?;
                    let (result, snapshot) = expanded.split_pump_configs(result);
                    *self.state.pump_configs.lock().map_err(|_| unavailable("parameter snapshot unavailable"))? = Some(snapshot);
                    Ok(result)
                } else {
                    self.step(query).await
                }
            }) => result.unwrap_or_else(|_| Err(unavailable("parameter RPC session timed out"))),
        };
        guard.completed = true;
        result
    }
    async fn step(&self, query: request::ParameterQuery) -> Result<Value> {
        match self.state.profile {
            Profile::Parameters => {
                self.runtime
                    .parameter_step(&self.url, query, self.state.deadline)
                    .await
            }
            Profile::Discovery => {
                self.runtime
                    .discovery_step(&self.url, query, self.state.deadline)
                    .await
            }
            Profile::SolHop => {
                self.runtime
                    .sol_hop_step(&self.url, query, self.state.deadline)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests;

impl<T: ReadHttpTransport + 'static> RpcSender for Sender<T> {
    fn send<'a, 'f>(
        &'a self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, ClientError>> + Send + 'f>>
    where
        'a: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            let result = self.request(request, params).await;
            if let Err(error) = &result {
                self.state.fail(error);
            }
            result.map_err(|error| ClientErrorKind::Custom(error.to_string()).into())
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats {
            request_count: self.state.calls.load(Ordering::Relaxed),
            ..RpcTransportStats::default()
        }
    }
    fn url(&self) -> String {
        self.url.clone()
    }
}
