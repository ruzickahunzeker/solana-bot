use super::*;
use crate::adapters::rpc::pump_native_probe::{PumpNativeProbePacket, PumpNativeProbeReply};
use anyhow::Context;

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn pump_native_probe(
        &self,
        url: &str,
        packet: PumpNativeProbePacket,
    ) -> Result<PumpNativeProbeReply> {
        let deadline = packet
            .deadline()
            .min(Instant::now() + Duration::from_secs(2));
        ensure!(
            Instant::now() < deadline,
            "native Pump probe expired before admission"
        );
        let _permit = self.aggregator_simulation_slot(url)?;
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| anyhow::anyhow!("native Pump probe endpoint is not registered"))?;
        let minimum = packet.minimum();
        let binding = packet.binding();
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::PumpNativeProbe { id, packet };
        let maximum = request.max_body();
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .read(Class::AggregatorSimulation, index, request, deadline)
                .await
                .map_err(|_| anyhow::anyhow!("native Pump probe RPC unavailable"))?;
            ensure!(
                reply.status.is_success(),
                "native Pump probe HTTP unavailable"
            );
            ensure!(
                reply.body.len() <= maximum,
                "native Pump probe body exceeds limit"
            );
            let envelope: Envelope = serde_json::from_slice(&reply.body)
                .map_err(|_| anyhow::anyhow!("native Pump probe JSON-RPC response is invalid"))?;
            ensure!(
                envelope.jsonrpc == "2.0" && envelope.id == id && envelope.error.is_none(),
                "native Pump probe RPC identity or outcome is invalid"
            );
            let response = envelope
                .result
                .context("native Pump probe RPC omitted result")?;
            ensure!(
                response
                    .get("context")
                    .and_then(|context| context.get("slot"))
                    .and_then(Value::as_u64)
                    .is_some_and(|slot| slot >= minimum),
                "native Pump probe context precedes its evidence"
            );
            ensure!(
                response.get("value").and_then(|value| value.get("err")) == Some(&Value::Null),
                "native Pump probe requires explicit success"
            );
            ensure!(
                response
                    .get("value")
                    .and_then(|value| value.get("accounts"))
                    .and_then(Value::as_array)
                    .is_some_and(|accounts| accounts.len()
                        == crate::adapters::rpc::pump_native_probe::EVIDENCE_ACCOUNTS),
                "native Pump probe returned account count differs"
            );
            ensure!(
                Instant::now() < deadline,
                "native Pump probe deadline expired after decoding"
            );
            binding.complete(response)
        })
        .await
        .map_err(|_| anyhow::anyhow!("native Pump probe timed out"))?
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Value>,
    error: Option<Value>,
}
