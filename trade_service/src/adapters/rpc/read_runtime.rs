use super::read_http::{HttpReply, ReadHttpTransport, ReadRequest, ReqwestReadTransport};
use crate::domain::blockhash::Blockhash;
use anyhow::{bail, ensure, Result};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const HASH_NODE_DEADLINE: Duration = Duration::from_millis(500);
const HASH_TOTAL_DEADLINE: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy)]
enum Class {
    Blockhash,
    Observation,
    Readiness,
    NativeBalances,
    QuoteBalance,
    PreparationBalance,
    Parameters,
    Discovery,
    SolHop,
    AggregatorAccounts,
    AggregatorSimulation,
    WsolQuote,
}
impl Class {
    fn index(self) -> usize {
        match self {
            Self::Blockhash => 0,
            Self::Observation => 1,
            Self::Readiness => 2,
            Self::NativeBalances => 3,
            Self::QuoteBalance => 4,
            Self::PreparationBalance => 5,
            Self::Parameters => 6,
            Self::Discovery => 7,
            Self::SolHop => 8,
            Self::AggregatorAccounts => 9,
            Self::AggregatorSimulation => 10,
            Self::WsolQuote => 11,
        }
    }
    fn interval(self) -> Duration {
        match self {
            Self::Blockhash => Duration::from_millis(100),
            Self::Observation => Duration::from_secs(1),
            Self::Readiness => Duration::from_millis(250),
            Self::NativeBalances => Duration::from_millis(250),
            Self::QuoteBalance => Duration::from_millis(250),
            Self::PreparationBalance => Duration::from_millis(250),
            Self::Parameters => Duration::from_millis(250),
            Self::Discovery => Duration::from_millis(100),
            Self::SolHop => Duration::from_millis(100),
            Self::AggregatorAccounts => Duration::from_millis(100),
            Self::AggregatorSimulation => Duration::from_millis(100),
            Self::WsolQuote => Duration::from_millis(100),
        }
    }
}

struct Budget {
    next_start: [Instant; 12],
    cooldowns: Vec<Option<Instant>>,
}

/// Fixed read profiles only; there is no arbitrary RPC/send capability.
pub(crate) struct SharedReadRuntime<T = ReqwestReadTransport> {
    urls: Vec<String>,
    endpoints: Vec<Option<Url>>,
    transport: T,
    slots: [Arc<Semaphore>; 12],
    budget: Mutex<Budget>,
    hash_id: AtomicU64,
}

impl SharedReadRuntime {
    pub(crate) fn new(urls: &[String]) -> Result<Self> {
        Ok(Self::with_transport(urls, ReqwestReadTransport::new()?))
    }
}

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    #[cfg(test)]
    pub(crate) fn with_test_transport(urls: &[String], transport: T) -> Self {
        Self::with_transport(urls, transport)
    }
    #[cfg(test)]
    pub(crate) fn test_transport(&self) -> &T {
        &self.transport
    }
    fn with_transport(urls: &[String], transport: T) -> Self {
        let mut unique = Vec::new();
        let mut endpoints = Vec::new();
        for raw in urls {
            // Canonical URL aliases share cooldown, while distinct credentials remain distinct.
            let endpoint = Url::parse(raw)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
            if endpoint
                .as_ref()
                .is_some_and(|_| endpoints.contains(&endpoint))
                || unique.contains(raw)
            {
                continue;
            }
            unique.push(raw.clone());
            endpoints.push(endpoint);
        }
        Self {
            budget: Mutex::new(Budget {
                next_start: [Instant::now(); 12],
                cooldowns: vec![None; unique.len()],
            }),
            urls: unique,
            endpoints,
            transport,
            slots: std::array::from_fn(|index| {
                Arc::new(Semaphore::new(if index == Class::Discovery.index() {
                    2
                } else {
                    1
                }))
            }),
            hash_id: AtomicU64::new(1),
        }
    }

    pub(super) fn urls(&self) -> &[String] {
        &self.urls
    }

    fn available(&self, index: usize, now: Instant) -> Result<bool> {
        let state = self
            .budget
            .lock()
            .map_err(|_| anyhow::anyhow!("read RPC budget unavailable"))?;
        Ok(self.endpoints[index].is_some()
            && state.cooldowns[index].is_none_or(|until| now >= until))
    }

    async fn read(
        &self,
        class: Class,
        index: usize,
        request: ReadRequest,
        deadline: Instant,
    ) -> Result<HttpReply> {
        let url = self.start_read(class, index, deadline).await?;
        self.finish_read(class, index, url, request, deadline).await
    }

    async fn start_read(&self, class: Class, index: usize, deadline: Instant) -> Result<&Url> {
        ensure!(
            Instant::now() < deadline,
            "read RPC deadline expired before admission"
        );
        let url = self.endpoints[index]
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("read RPC endpoint unavailable"))?;
        loop {
            let until = self
                .budget
                .lock()
                .map_err(|_| anyhow::anyhow!("read RPC budget unavailable"))?
                .next_start[class.index()];
            ensure!(until < deadline, "read RPC pacing exceeds deadline");
            tokio::time::sleep_until(tokio::time::Instant::from_std(until)).await;
            let mut state = self
                .budget
                .lock()
                .map_err(|_| anyhow::anyhow!("read RPC budget unavailable"))?;
            let now = Instant::now();
            ensure!(now < deadline, "read RPC deadline expired before admission");
            // Two discovery branches may wake together; only one can claim this start.
            if now < state.next_start[class.index()] {
                continue;
            }
            ensure!(
                state.cooldowns[index].is_none_or(|until| now >= until),
                "read RPC endpoint cooling down or unavailable"
            );
            state.next_start[class.index()] = now + class.interval();
            break;
        }
        Ok(url)
    }

    async fn finish_read(
        &self,
        class: Class,
        index: usize,
        url: &Url,
        request: ReadRequest,
        deadline: Instant,
    ) -> Result<HttpReply> {
        ensure!(
            Instant::now() < deadline,
            "read RPC deadline expired before I/O"
        );
        let reply = self.transport.fetch(url, &request).await?;
        if reply.status == StatusCode::TOO_MANY_REQUESTS {
            let until =
                Instant::now() + Duration::from_secs(reply.retry_after.unwrap_or(30).clamp(1, 300));
            let mut state = self
                .budget
                .lock()
                .map_err(|_| anyhow::anyhow!("read RPC budget unavailable"))?;
            state.cooldowns[index] =
                Some(state.cooldowns[index].map_or(until, |old| old.max(until)));
            tracing::warn!(
                rpc_lane = index + 1,
                operation = match class {
                    Class::Blockhash => "blockhash",
                    Class::Observation => "observation",
                    Class::Readiness => "readiness",
                    Class::NativeBalances => "native balances",
                    Class::QuoteBalance => "quote balance",
                    Class::PreparationBalance => "preparation balance",
                    Class::Parameters => "protocol parameters",
                    Class::Discovery => "pool discovery",
                    Class::SolHop => "SOL hop discovery",
                    Class::AggregatorAccounts => "aggregator accounts",
                    Class::AggregatorSimulation => "aggregator simulation",
                    Class::WsolQuote => "WSOL quote",
                },
                "read RPC rate limited; shared endpoint cooling down"
            );
        }
        // A response can become ready while executor polling is delayed past the timeout.
        ensure!(
            Instant::now() < deadline,
            "read RPC deadline expired after response"
        );
        Ok(reply)
    }

    pub(super) async fn signature_status(
        &self,
        url: &Url,
        id: u64,
        signature: &str,
    ) -> Result<HttpReply> {
        let deadline = Instant::now() + Duration::from_secs(2);
        tokio::time::timeout(Duration::from_secs(2), async {
            let index = self
                .endpoints
                .iter()
                .position(|endpoint| endpoint.as_ref() == Some(url))
                .ok_or_else(|| anyhow::anyhow!("observation RPC endpoint not registered"))?;
            let _slot = self.slots[Class::Observation.index()]
                .clone()
                .try_acquire_owned()
                .map_err(|_| {
                    anyhow::anyhow!("observation quota busy; original transaction was not retried")
                })?;
            self.read(
                Class::Observation,
                index,
                ReadRequest::SignatureStatus {
                    id,
                    signature: signature.into(),
                },
                deadline,
            )
            .await
        })
        .await
        .map_err(|_| anyhow::anyhow!("observation RPC lookup timed out"))?
    }

    async fn hash_slot(&self, class: Class) -> Result<OwnedSemaphorePermit> {
        let slot = self.slots[class.index()].clone();
        if matches!(class, Class::Readiness) {
            return slot
                .try_acquire_owned()
                .map_err(|_| anyhow::anyhow!("readiness RPC budget busy"));
        }
        slot.acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("blockhash RPC budget unavailable"))
    }

    pub(crate) async fn latest_blockhash(&self) -> Result<Blockhash> {
        self.read_hash(
            Class::Blockhash,
            &(0..self.endpoints.len()).collect::<Vec<_>>(),
        )
        .await
    }

    pub(crate) async fn readiness_ping(&self, execution_urls: &[String]) -> bool {
        self.read_hash(Class::Readiness, &self.candidate_indices(execution_urls))
            .await
            .is_ok()
    }

    fn candidate_indices(&self, execution_urls: &[String]) -> Vec<usize> {
        let mut candidates = Vec::new();
        for raw in execution_urls {
            let Some(index) = Url::parse(raw).ok().and_then(|url| {
                self.endpoints
                    .iter()
                    .position(|entry| entry.as_ref() == Some(&url))
            }) else {
                continue;
            };
            if !candidates.contains(&index) {
                candidates.push(index);
            }
        }
        candidates
    }

    async fn read_hash(&self, class: Class, candidates: &[usize]) -> Result<Blockhash> {
        let deadline = Instant::now() + HASH_TOTAL_DEADLINE;
        let result = tokio::time::timeout(HASH_TOTAL_DEADLINE, async {
            // Readiness must not enqueue on the critical refresher's reserved FIFO.
            let _slot = self.hash_slot(class).await?;
            for &index in candidates {
                if Instant::now() >= deadline {
                    break;
                }
                if !self.available(index, Instant::now())? {
                    continue;
                }
                let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
                let node_deadline = (Instant::now() + HASH_NODE_DEADLINE).min(deadline);
                let reply = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(node_deadline),
                    self.read(
                        class,
                        index,
                        ReadRequest::LatestBlockhash { id },
                        node_deadline,
                    ),
                )
                .await;
                if let Ok(Ok(reply)) = reply {
                    if reply.status.is_success() {
                        if let Ok(hash) = parse_blockhash(id, &reply.body) {
                            return Ok(hash);
                        }
                    }
                }
            }
            bail!("all RPC lanes failed to refresh blockhash")
        })
        .await;
        match result {
            Ok(result) => result,
            Err(_) => bail!("all RPC lanes failed to refresh blockhash"),
        }
    }
}

#[derive(Deserialize)]
struct HashEnvelope {
    jsonrpc: String,
    id: u64,
    result: Option<HashResult>,
    error: Option<Value>,
}
#[derive(Deserialize)]
struct HashResult {
    context: HashContext,
    value: HashValue,
}
#[derive(Deserialize)]
struct HashContext {
    slot: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HashValue {
    blockhash: String,
    last_valid_block_height: u64,
}

fn parse_blockhash(id: u64, bytes: &[u8]) -> Result<Blockhash> {
    ensure!(
        bytes.len() <= super::read_http::MAX_BODY,
        "blockhash RPC body exceeds limit"
    );
    let envelope: HashEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("invalid blockhash JSON-RPC response"))?;
    ensure!(
        envelope.jsonrpc == "2.0" && envelope.id == id && envelope.error.is_none(),
        "blockhash RPC identity or result invalid"
    );
    let result = envelope
        .result
        .ok_or_else(|| anyhow::anyhow!("blockhash RPC omitted result"))?;
    // Decode the same required response fields without leaking SDK types into the read port.
    let _context_slot = result.context.slot;
    let _last_valid_height = result.value.last_valid_block_height;
    ensure!(
        result.value.blockhash.len() <= 44,
        "invalid RPC blockhash length"
    );
    let mut hash = [0u8; 32];
    let length = bs58::decode(result.value.blockhash)
        .onto(&mut hash[..])
        .map_err(|_| anyhow::anyhow!("invalid RPC blockhash"))?;
    ensure!(length == hash.len(), "invalid RPC blockhash length");
    Ok(Blockhash::from_bytes(hash))
}

#[cfg(test)]
mod tests;

pub(crate) mod aggregator_accounts;
mod aggregator_simulation;
mod damm_quotes;
mod damm_simulation;
mod native_balances;
mod quote_balance;
mod quote_status;
mod wsol_quotes;
mod wsol_simulation;
mod wsol_status;
pub(crate) use quote_balance::preparation_scan;
#[cfg(test)]
pub(crate) use quote_status::tests as quote_status_fixture;
mod parameters;
mod preparation_balance;
