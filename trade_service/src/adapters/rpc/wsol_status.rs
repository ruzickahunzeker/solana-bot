use anyhow::{bail, Context};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::Signature;
use solana_transaction_status_client_types::TransactionConfirmationStatus;
use std::sync::Arc;

use crate::{
    application::ports::wsol_queries::WsolStatusReader,
    domain::{
        reconciliation::{ChainCommitment, ChainObservation},
        wsol_progress::WsolSignatureObservation,
    },
};

pub(crate) struct WsolSignatureReader {
    rpc: Arc<RpcClient>,
    signature: Signature,
}
impl WsolSignatureReader {
    pub(crate) fn new(rpc: Arc<RpcClient>, signature: Signature) -> Self {
        Self { rpc, signature }
    }
}
impl WsolStatusReader for WsolSignatureReader {
    async fn lookup(&self) -> anyhow::Result<Option<WsolSignatureObservation>> {
        let response = self
            .rpc
            .get_signature_statuses_with_history(&[self.signature])
            .await?;
        if response.value.len() != 1 {
            bail!("WSOL signature status RPC returned an invalid result count");
        }
        if response.context.slot == 0 {
            bail!("WSOL signature status RPC returned a zero context slot");
        }
        let Some(status) = response.value.into_iter().next().flatten() else {
            return Ok(None);
        };
        if status.status.as_ref().err() != status.err.as_ref() {
            bail!("WSOL signature status RPC returned inconsistent transaction errors");
        }
        let commitment = match status
            .confirmation_status
            .context("WSOL signature status RPC omitted explicit confirmation status")?
        {
            TransactionConfirmationStatus::Processed => ChainCommitment::Processed,
            TransactionConfirmationStatus::Confirmed => ChainCommitment::Confirmed,
            TransactionConfirmationStatus::Finalized => ChainCommitment::Finalized,
        };
        if (commitment == ChainCommitment::Finalized) != status.confirmations.is_none() {
            bail!("WSOL signature status RPC returned inconsistent rooted confirmation fields");
        }
        let signature = self.signature.to_string();
        let observation = WsolSignatureObservation {
            chain: ChainObservation {
                signature: signature.clone(),
                commitment,
                slot: status.slot,
                error: status.err.as_ref().map(ToString::to_string),
            },
            context_slot: response.context.slot,
        };
        observation.outcome_for(&signature)?;
        Ok(Some(observation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use solana_client::rpc_request::RpcRequest;

    fn reader(values: Value) -> WsolSignatureReader {
        reader_at(values, 42)
    }

    fn reader_at(values: Value, context_slot: u64) -> WsolSignatureReader {
        let mut mocks = std::collections::HashMap::new();
        mocks.insert(
            RpcRequest::GetSignatureStatuses,
            json!({"context":{"slot":context_slot},"value":values}),
        );
        WsolSignatureReader::new(
            Arc::new(RpcClient::new_mock_with_mocks("fails".into(), mocks)),
            Signature::from([7; 64]),
        )
    }

    fn status(commitment: &str, confirmations: Value, error: Value) -> Value {
        json!({"slot":42,"confirmations":confirmations,"confirmationStatus":commitment,"err":error,
            "status":if error.is_null() {json!({"Ok":null})}else{json!({"Err":error})}})
    }

    #[tokio::test]
    async fn wsol_status_rpc_uses_explicit_finality_and_preserves_consistent_error_text() {
        for (commitment, confirmations, expected) in [
            ("processed", json!(0), ChainCommitment::Processed),
            ("confirmed", json!(1), ChainCommitment::Confirmed),
            ("finalized", json!(null), ChainCommitment::Finalized),
        ] {
            for error in [json!(null), json!({"InstructionError":[0,{"Custom":42}]})] {
                let observed = reader(json!([status(
                    commitment,
                    confirmations.clone(),
                    error.clone()
                )]))
                .lookup()
                .await
                .unwrap()
                .unwrap();
                assert_eq!(observed.chain.commitment, expected);
                assert_eq!(observed.chain.error.is_some(), !error.is_null());
                assert_eq!(observed.context_slot, 42);
                assert_eq!(observed.chain.slot, 42);
                if observed.chain.error.is_some() {
                    let expected = serde_json::from_value::<
                        solana_sdk::transaction::TransactionError,
                    >(error.clone())
                    .unwrap()
                    .to_string();
                    assert_eq!(observed.chain.error.as_deref(), Some(expected.as_str()));
                }
            }
        }
    }

    #[tokio::test]
    async fn wsol_status_rpc_empty_missing_and_errors_do_not_fabricate_observations() {
        assert!(reader(json!([null])).lookup().await.unwrap().is_none());
        assert!(reader(json!([])).lookup().await.is_err());
        let rpc = Arc::new(RpcClient::new_mock("fails".into()));
        let bound = WsolSignatureReader::new(rpc.clone(), Signature::from([7; 64]));
        assert!(Arc::ptr_eq(&bound.rpc, &rpc));
        assert!(bound.lookup().await.is_err());
    }

    #[tokio::test]
    async fn wsol_status_rpc_rejects_wrong_result_counts_and_invalid_context_or_transaction_slots()
    {
        let good = status("finalized", json!(null), json!(null));
        for value in [
            json!([]),
            json!([null, null]),
            json!([good.clone(), good.clone()]),
        ] {
            assert!(reader(value)
                .lookup()
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid result count"));
        }
        assert!(reader_at(json!([null]), 0).lookup().await.is_err());
        for (slot, context) in [(0, 42), (43, 42), (42, 0)] {
            let mut bad = good.clone();
            bad["slot"] = json!(slot);
            assert!(reader_at(json!([bad]), context).lookup().await.is_err());
        }
        assert!(reader_at(json!([good]), 43)
            .lookup()
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn wsol_status_rpc_rejects_missing_unknown_and_contradictory_confirmation_metadata() {
        for (commitment, confirmations) in [
            (json!(null), json!(null)),
            (json!(null), json!(1)),
            (json!("processed"), json!(null)),
            (json!("confirmed"), json!(null)),
            (json!("finalized"), json!(0)),
            (json!("finalized"), json!(1)),
            (json!("unknown"), json!(null)),
            (json!("finalized"), json!("invalid")),
        ] {
            let mut bad = status("finalized", json!(null), json!(null));
            bad["confirmationStatus"] = commitment;
            bad["confirmations"] = confirmations;
            assert!(reader(json!([bad])).lookup().await.is_err());
        }
        let mut missing = status("finalized", json!(null), json!(null));
        missing
            .as_object_mut()
            .unwrap()
            .remove("confirmationStatus");
        assert!(reader(json!([missing])).lookup().await.is_err());
    }

    #[tokio::test]
    async fn wsol_status_rpc_rejects_inconsistent_legacy_and_explicit_errors() {
        let failure = json!({"InstructionError":[0,{"Custom":42}]});
        for (error, legacy) in [
            (json!(null), json!({"Err":failure.clone()})),
            (failure.clone(), json!({"Ok":null})),
            (
                failure,
                json!({"Err":{"InstructionError":[0,{"Custom":43}]}}),
            ),
        ] {
            let mut bad = status("finalized", json!(null), error);
            bad["status"] = legacy;
            assert!(reader(json!([bad]))
                .lookup()
                .await
                .unwrap_err()
                .to_string()
                .contains("inconsistent transaction errors"));
        }
    }

    #[tokio::test]
    async fn wsol_status_rpc_queries_only_bound_original_signature_with_history() {
        use solana_client::{
            client_error::Result as RpcResult,
            rpc_client::RpcClientConfig,
            rpc_sender::{RpcSender, RpcTransportStats},
        };
        use std::{future::Future, pin::Pin, sync::Mutex};
        #[derive(Clone)]
        struct MemoryRpc(Arc<Mutex<Vec<(RpcRequest, Value)>>>);
        impl RpcSender for MemoryRpc {
            fn send<'life0, 'async_trait>(
                &'life0 self,
                request: RpcRequest,
                params: Value,
            ) -> Pin<Box<dyn Future<Output = RpcResult<Value>> + Send + 'async_trait>>
            where
                'life0: 'async_trait,
                Self: 'async_trait,
            {
                Box::pin(async move {
                    self.0.lock().unwrap().push((request, params));
                    assert_eq!(request, RpcRequest::GetSignatureStatuses);
                    Ok(
                        json!({"context":{"slot":43},"value":[status("finalized", json!(null), json!(null))]}),
                    )
                })
            }
            fn get_transport_stats(&self) -> RpcTransportStats {
                RpcTransportStats::default()
            }
            fn url(&self) -> String {
                "memory://bound-wsol-status".into()
            }
        }
        let sender = MemoryRpc(Arc::default());
        let rpc = Arc::new(RpcClient::new_sender(
            sender.clone(),
            RpcClientConfig::default(),
        ));
        let signature = Signature::from([7; 64]);
        let reader = WsolSignatureReader::new(rpc.clone(), signature);
        let observed = reader.lookup().await.unwrap().unwrap();
        assert!(Arc::ptr_eq(&reader.rpc, &rpc));
        assert_eq!(observed.chain.signature, signature.to_string());
        assert_eq!(
            observed.outcome_for(&signature.to_string()).unwrap(),
            crate::domain::wsol_progress::WsolObservedOutcome::Confirmed
        );
        assert_eq!(
            *sender.0.lock().unwrap(),
            [(
                RpcRequest::GetSignatureStatuses,
                json!([[signature.to_string()], {"searchTransactionHistory":true}])
            )]
        );
    }
}
