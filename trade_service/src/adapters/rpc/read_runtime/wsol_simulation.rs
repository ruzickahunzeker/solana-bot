use super::*;
use crate::sdk_client::wsol_conversion::simulation::WsolSimulationPacket;
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn wsol_simulation(
        &self,
        url: &str,
        packet: WsolSimulationPacket,
    ) -> Result<RpcSimulateTransactionResult> {
        let deadline = Instant::now() + Duration::from_secs(2);
        // WSOL and aggregator simulations share the existing single simulation permit.
        let _permit = self.aggregator_simulation_slot(url)?;
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| anyhow::anyhow!("WSOL simulation endpoint not registered"))?;
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::WsolSimulation { id, packet };
        let maximum = request.max_body();
        let result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .read(Class::AggregatorSimulation, index, request, deadline)
                .await
                .map_err(|_| anyhow::anyhow!("WSOL simulation RPC unavailable"))?;
            ensure!(
                reply.status.is_success(),
                "WSOL simulation HTTP unavailable"
            );
            ensure!(
                reply.body.len() <= maximum,
                "WSOL simulation body exceeds limit"
            );
            let envelope: Envelope = serde_json::from_slice(&reply.body)
                .map_err(|_| anyhow::anyhow!("invalid WSOL simulation JSON-RPC response"))?;
            ensure!(
                envelope.jsonrpc == "2.0" && envelope.id == id && envelope.error.is_none(),
                "WSOL simulation RPC identity or result invalid"
            );
            let response = envelope
                .result
                .ok_or_else(|| anyhow::anyhow!("WSOL simulation RPC omitted result"))?;
            ensure!(
                response.context.slot > 0,
                "WSOL simulation RPC context is invalid"
            );
            ensure!(
                response
                    .value
                    .as_object()
                    .is_some_and(|value| value.contains_key("err")),
                "WSOL simulation RPC omitted explicit error outcome"
            );
            serde_json::from_value::<RpcSimulateTransactionResult>(response.value)
                .map_err(|_| anyhow::anyhow!("invalid WSOL simulation result"))
        })
        .await
        .map_err(|_| anyhow::anyhow!("WSOL simulation RPC timed out"))??;
        ensure!(
            Instant::now() < deadline,
            "WSOL simulation deadline expired after decoding"
        );
        Ok(result)
    }
}
#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Response<Value>>,
    error: Option<Value>,
}
