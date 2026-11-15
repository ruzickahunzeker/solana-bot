use std::future::Future;

use crate::{
    application::{
        ordinary_observation_context::{CapturedOrdinaryObservation, OrdinaryObservationContext},
        ports::ordinary_claims::ExecutionOwner,
        trade_admission::AdmittedTrade,
        trade_record::TradeRecordV1,
    },
    domain::reconciliation::ChainObservation,
};

/// Observation only: no reservation, signing, broadcast or execution-claim capability.
pub(crate) trait OrdinaryReconciliationJournal: Sync {
    fn capture_live_observation(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
    ) -> impl Future<Output = anyhow::Result<CapturedOrdinaryObservation>> + Send;

    #[cfg(test)]
    fn observation_record(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
    ) -> impl Future<Output = anyhow::Result<TradeRecordV1>> + Send;

    #[cfg(test)]
    fn record_final_observation(
        &self,
        execution: &AdmittedTrade,
        owner: &ExecutionOwner,
        signature: &str,
        observation: &ChainObservation,
    ) -> impl Future<Output = anyhow::Result<TradeRecordV1>> + Send;
}

/// Recovery has no AdmittedTrade/ExecutionOwner, raw transaction, signer or broadcast method.
pub(crate) trait RecoveredOrdinaryObservations: Sync {
    fn capture_observation(
        &self,
        request_id: &str,
    ) -> impl Future<Output = anyhow::Result<CapturedOrdinaryObservation>> + Send;

    fn read_observation(
        &self,
        context: &OrdinaryObservationContext,
    ) -> impl Future<Output = anyhow::Result<TradeRecordV1>> + Send;

    fn record_captured_finality(
        &self,
        context: &OrdinaryObservationContext,
        observation: &ChainObservation,
    ) -> impl Future<Output = anyhow::Result<TradeRecordV1>> + Send;
}
