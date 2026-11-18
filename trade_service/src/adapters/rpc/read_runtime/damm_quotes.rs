use super::*;
use crate::adapters::rpc::damm_quote_reads::{DammQuoteAccounts, DammQuotePacket};
use solana_account_decoder::UiAccount;
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn damm_quote_accounts(
        &self,
        url: &str,
        query: DammQuoteAccounts,
    ) -> Result<Response<Vec<Option<UiAccount>>>> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let _permit = self.parameter_slot()?;
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .damm_quote_step(
                url,
                ReadRequest::DammQuoteAccounts { id, query },
                id,
                Class::Parameters,
                deadline,
                "DAMM quote accounts",
            )
            .await?;
        // Leave slot/count/role decisions at their original SDK validator so typed
        // terminal balance rejections cannot turn into generic lane fallback.
        let value = serde_json::from_value(response.value)
            .map_err(|_| anyhow::anyhow!("invalid DAMM quote account response"))?;
        ensure!(
            Instant::now() < deadline,
            "DAMM quote account deadline expired after decoding"
        );
        Ok(Response {
            context: response.context,
            value,
        })
    }

    pub(crate) async fn damm_quote_probe(
        &self,
        url: &str,
        packet: DammQuotePacket,
    ) -> Result<Response<RpcSimulateTransactionResult>> {
        // The selected SDK client's original default is 30s. Buy's existing
        // whole-quote 5s wrapper and original evidence freshness still apply.
        let deadline = Instant::now() + Duration::from_secs(30);
        let _permit = self.aggregator_simulation_slot(url)?;
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .damm_quote_step(
                url,
                ReadRequest::DammQuoteProbe { id, packet },
                id,
                Class::AggregatorSimulation,
                deadline,
                "DAMM quote probe",
            )
            .await?;
        ensure!(
            response
                .value
                .as_object()
                .is_some_and(|value| value.contains_key("err")),
            "DAMM quote simulation omitted explicit error outcome"
        );
        let value = serde_json::from_value(response.value)
            .map_err(|_| anyhow::anyhow!("invalid DAMM quote simulation result"))?;
        ensure!(
            Instant::now() < deadline,
            "DAMM quote probe deadline expired after decoding"
        );
        Ok(Response {
            context: response.context,
            value,
        })
    }

    async fn damm_quote_step(
        &self,
        url: &str,
        request: ReadRequest,
        id: u64,
        class: Class,
        deadline: Instant,
        label: &'static str,
    ) -> Result<Response<Value>> {
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| anyhow::anyhow!("DAMM quote endpoint not registered"))?;
        let maximum = request.max_body();
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .read(class, index, request, deadline)
                .await
                .map_err(|_| anyhow::anyhow!("{label} RPC unavailable"))?;
            ensure!(reply.status.is_success(), "{label} HTTP unavailable");
            ensure!(reply.body.len() <= maximum, "{label} body exceeds limit");
            let envelope: Envelope = serde_json::from_slice(&reply.body)
                .map_err(|_| anyhow::anyhow!("invalid DAMM quote JSON-RPC response"))?;
            ensure!(
                envelope.jsonrpc == "2.0" && envelope.id == id && envelope.error.is_none(),
                "DAMM quote RPC identity or result invalid"
            );
            let result = envelope
                .result
                .ok_or_else(|| anyhow::anyhow!("DAMM quote RPC omitted result"))?;
            ensure!(
                Instant::now() < deadline,
                "{label} deadline expired after parsing"
            );
            Ok(result)
        })
        .await
        .map_err(|_| anyhow::anyhow!("{label} timed out"))?
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Response<Value>>,
    error: Option<Value>,
}
