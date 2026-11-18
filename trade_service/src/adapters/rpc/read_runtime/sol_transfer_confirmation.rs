use super::*;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(in crate::adapters::rpc) async fn sol_transfer_confirmation(
        &self,
        url: &str,
        signature: Signature,
        commitment: CommitmentConfig,
        budget: Duration,
    ) -> Result<bool> {
        ensure!(
            signature != Signature::default(),
            "invalid SOL transfer confirmation signature"
        );
        ensure!(
            !budget.is_zero() && budget <= Duration::from_secs(15),
            "invalid SOL transfer confirmation budget"
        );
        let deadline = Instant::now() + budget;
        let index = *self
            .candidate_indices(&[url.into()])
            .first()
            .ok_or_else(|| {
                anyhow::anyhow!("SOL transfer confirmation endpoint is not registered")
            })?;
        ensure!(
            self.available(index, Instant::now())?,
            "SOL transfer confirmation endpoint cooling down or unavailable"
        );
        let _permit = self.slots[Class::Observation.index()]
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("SOL transfer confirmation quota busy"))?;
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        let request = ReadRequest::SolTransferConfirmation { id, signature };
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .read(Class::Observation, index, request, deadline)
                .await
                .map_err(|_| anyhow::anyhow!("SOL transfer confirmation RPC unavailable"))?;
            ensure!(
                reply.status.is_success(),
                "SOL transfer confirmation HTTP unavailable"
            );
            let confirmed =
                super::super::sol_transfer_confirmation::decode(id, &reply.body, commitment)?;
            ensure!(
                Instant::now() < deadline,
                "SOL transfer confirmation deadline expired after decoding"
            );
            Ok(confirmed)
        })
        .await
        .map_err(|_| anyhow::anyhow!("SOL transfer confirmation timed out"))?
    }
}
