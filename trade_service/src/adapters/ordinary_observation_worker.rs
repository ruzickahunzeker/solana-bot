use crate::{
    adapters::{
        persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
        rpc::ordinary_observation::SingleShotStatusReader, rpc::read_runtime::SharedReadRuntime,
    },
    application::{
        ordinary_observation_scan::{OrdinaryObservationScanner, PendingOrdinaryObservations},
        ports::{
            ordinary_reconciliation::RecoveredOrdinaryObservations,
            reconciliation::SignatureStatusReader,
        },
    },
    config::ordinary_observation::OrdinaryObservationConfig,
    persistence::TradePersistence,
};
use anyhow::{ensure, Result};
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle};

pub(crate) struct ObservationPlan {
    reader: Arc<SingleShotStatusReader>,
    scanner: OrdinaryObservationScanner<SqliteOrdinaryExecutionClaims, SingleShotStatusReader>,
    config: OrdinaryObservationConfig,
}
impl ObservationPlan {
    pub(crate) fn prepare(
        config: &OrdinaryObservationConfig,
        persistence: &TradePersistence,
        runtime: Arc<SharedReadRuntime>,
    ) -> Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }
        config.validate()?;
        ensure!(
            persistence.is_durable(),
            "ordinary observation worker requires durable persistence"
        );
        let journal = persistence
            .ordinary_execution_claims()
            .ok_or_else(|| anyhow::anyhow!("ordinary observation journal unavailable"))?;
        let reader = Arc::new(SingleShotStatusReader::with_runtime(
            runtime,
            Duration::from_millis(config.request_interval_ms),
        )?);
        Ok(Some(Self {
            scanner: OrdinaryObservationScanner::new(journal, reader.clone()),
            reader,
            config: config.clone(),
        }))
    }
    pub(crate) fn reader(&self) -> Arc<SingleShotStatusReader> {
        self.reader.clone()
    }
    pub(crate) fn start(self) -> ObservationWorker {
        spawn_scanner(
            self.scanner,
            self.config,
            crate::state::now_ms,
            Duration::from_secs(20),
        )
    }
}

#[derive(Clone)]
pub(crate) struct ObservationStop(watch::Sender<bool>);
impl ObservationStop {
    pub(crate) fn request(&self) {
        let _ = self.0.send(true);
    }
}
pub(crate) struct ObservationWorker {
    stop: ObservationStop,
    task: Option<JoinHandle<()>>,
}
impl ObservationWorker {
    pub(crate) fn stop_signal(&self) -> ObservationStop {
        self.stop.clone()
    }
    pub(crate) async fn shutdown(mut self) -> Result<()> {
        self.stop.request();
        if let Some(task) = self.task.take() {
            task.await
                .map_err(|_| anyhow::anyhow!("ordinary observation task failed"))?;
        }
        Ok(())
    }
}
impl Drop for ObservationWorker {
    fn drop(&mut self) {
        self.stop.request();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

fn spawn_scanner<J, R>(
    mut scanner: OrdinaryObservationScanner<J, R>,
    config: OrdinaryObservationConfig,
    now: fn() -> u128,
    batch_deadline: Duration,
) -> ObservationWorker
where
    J: PendingOrdinaryObservations + RecoveredOrdinaryObservations + Send + 'static,
    R: SignatureStatusReader + Send + Sync + 'static,
{
    let (tx, mut stop) = watch::channel(false);
    let task = tokio::spawn(async move {
        loop {
            if *stop.borrow() {
                return;
            }
            let cutoff = match i64::try_from(now().saturating_sub(u128::from(config.min_age_ms))) {
                Ok(value) => value,
                Err(_) => {
                    tracing::error!("ordinary observation clock outside storage range");
                    return;
                }
            };
            let result = tokio::select! {
                biased;
                _ = stop.changed() => return,
                result = tokio::time::timeout(batch_deadline, scanner.step(cutoff, config.batch_size, Duration::from_millis(config.request_interval_ms))) => result,
            };
            match result {
                Ok(Ok(report)) => tracing::debug!(?report, "ordinary observation scan completed"),
                Ok(Err(error)) => tracing::warn!(error = %crate::sanitize::redact_urls(&error.to_string()), "ordinary observation scan unavailable; no transaction retry"),
                Err(_) => tracing::warn!("ordinary observation batch deadline; preserve scan progress without transaction retry"),
            }
            tokio::select! {
                biased;
                _ = stop.changed() => return,
                _ = tokio::time::sleep(Duration::from_millis(config.poll_interval_ms)) => {},
            }
        }
    });
    ObservationWorker {
        stop: ObservationStop(tx),
        task: Some(task),
    }
}

#[cfg(test)]
mod tests;
