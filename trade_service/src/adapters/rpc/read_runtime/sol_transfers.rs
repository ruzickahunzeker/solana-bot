use super::*;
use crate::adapters::rpc::sol_transfer_reads::SolTransferQuery;

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) fn sol_transfer_slot(&self, url: &str) -> Result<OwnedSemaphorePermit> {
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| {
                anyhow::anyhow!("SOL transfer preparation endpoint is not registered")
            })?;
        ensure!(
            self.available(index, Instant::now())?,
            "SOL transfer preparation endpoint cooling down or unavailable"
        );
        // Reuse the existing balance/hash/fee/rent profile, not a thirteenth resource class.
        self.slots[Class::WsolQuote.index()]
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("SOL transfer preparation quota busy"))
    }

    pub(crate) async fn sol_transfer_step(
        &self,
        url: &str,
        query: SolTransferQuery,
        deadline: Instant,
    ) -> Result<Value> {
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| {
                anyhow::anyhow!("SOL transfer preparation endpoint is not registered")
            })?;
        ensure!(
            self.available(index, Instant::now())?,
            "SOL transfer preparation endpoint cooling down or unavailable"
        );
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::SolTransferPreparation { id, query };
        let maximum = request.max_body();
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .read(Class::WsolQuote, index, request, deadline)
                .await
                .map_err(|_| anyhow::anyhow!("SOL transfer preparation RPC unavailable"))?;
            ensure!(
                reply.status.is_success(),
                "SOL transfer preparation HTTP unavailable"
            );
            ensure!(
                reply.body.len() <= maximum,
                "SOL transfer preparation body exceeds limit"
            );
            let envelope: Envelope = serde_json::from_slice(&reply.body).map_err(|_| {
                anyhow::anyhow!("invalid SOL transfer preparation JSON-RPC response")
            })?;
            ensure!(
                envelope.jsonrpc == "2.0" && envelope.id == id && envelope.error.is_none(),
                "SOL transfer preparation RPC identity or result invalid"
            );
            let value = envelope
                .result
                .ok_or_else(|| anyhow::anyhow!("SOL transfer preparation RPC omitted result"))?;
            ensure!(
                Instant::now() < deadline,
                "SOL transfer preparation deadline expired after parsing"
            );
            Ok(value)
        })
        .await
        .map_err(|_| anyhow::anyhow!("SOL transfer preparation timed out"))?
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Value>,
    error: Option<Value>,
}
