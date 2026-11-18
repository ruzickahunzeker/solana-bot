use super::*;
use crate::adapters::rpc::damm_sell_simulation::{
    shared::DammSimulationPacket, DammSimulationResult,
};
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn damm_candidate_simulation(
        &self,
        url: &str,
        packet: DammSimulationPacket,
    ) -> Result<DammSimulationResult> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let _permit = self.aggregator_simulation_slot(url)?;
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| anyhow::anyhow!("DAMM simulation endpoint not registered"))?;
        let minimum = packet.minimum();
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::DammCandidateSimulation { id, packet };
        let maximum = request.max_body();
        let result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .read(Class::AggregatorSimulation, index, request, deadline)
                .await
                .map_err(|_| anyhow::anyhow!("DAMM unsigned simulation RPC unavailable"))?;
            ensure!(
                reply.status.is_success(),
                "DAMM unsigned simulation HTTP unavailable"
            );
            ensure!(
                reply.body.len() <= maximum,
                "DAMM unsigned simulation body exceeds limit"
            );
            let envelope: Envelope = serde_json::from_slice(&reply.body)
                .map_err(|_| anyhow::anyhow!("invalid DAMM simulation JSON-RPC response"))?;
            ensure!(
                envelope.jsonrpc == "2.0" && envelope.id == id && envelope.error.is_none(),
                "DAMM simulation RPC identity or result invalid"
            );
            let response = envelope
                .result
                .ok_or_else(|| anyhow::anyhow!("DAMM simulation RPC omitted result"))?;
            ensure!(
                response.context.slot > 0 && response.context.slot >= minimum,
                "DAMM unsigned simulation returned a zero slot or precedes its output quote"
            );
            ensure!(
                response
                    .value
                    .as_object()
                    .is_some_and(|v| v.contains_key("err")),
                "DAMM simulation RPC omitted explicit error outcome"
            );
            let result: RpcSimulateTransactionResult = serde_json::from_value(response.value)
                .map_err(|_| anyhow::anyhow!("invalid DAMM simulation result"))?;
            ensure!(
                result.replacement_blockhash.is_none()
                    && result
                        .loaded_addresses
                        .as_ref()
                        .is_none_or(|a| a.writable.is_empty() && a.readonly.is_empty()),
                "DAMM unsigned simulation changed the fixed candidate context"
            );
            Ok(DammSimulationResult {
                error: result.err.map(|err| format!("{err:?}")),
            })
        })
        .await
        .map_err(|_| anyhow::anyhow!("DAMM unsigned simulation timed out"))??;
        ensure!(
            Instant::now() < deadline,
            "DAMM unsigned simulation timed out"
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
