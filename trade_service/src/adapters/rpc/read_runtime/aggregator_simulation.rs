use super::*;
use crate::adapters::rpc::{
    aggregator_simulation::UnsignedAggregatorSimulation,
    parameter_reads::{invalid, unavailable},
};

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) fn aggregator_simulation_slot(&self, url: &str) -> Result<OwnedSemaphorePermit> {
        let index = *self
            .candidate_indices(&[url.to_owned()])
            .first()
            .ok_or_else(|| unavailable("aggregator simulation endpoint is not registered"))?;
        ensure!(
            self.available(index, Instant::now())?,
            unavailable("aggregator simulation endpoint cooling down or unavailable")
        );
        self.slots[Class::AggregatorSimulation.index()]
            .clone()
            .try_acquire_owned()
            .map_err(|_| unavailable("aggregator simulation quota busy"))
    }
    pub(crate) async fn aggregator_simulation_step(
        &self,
        url: &str,
        plan: &UnsignedAggregatorSimulation,
        minimum_slot: Option<u64>,
        total: Instant,
    ) -> Result<Value> {
        let index = *self
            .candidate_indices(&[url.to_owned()])
            .first()
            .ok_or_else(|| unavailable("aggregator simulation endpoint is not registered"))?;
        ensure!(
            self.available(index, Instant::now())?,
            unavailable("aggregator simulation endpoint cooling down or unavailable")
        );
        let deadline = total.min(Instant::now() + Duration::from_secs(2));
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::AggregatorSimulation {
            id,
            plan: plan.clone(),
            minimum_slot,
        };
        let maximum = request.max_body();
        let reply = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.read(Class::AggregatorSimulation, index, request, deadline),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) if error.is::<super::super::read_http::ReadBodyLimit>() => {
                return Err(invalid("aggregator simulation body exceeds limit"))
            }
            _ => return Err(unavailable("aggregator simulation read unavailable")),
        };
        ensure!(
            reply.status.is_success(),
            unavailable("aggregator simulation HTTP unavailable")
        );
        ensure!(
            reply.body.len() <= maximum,
            invalid("aggregator simulation body exceeds limit")
        );
        let envelope: Envelope = serde_json::from_slice(&reply.body)
            .map_err(|_| invalid("aggregator simulation envelope is invalid"))?;
        ensure!(
            envelope.jsonrpc == "2.0" && envelope.id == id,
            invalid("aggregator simulation identity is invalid")
        );
        let value = match (envelope.result, envelope.error) {
            (Some(value), None) => value,
            (None, Some(_)) => return Err(unavailable("aggregator simulation RPC error")),
            _ => return Err(invalid("aggregator simulation result is invalid")),
        };
        ensure!(
            Instant::now() < deadline,
            unavailable("aggregator simulation deadline expired")
        );
        Ok(value)
    }
}
#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Value>,
    error: Option<Fault>,
}
#[derive(Deserialize)]
struct Fault {
    #[serde(rename = "code")]
    _code: i64,
    #[serde(rename = "message")]
    _message: String,
}
