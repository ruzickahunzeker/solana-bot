use super::*;
use crate::adapters::rpc::parameter_reads::{invalid, request::ParameterQuery, unavailable};

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) fn parameter_slot(&self) -> Result<OwnedSemaphorePermit> {
        self.scoped_slot(Class::Parameters)
    }

    pub(crate) fn discovery_slot(&self) -> Result<OwnedSemaphorePermit> {
        self.scoped_slot(Class::Discovery)
    }

    fn scoped_slot(&self, class: Class) -> Result<OwnedSemaphorePermit> {
        self.slots[class.index()]
            .clone()
            .try_acquire_owned()
            .map_err(|_| unavailable("parameter RPC quota busy"))
    }

    pub(crate) async fn parameter_step(
        &self,
        url: &str,
        query: ParameterQuery,
        total: Instant,
    ) -> Result<Value> {
        self.scoped_parameter_step(Class::Parameters, url, query, total)
            .await
    }

    pub(crate) async fn discovery_step(
        &self,
        url: &str,
        query: ParameterQuery,
        total: Instant,
    ) -> Result<Value> {
        self.scoped_parameter_step(Class::Discovery, url, query, total)
            .await
    }

    async fn scoped_parameter_step(
        &self,
        class: Class,
        url: &str,
        query: ParameterQuery,
        total: Instant,
    ) -> Result<Value> {
        let index = *self
            .candidate_indices(&[url.to_owned()])
            .first()
            .ok_or_else(|| unavailable("parameter RPC endpoint is not registered"))?;
        ensure!(
            self.available(index, Instant::now())
                .map_err(|_| unavailable("parameter RPC budget unavailable"))?,
            unavailable("parameter RPC endpoint cooling down or unavailable")
        );
        let deadline = (Instant::now() + Duration::from_secs(2)).min(total);
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let max = query.max_body();
        let reply = match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.read(
                class,
                index,
                ReadRequest::Parameters {
                    id,
                    query: query.clone(),
                },
                deadline,
            ),
        )
        .await
        {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) if error.is::<super::super::read_http::ReadBodyLimit>() => {
                return Err(invalid("parameter RPC body exceeds limit"))
            }
            _ => return Err(unavailable("parameter RPC read unavailable")),
        };
        if matches!(class, Class::Discovery) && reply.status == StatusCode::TOO_MANY_REQUESTS {
            return Err(unavailable("pool discovery RPC rate limited (429)"));
        }
        ensure!(
            reply.status.is_success(),
            unavailable("parameter RPC HTTP unavailable")
        );
        ensure!(
            reply.body.len() <= max,
            invalid("parameter RPC body exceeds limit")
        );
        let envelope: Envelope = serde_json::from_slice(&reply.body)
            .map_err(|_| invalid("parameter RPC envelope is invalid"))?;
        ensure!(
            envelope.jsonrpc == "2.0" && envelope.id == id,
            invalid("parameter RPC identity is invalid")
        );
        let value = match (envelope.result, envelope.error) {
            (Some(result), None) => result,
            (None, Some(_)) => return Err(unavailable("parameter RPC error")),
            _ => return Err(invalid("parameter RPC result is invalid")),
        };
        query.validate_result(&value)?;
        ensure!(
            Instant::now() < deadline,
            unavailable("parameter RPC deadline expired")
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
