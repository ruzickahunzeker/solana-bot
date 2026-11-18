use anyhow::{bail, Context, Result};
#[cfg(test)]
use solana_client::nonblocking::rpc_client::RpcClient;
#[cfg(test)]
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::{TransactionConfirmationStatus, TransactionStatus};
#[cfg(test)]
use std::{sync::Arc, time::Duration};

#[cfg(test)]
use crate::application::ports::reconciliation::SignatureStatusReader;
use crate::domain::reconciliation::{ChainCommitment, ChainObservation};

#[cfg(test)]
pub(crate) struct RpcSignatureStatusReader {
    pub rpcs: Vec<Arc<RpcClient>>,
}

#[cfg(test)]
impl SignatureStatusReader for RpcSignatureStatusReader {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        let parsed = signature
            .parse::<Signature>()
            .context("invalid persisted transaction signature")?;
        let mut missing = false;
        let mut last_error = "no status RPC configured".to_string();
        for rpc in self.rpcs.iter().take(2) {
            match tokio::time::timeout(
                Duration::from_millis(1_000),
                rpc.get_signature_statuses_with_history(&[parsed]),
            )
            .await
            {
                Ok(Ok(response)) if response.value.len() == 1 => {
                    if let Some(status) = response.value.into_iter().next().flatten() {
                        return Ok(Some(observation(signature, status)?));
                    }
                    missing = true;
                }
                Ok(Ok(_)) => {
                    last_error = "signature status RPC returned an invalid result count".into()
                }
                Ok(Err(error)) => last_error = crate::sanitize::redact_urls(&error.to_string()),
                Err(_) => last_error = "signature status RPC timed out".into(),
            }
        }
        if missing {
            return Ok(None);
        }
        bail!("{last_error}")
    }
}

pub(super) fn observation(signature: &str, status: TransactionStatus) -> Result<ChainObservation> {
    if status.status.as_ref().err() != status.err.as_ref() {
        bail!("signature status RPC returned inconsistent transaction errors");
    }
    let commitment = match status
        .confirmation_status
        .context("signature status RPC omitted explicit confirmation status")?
    {
        TransactionConfirmationStatus::Processed => ChainCommitment::Processed,
        TransactionConfirmationStatus::Confirmed => ChainCommitment::Confirmed,
        TransactionConfirmationStatus::Finalized => ChainCommitment::Finalized,
    };
    Ok(ChainObservation {
        signature: signature.into(),
        commitment,
        slot: status.slot,
        error: status
            .err
            .map(|error| format!("on-chain transaction error: {error:?}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use solana_client::rpc_request::RpcRequest;
    use std::collections::HashMap;

    fn status(commitment: &str) -> serde_json::Value {
        json!({"slot":123,"confirmations":1,"status":{"Ok":null},"err":null,"confirmationStatus":commitment})
    }

    fn rpc(value: serde_json::Value) -> Arc<RpcClient> {
        let mut mocks = HashMap::new();
        mocks.insert(
            RpcRequest::GetSignatureStatuses,
            json!({"context":{"slot":123},"value":value}),
        );
        Arc::new(RpcClient::new_mock_with_mocks("succeeds".into(), mocks))
    }

    #[tokio::test]
    async fn rpc_mock_maps_explicit_finality_without_any_signer() {
        let signature = Signature::from([1u8; 64]).to_string();
        let reader = RpcSignatureStatusReader {
            rpcs: vec![rpc(json!([status("finalized")]))],
        };
        let result = reader.lookup_signature(&signature).await.unwrap().unwrap();
        assert_eq!(result.signature, signature);
        assert_eq!(result.commitment, ChainCommitment::Finalized);
        assert_eq!(result.slot, 123);
    }

    #[tokio::test]
    async fn rpc_mock_tries_backup_after_primary_missing() {
        let signature = Signature::from([1u8; 64]).to_string();
        let reader = RpcSignatureStatusReader {
            rpcs: vec![rpc(json!([null])), rpc(json!([status("confirmed")]))],
        };
        assert_eq!(
            reader
                .lookup_signature(&signature)
                .await
                .unwrap()
                .unwrap()
                .commitment,
            ChainCommitment::Confirmed
        );
        let missing = RpcSignatureStatusReader {
            rpcs: vec![rpc(json!([null]))],
        };
        assert!(missing
            .lookup_signature(&signature)
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn missing_commitment_and_inconsistent_errors_are_not_treated_as_finality() {
        let mut incomplete = status("confirmed");
        incomplete
            .as_object_mut()
            .unwrap()
            .remove("confirmationStatus");
        assert!(observation("sig", serde_json::from_value(incomplete).unwrap()).is_err());
        let mut inconsistent = status("finalized");
        inconsistent["err"] = json!({"InstructionError":[0,{"Custom":42}]});
        assert!(observation("sig", serde_json::from_value(inconsistent.clone()).unwrap()).is_err());
        inconsistent["status"] = json!({"Err":inconsistent["err"]});
        assert!(
            observation("sig", serde_json::from_value(inconsistent).unwrap())
                .unwrap()
                .error
                .is_some()
        );
    }

    #[tokio::test]
    async fn invalid_signature_empty_lanes_and_malformed_result_fail_closed() {
        let reader = RpcSignatureStatusReader { rpcs: vec![] };
        assert!(reader.lookup_signature("not-a-signature").await.is_err());
        let signature = Signature::from([1u8; 64]).to_string();
        assert!(reader.lookup_signature(&signature).await.is_err());
        let malformed = RpcSignatureStatusReader {
            rpcs: vec![rpc(json!([]))],
        };
        assert!(malformed.lookup_signature(&signature).await.is_err());
    }
}
