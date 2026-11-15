use anyhow::{bail, ensure, Result};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use solana_client::rpc_response::Response as RpcResponse;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::{TransactionConfirmationStatus, TransactionStatus};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use crate::{
    application::ports::reconciliation::SignatureStatusReader,
    domain::reconciliation::ChainObservation,
};

pub(crate) use super::read_http::HttpReply;
use super::{read_http::MAX_BODY, read_runtime::SharedReadRuntime};
pub(crate) trait StatusTransport: Send + Sync {
    fn fetch(
        &self,
        url: &Url,
        id: u64,
        signature: &str,
    ) -> impl Future<Output = Result<HttpReply>> + Send;
}
pub(crate) struct ReqwestStatusTransport {
    runtime: Arc<SharedReadRuntime>,
}
impl StatusTransport for ReqwestStatusTransport {
    async fn fetch(&self, url: &Url, id: u64, signature: &str) -> Result<HttpReply> {
        self.runtime.signature_status(url, id, signature).await
    }
}
#[cfg(test)]
fn request(id: u64, signature: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"getSignatureStatuses","params":[[signature],{"searchTransactionHistory":true}]})
}

struct GateState {
    in_flight: bool,
    next_allowed: Instant,
    next_lane: usize,
    cooldowns: Vec<Option<Instant>>,
}
struct RequestGate {
    state: Arc<Mutex<GateState>>,
    interval: Duration,
    capacity: Arc<tokio::sync::Semaphore>,
}
impl RequestGate {
    fn new(lanes: usize, interval: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(GateState {
                in_flight: false,
                next_allowed: Instant::now(),
                next_lane: 0,
                cooldowns: vec![None; lanes],
            })),
            interval,
            capacity: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
    #[cfg(test)]
    fn begin(&self, now: Instant) -> Result<RequestPermit> {
        self.begin_inner(now, None)
    }

    fn foreground(&self, now: Instant) -> Result<RequestPermit> {
        let slot = self.capacity.clone().try_acquire_owned().map_err(|_| {
            anyhow::anyhow!("observation quota busy; original transaction was not retried")
        })?;
        self.begin_inner(now, Some(slot))
    }

    async fn background(&self) -> Result<RequestPermit> {
        // FIFO ownership covers the spacing wait, so foreground probes cannot steal its next slot.
        let slot = self
            .capacity
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("observation quota unavailable"))?;
        let until = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("observation quota unavailable"))?
            .next_allowed;
        tokio::time::sleep_until(tokio::time::Instant::from_std(until)).await;
        self.begin_inner(Instant::now(), Some(slot))
    }

    fn begin_inner(
        &self,
        now: Instant,
        slot: Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<RequestPermit> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("observation quota unavailable"))?;
        ensure!(
            !state.in_flight && now >= state.next_allowed,
            "observation quota busy; original transaction was not retried"
        );
        let index = (0..state.cooldowns.len())
            .map(|offset| (state.next_lane + offset) % state.cooldowns.len())
            .find(|index| state.cooldowns[*index].is_none_or(|until| now >= until))
            .ok_or_else(|| anyhow::anyhow!("observation RPC endpoints cooling down"))?;
        state.in_flight = true;
        state.next_allowed = now + self.interval;
        state.next_lane = (index + 1) % state.cooldowns.len();
        Ok(RequestPermit {
            state: self.state.clone(),
            index,
            _slot: slot,
        })
    }
}
struct RequestPermit {
    state: Arc<Mutex<GateState>>,
    index: usize,
    _slot: Option<tokio::sync::OwnedSemaphorePermit>,
}
impl RequestPermit {
    fn rate_limited(&self, seconds: Option<u64>) {
        if let Ok(mut state) = self.state.lock() {
            state.cooldowns[self.index] =
                Some(Instant::now() + Duration::from_secs(seconds.unwrap_or(30).clamp(1, 300)));
        }
    }
}
impl Drop for RequestPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.in_flight = false;
        }
    }
}

pub(crate) struct SingleShotStatusReader<T = ReqwestStatusTransport> {
    transport: T,
    urls: Vec<Url>,
    gate: RequestGate,
    request_id: AtomicU64,
}
impl SingleShotStatusReader {
    #[cfg(test)]
    pub(crate) fn new(urls: &[String], interval: Duration) -> Result<Self> {
        Self::with_runtime(Arc::new(SharedReadRuntime::new(urls)?), interval)
    }

    pub(crate) fn with_runtime(
        runtime: Arc<SharedReadRuntime>,
        interval: Duration,
    ) -> Result<Self> {
        ensure!(
            (Duration::from_secs(1)..=Duration::from_secs(60)).contains(&interval),
            "invalid observation request interval"
        );
        Self::with_transport(
            runtime.urls(),
            interval,
            ReqwestStatusTransport {
                runtime: runtime.clone(),
            },
        )
    }
}
impl<T: StatusTransport> SingleShotStatusReader<T> {
    fn with_transport(urls: &[String], interval: Duration, transport: T) -> Result<Self> {
        let mut selected = Vec::new();
        for raw in urls {
            let url =
                Url::parse(raw).map_err(|_| anyhow::anyhow!("invalid observation RPC URL"))?;
            ensure!(
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none(),
                "invalid observation RPC URL"
            );
            if !selected.contains(&url) {
                selected.push(url);
            }
            if selected.len() == 2 {
                break;
            }
        }
        ensure!(
            !selected.is_empty(),
            "observation requires at least one HTTP RPC endpoint"
        );
        Ok(Self {
            gate: RequestGate::new(selected.len(), interval),
            transport,
            urls: selected,
            request_id: AtomicU64::new(1),
        })
    }
}
impl<T: StatusTransport> SingleShotStatusReader<T> {
    pub(crate) async fn lookup_foreground(
        &self,
        signature: &str,
    ) -> Result<Option<ChainObservation>> {
        self.lookup_once(signature, true).await
    }
    async fn lookup_once(
        &self,
        signature: &str,
        foreground: bool,
    ) -> Result<Option<ChainObservation>> {
        signature
            .parse::<Signature>()
            .map_err(|_| anyhow::anyhow!("invalid persisted transaction signature"))?;
        let permit = if foreground {
            self.gate.foreground(Instant::now())?
        } else {
            self.gate.background().await?
        };
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        let reply = tokio::time::timeout(
            Duration::from_secs(2),
            self.transport
                .fetch(&self.urls[permit.index], id, signature),
        )
        .await
        .map_err(|_| anyhow::anyhow!("observation RPC lookup timed out"))??;
        if reply.status == StatusCode::TOO_MANY_REQUESTS {
            permit.rate_limited(reply.retry_after);
            bail!("observation RPC rate limited (429); endpoint cooling down");
        }
        ensure!(
            reply.status.is_success(),
            "observation RPC HTTP error {}",
            reply.status.as_u16()
        );
        parse_reply(id, signature, &reply.body)
    }
}

impl<T: StatusTransport> SignatureStatusReader for SingleShotStatusReader<T> {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        self.lookup_once(signature, false).await
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Value>,
    error: Option<Value>,
}
fn parse_reply(id: u64, signature: &str, bytes: &[u8]) -> Result<Option<ChainObservation>> {
    ensure!(
        bytes.len() <= MAX_BODY,
        "observation RPC body exceeds limit"
    );
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("invalid observation JSON-RPC response"))?;
    ensure!(
        envelope.jsonrpc == "2.0"
            && envelope.id == id
            && !(envelope.result.is_some() && envelope.error.is_some()),
        "observation JSON-RPC identity mismatch"
    );
    if let Some(error) = envelope.error {
        bail!(
            "observation JSON-RPC error {}",
            error.get("code").and_then(Value::as_i64).unwrap_or(0)
        );
    }
    let response: RpcResponse<Vec<Option<TransactionStatus>>> = serde_json::from_value(
        envelope
            .result
            .ok_or_else(|| anyhow::anyhow!("observation RPC omitted result"))?,
    )
    .map_err(|_| anyhow::anyhow!("invalid observation signature status result"))?;
    ensure!(
        response.value.len() == 1,
        "observation RPC returned invalid result count"
    );
    let Some(status) = response.value.into_iter().next().flatten() else {
        return Ok(None);
    };
    ensure!(
        status.slot > 0 && response.context.slot >= status.slot,
        "observation RPC slot evidence inconsistent"
    );
    if status.confirmation_status == Some(TransactionConfirmationStatus::Finalized) {
        ensure!(
            status.confirmations.is_none(),
            "observation RPC finalized confirmations inconsistent"
        );
    }
    Ok(Some(super::signature_status::observation(
        signature, status,
    )?))
}

#[cfg(test)]
mod tests;
