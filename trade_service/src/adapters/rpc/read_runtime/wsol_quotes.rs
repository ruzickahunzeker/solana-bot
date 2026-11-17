use super::*;
use crate::adapters::rpc::{
    parameter_reads::{invalid, unavailable},
    wsol_quote_reads::WsolQuoteQuery,
};
impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) fn wsol_quote_slot(&self, url: &str) -> Result<OwnedSemaphorePermit> {
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| unavailable("WSOL quote endpoint is not registered"))?;
        ensure!(
            self.available(index, Instant::now())?,
            unavailable("WSOL quote endpoint cooling down or unavailable")
        );
        self.slots[Class::WsolQuote.index()]
            .clone()
            .try_acquire_owned()
            .map_err(|_| unavailable("WSOL quote quota busy"))
    }
    pub(crate) async fn wsol_quote_step(
        &self,
        url: &str,
        query: WsolQuoteQuery,
        total: Instant,
    ) -> Result<Value> {
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| unavailable("WSOL quote endpoint is not registered"))?;
        ensure!(
            self.available(index, Instant::now())?,
            unavailable("WSOL quote endpoint cooling down or unavailable")
        );
        let deadline = total.min(Instant::now() + Duration::from_secs(2));
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::WsolQuote { id, query };
        let max = request.max_body();
        let reply = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.read(Class::WsolQuote, index, request, deadline),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) if error.is::<super::super::read_http::ReadBodyLimit>() => {
                return Err(invalid("WSOL quote body exceeds limit"))
            }
            _ => return Err(unavailable("WSOL quote read unavailable")),
        };
        ensure!(
            reply.status.is_success(),
            unavailable("WSOL quote HTTP unavailable")
        );
        ensure!(
            reply.body.len() <= max,
            invalid("WSOL quote body exceeds limit")
        );
        let envelope: Envelope = serde_json::from_slice(&reply.body)
            .map_err(|_| invalid("WSOL quote envelope is invalid"))?;
        ensure!(
            envelope.jsonrpc == "2.0" && envelope.id == id,
            invalid("WSOL quote identity is invalid")
        );
        let result = match (envelope.result, envelope.error) {
            (Some(result), None) => result,
            (None, Some(_)) => return Err(unavailable("WSOL quote RPC error")),
            _ => return Err(invalid("WSOL quote result is invalid")),
        };
        ensure!(
            Instant::now() < deadline,
            unavailable("WSOL quote deadline expired")
        );
        Ok(result)
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
