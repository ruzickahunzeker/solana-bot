use super::*;
use crate::domain::trade::Address;

const PAGE_SIZE: usize = 100;
const MAX_WALLETS: usize = 1000;
const NODE_DEADLINE: Duration = Duration::from_secs(4);
const TOTAL_DEADLINE: Duration = Duration::from_secs(8);

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn native_balances(
        &self,
        execution_urls: &[String],
        wallets: &[Address],
    ) -> Result<Vec<u64>> {
        ensure!(
            wallets.len() <= MAX_WALLETS,
            "wallet balances unavailable: at most 1000 addresses per query"
        );
        if wallets.is_empty() {
            return Ok(vec![]);
        }
        let candidates = self.candidate_indices(execution_urls);
        ensure!(
            !candidates.is_empty(),
            "wallet balances unavailable: no Default RPC lane configured"
        );
        let deadline = Instant::now() + TOTAL_DEADLINE;
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let _slot = self.slots[Class::NativeBalances.index()]
                .clone()
                .try_acquire_owned()
                .map_err(|_| {
                    anyhow::anyhow!("wallet balances unavailable: native balance quota busy")
                })?;
            for index in candidates {
                if Instant::now() >= deadline {
                    break;
                }
                if !self.available(index, Instant::now())? {
                    continue;
                }
                let node_deadline = (Instant::now() + NODE_DEADLINE).min(deadline);
                let result =
                    tokio::time::timeout_at(tokio::time::Instant::from_std(node_deadline), async {
                        // Discard every partial page before changing nodes; never mix provider snapshots.
                        let mut balances = Vec::with_capacity(wallets.len());
                        for page in wallets.chunks(PAGE_SIZE) {
                            ensure!(
                                Instant::now() < node_deadline,
                                "native balance deadline expired"
                            );
                            let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
                            let reply = self
                                .read(
                                    Class::NativeBalances,
                                    index,
                                    ReadRequest::NativeBalances {
                                        id,
                                        wallets: page.to_vec(),
                                    },
                                    node_deadline,
                                )
                                .await?;
                            ensure!(reply.status.is_success(), "native balance RPC unavailable");
                            let page_balances = parse_balances(id, page.len(), &reply.body)?;
                            ensure!(
                                Instant::now() < node_deadline,
                                "native balance deadline expired after decoding"
                            );
                            balances.extend(page_balances);
                        }
                        Ok::<_, anyhow::Error>(balances)
                    })
                    .await;
                if let Ok(Ok(balances)) = result {
                    return Ok(balances);
                }
            }
            bail!("wallet balances unavailable: all Default RPC lanes failed")
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!("wallet balances unavailable: native balance lookup timed out")
        })?
    }
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: u64,
    result: Option<Balances>,
    error: Option<Value>,
}
#[derive(Deserialize)]
struct Balances {
    context: Context,
    value: Vec<Option<Account>>,
}
#[derive(Deserialize)]
struct Context {
    slot: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    lamports: u64,
    owner: String,
    data: [String; 2],
    executable: bool,
    rent_epoch: u64,
}

fn parse_balances(id: u64, count: usize, bytes: &[u8]) -> Result<Vec<u64>> {
    ensure!(
        bytes.len() <= super::super::read_http::MAX_BODY,
        "native balance RPC body exceeds limit"
    );
    let response: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("invalid native balance JSON-RPC response"))?;
    ensure!(
        response.jsonrpc == "2.0" && response.id == id && response.error.is_none(),
        "native balance RPC identity or result invalid"
    );
    let result = response
        .result
        .ok_or_else(|| anyhow::anyhow!("native balance RPC omitted result"))?;
    ensure!(
        result.value.len() == count,
        "native balance RPC account count mismatch"
    );
    let _slot = result.context.slot;
    result
        .value
        .into_iter()
        .map(|account| {
            let Some(account) = account else {
                return Ok(0);
            };
            ensure!(
                account.owner.len() <= 44,
                "invalid native balance account owner"
            );
            let mut owner = [0u8; 32];
            let length = bs58::decode(account.owner)
                .onto(&mut owner[..])
                .map_err(|_| anyhow::anyhow!("invalid native balance account owner"))?;
            ensure!(length == 32, "invalid native balance account owner");
            ensure!(
                account.data[0].is_empty() && account.data[1] == "base64",
                "native balance RPC did not honor empty data slice"
            );
            let _diagnostics = (account.executable, account.rent_epoch);
            Ok(account.lamports)
        })
        .collect()
}

#[cfg(test)]
mod tests;
