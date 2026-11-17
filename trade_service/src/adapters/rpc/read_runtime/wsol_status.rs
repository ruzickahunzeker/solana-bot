use super::*;
use crate::domain::wsol_progress::WsolSignatureObservation;
use solana_sdk::signature::Signature;

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    pub(crate) async fn wsol_signature_status(
        &self,
        url: &str,
        signature: &Signature,
    ) -> Result<Option<WsolSignatureObservation>> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let url =
            Url::parse(url).map_err(|_| anyhow::anyhow!("invalid WSOL status RPC endpoint"))?;
        let id = self.hash_id.fetch_add(1, Ordering::Relaxed);
        // Reuse the existing observation permit and pacing, not a new per-reader quota.
        let result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let reply = self
                .signature_status(&url, id, &signature.to_string())
                .await
                .map_err(|_| anyhow::anyhow!("WSOL signature status RPC unavailable"))?;
            ensure!(
                reply.status.is_success(),
                "WSOL signature status RPC HTTP unavailable"
            );
            super::super::wsol_status::decode(id, signature, &reply.body)
        })
        .await
        .map_err(|_| anyhow::anyhow!("WSOL signature status RPC lookup timed out"))??;
        ensure!(
            Instant::now() < deadline,
            "WSOL signature status RPC deadline expired"
        );
        Ok(result)
    }
}
