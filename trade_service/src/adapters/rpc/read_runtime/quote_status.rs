use super::*;
use crate::domain::reconciliation::ChainObservation;
use solana_client::rpc_response::Response;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::TransactionStatus;

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn quote_signature_status(
        &self,
        urls: &[String],
        signature: &str,
    ) -> Result<Option<ChainObservation>> {
        use anyhow::Context;
        let parsed = signature
            .parse::<Signature>()
            .context("invalid persisted transaction signature")?;
        ensure!(!urls.is_empty(), "no status RPC configured");
        let deadline = Instant::now() + Duration::from_secs(3);
        let _permit = self.slots[Class::Observation.index()]
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                anyhow::anyhow!(
                    "signature status RPC quota busy; original transaction was not retried"
                )
            })?;
        let result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let mut missing = false;
            let mut last_error = "no status RPC configured".to_string();
            for url in urls.iter().take(2) {
                if Instant::now() >= deadline {
                    last_error = "signature status RPC timed out".into();
                    break;
                }
                match self.quote_status_node(url, &parsed, deadline).await {
                    Ok(response) if response.value.len() == 1 => {
                        if let Some(status) = response.value.into_iter().next().flatten() {
                            return Ok(Some(super::super::signature_status::observation(
                                signature, status,
                            )?));
                        }
                        missing = true;
                    }
                    Ok(_) => {
                        last_error = "signature status RPC returned an invalid result count".into()
                    }
                    Err(error) => last_error = error.to_string(),
                }
            }
            if missing {
                return Ok(None);
            }
            bail!("{last_error}")
        })
        .await
        .map_err(|_| anyhow::anyhow!("signature status RPC timed out"))??;
        ensure!(Instant::now() < deadline, "signature status RPC timed out");
        Ok(result)
    }

    async fn quote_status_node(
        &self,
        url: &str,
        signature: &Signature,
        operation_deadline: Instant,
    ) -> Result<Response<Vec<Option<TransactionStatus>>>> {
        let index = Url::parse(url)
            .ok()
            .and_then(|url| {
                self.endpoints
                    .iter()
                    .position(|candidate| candidate.as_ref() == Some(&url))
            })
            .ok_or_else(|| anyhow::anyhow!("signature status RPC endpoint unavailable"))?;
        ensure!(
            self.available(index, Instant::now())?,
            "signature status RPC endpoint cooling down or unavailable"
        );
        let url = self
            .start_read(Class::Observation, index, operation_deadline)
            .await
            .map_err(|_| anyhow::anyhow!("signature status RPC admission unavailable"))?;
        // Pacing is shared, but must not consume the original backup node's one-second I/O budget.
        let deadline = (Instant::now() + Duration::from_secs(1)).min(operation_deadline);
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .finish_read(
                    Class::Observation,
                    index,
                    url,
                    ReadRequest::SignatureStatus {
                        id,
                        signature: signature.to_string(),
                    },
                    deadline,
                )
                .await
                .map_err(|_| anyhow::anyhow!("signature status RPC unavailable"))?;
            ensure!(
                reply.status.is_success(),
                "signature status RPC HTTP error {}",
                reply.status.as_u16()
            );
            decode(id, &reply.body)
        })
        .await
        .map_err(|_| anyhow::anyhow!("signature status RPC timed out"))??;
        ensure!(Instant::now() < deadline, "signature status RPC timed out");
        Ok(result)
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Value>,
    error: Option<Value>,
}
fn decode(id: u64, bytes: &[u8]) -> Result<Response<Vec<Option<TransactionStatus>>>> {
    ensure!(
        bytes.len() <= super::super::read_http::MAX_BODY,
        "signature status RPC body exceeds limit"
    );
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("invalid signature status JSON-RPC response"))?;
    ensure!(
        envelope.jsonrpc == "2.0"
            && envelope.id == id
            && !(envelope.result.is_some() && envelope.error.is_some()),
        "signature status RPC identity or result invalid"
    );
    if let Some(error) = envelope.error {
        bail!(
            "signature status JSON-RPC error {}",
            error.get("code").and_then(Value::as_i64).unwrap_or(0)
        );
    }
    let result = envelope
        .result
        .ok_or_else(|| anyhow::anyhow!("signature status RPC omitted result"))?;
    serde_json::from_value(result)
        .map_err(|_| anyhow::anyhow!("invalid signature status RPC result"))
}

#[cfg(test)]
pub(crate) mod tests;
