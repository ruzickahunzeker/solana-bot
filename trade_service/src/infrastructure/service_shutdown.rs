use super::trade_tasks::TradeTasks;
use crate::{
    adapters::ordinary_observation_worker::ObservationWorker, blockhash_cache::BlockhashRefresher,
};
use std::time::Duration;

/// Keep original refresh/observation resources alive until the trade drain has finished or expired.
pub(crate) async fn finish(
    tasks: &TradeTasks,
    deadline: Duration,
    blockhash: BlockhashRefresher,
    observation: Option<ObservationWorker>,
    persistence: &crate::persistence::TradePersistence,
    shadow_deadline: Duration,
) -> anyhow::Result<()> {
    let report = tasks.drain(deadline).await;
    if !report.clean() {
        tracing::warn!(
            timed_out = report.timed_out, active = report.active,
            admitting = report.admitting, abandoned = report.abandoned,
            request_ids = ?report.request_ids,
            "ordinary trade shutdown incomplete; retain original journal for recovery, never resubmit automatically"
        );
    }
    blockhash.abort_handle().abort();
    if let Some(worker) = &observation {
        worker.stop_signal().request();
    }
    // Finish every original resource before propagating a cleanup error.
    let observation_result = match observation {
        Some(worker) => worker.shutdown().await,
        None => Ok(()),
    };
    let blockhash_result = blockhash.shutdown().await;
    let shadow_result = persistence.shutdown_shadow(shadow_deadline).await;
    if let Err(error) = &shadow_result {
        tracing::warn!(%error, "shadow persistence did not drain cleanly; no replay or durable promotion");
    }
    observation_result?;
    blockhash_result?;
    anyhow::ensure!(
        report.clean(),
        "ordinary trade shutdown incomplete: active={}, admitting={}, abandoned={}, timed_out={}",
        report.active,
        report.admitting,
        report.abandoned,
        report.timed_out
    );
    shadow_result?;
    Ok(())
}

#[cfg(test)]
mod tests;
