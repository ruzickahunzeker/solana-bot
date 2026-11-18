use super::*;
use crate::adapters::rpc::pump_native_final::{PumpNativeFinalPacket, PumpNativeFinalReply};
use anyhow::Context;

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn pump_native_final(
        &self,
        url: &str,
        packet: PumpNativeFinalPacket,
    ) -> Result<PumpNativeFinalReply> {
        let deadline = packet
            .deadline()
            .min(Instant::now() + Duration::from_secs(2));
        ensure!(
            Instant::now() < deadline,
            "native Pump final expired before admission"
        );
        let _permit = self.aggregator_simulation_slot(url)?;
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| anyhow::anyhow!("native Pump final endpoint is not registered"))?;
        let minimum = packet.minimum();
        let count = packet.account_count();
        let binding = packet.binding();
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::PumpNativeFinal { id, packet };
        let maximum = request.max_body();
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .read(Class::AggregatorSimulation, index, request, deadline)
                .await
                .map_err(|_| anyhow::anyhow!("native Pump final RPC unavailable"))?;
            ensure!(
                reply.status.is_success(),
                "native Pump final HTTP unavailable"
            );
            ensure!(
                reply.body.len() <= maximum,
                "native Pump final body exceeds limit"
            );
            let envelope: Envelope = serde_json::from_slice(&reply.body)
                .map_err(|_| anyhow::anyhow!("native Pump final JSON-RPC response is invalid"))?;
            ensure!(
                envelope.jsonrpc == "2.0" && envelope.id == id && envelope.error.is_none(),
                "native Pump final RPC identity or outcome is invalid"
            );
            let response = envelope
                .result
                .context("native Pump final RPC omitted result")?;
            ensure!(
                response
                    .get("context")
                    .and_then(|value| value.get("slot"))
                    .and_then(Value::as_u64)
                    .is_some_and(|slot| slot >= minimum),
                "native Pump final context precedes its evidence"
            );
            ensure!(
                response.get("value").and_then(|value| value.get("err")) == Some(&Value::Null),
                "native Pump final requires explicit success"
            );
            ensure!(
                response
                    .get("value")
                    .and_then(|value| value.get("accounts"))
                    .and_then(Value::as_array)
                    .is_some_and(|accounts| accounts.len() == count),
                "native Pump final returned account count differs"
            );
            ensure!(
                Instant::now() < deadline,
                "native Pump final expired after response decoding"
            );
            binding.complete(response)
        })
        .await
        .map_err(|_| anyhow::anyhow!("native Pump final timed out"))?
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Value>,
    error: Option<Value>,
}
