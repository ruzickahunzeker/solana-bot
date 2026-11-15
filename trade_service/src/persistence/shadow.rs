use super::{insert_trade, update_trade, TradeRecord};
use sqlx::SqlitePool;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

pub(super) enum ShadowWrite {
    Insert {
        record: TradeRecord,
        key: String,
        hash: String,
        request_json: String,
    },
    Update(TradeRecord),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    accepted: u64,
    completed: u64,
    failed: u64,
    rejected: u64,
    overflowed: bool,
}
impl Counts {
    fn increment(value: &mut u64, overflowed: &mut bool) {
        match value.checked_add(1) {
            Some(next) => *value = next,
            None => *overflowed = true,
        }
    }
    fn accepted(&mut self) {
        Self::increment(&mut self.accepted, &mut self.overflowed);
    }
    fn rejected(&mut self) {
        Self::increment(&mut self.rejected, &mut self.overflowed);
    }
    fn completed(&mut self, success: bool) {
        Self::increment(&mut self.completed, &mut self.overflowed);
        if !success {
            Self::increment(&mut self.failed, &mut self.overflowed);
        }
    }
    fn clean(self) -> bool {
        !self.overflowed
            && self.accepted == self.completed
            && self.failed == 0
            && self.rejected == 0
    }
}

struct Gate {
    sender: Option<mpsc::Sender<ShadowWrite>>,
    counts: Counts,
}
struct Completion {
    task: Option<JoinHandle<()>>,
    joined: Option<bool>,
}

/// Owns the original single FIFO and writer, not a reservation or durable guarantee.
pub(super) struct ShadowWriter {
    gate: Arc<Mutex<Gate>>,
    completion: tokio::sync::Mutex<Completion>,
    healthy: Arc<AtomicBool>,
}

fn lock(gate: &Mutex<Gate>) -> MutexGuard<'_, Gate> {
    gate.lock().unwrap_or_else(|error| error.into_inner())
}

impl ShadowWriter {
    pub(super) fn start(pool: SqlitePool, healthy: Arc<AtomicBool>) -> Self {
        let (sender, mut receiver) = mpsc::channel(1_024);
        let gate = Arc::new(Mutex::new(Gate {
            sender: Some(sender),
            counts: Counts::default(),
        }));
        let writer_gate = gate.clone();
        let writer_healthy = healthy.clone();
        let task = tokio::spawn(async move {
            while let Some(write) = receiver.recv().await {
                let result = match write {
                    ShadowWrite::Insert {
                        record,
                        key,
                        hash,
                        request_json,
                    } => insert_trade(&pool, &record, &key, &hash, &request_json)
                        .await
                        .map(|_| ()),
                    ShadowWrite::Update(record) => update_trade(&pool, &record).await,
                };
                // Preserve the existing last-write health behavior; exit evidence is separate.
                match result {
                    Ok(()) => writer_healthy.store(true, Ordering::Relaxed),
                    Err(ref error) => {
                        writer_healthy.store(false, Ordering::Relaxed);
                        tracing::warn!(%error, "shadow trade persistence write failed");
                    }
                }
                lock(&writer_gate).counts.completed(result.is_ok());
            }
        });
        Self {
            gate,
            completion: tokio::sync::Mutex::new(Completion {
                task: Some(task),
                joined: None,
            }),
            healthy,
        }
    }

    pub(super) fn enqueue(
        &self,
        write: ShadowWrite,
    ) -> Result<(), mpsc::error::TrySendError<ShadowWrite>> {
        let mut gate = lock(&self.gate);
        let result = match &gate.sender {
            Some(sender) => sender.try_send(write),
            None => Err(mpsc::error::TrySendError::Closed(write)),
        };
        if result.is_ok() {
            gate.counts.accepted();
        } else {
            gate.counts.rejected();
        }
        result
    }

    fn close(&self) {
        // The sender is never cloned outside the gate, making close the final enqueue boundary.
        let sender = lock(&self.gate).sender.take();
        drop(sender);
    }

    pub(super) async fn shutdown(&self, deadline: Duration) -> anyhow::Result<()> {
        self.close();
        match tokio::time::timeout(deadline, self.join()).await {
            Ok(result) => result,
            Err(_) => {
                let counts = lock(&self.gate).counts;
                anyhow::bail!("shadow writer drain timed out; original writer retained: accepted={}, completed={}, failed={}, rejected={}",
                    counts.accepted, counts.completed, counts.failed, counts.rejected)
            }
        }
    }

    async fn join(&self) -> anyhow::Result<()> {
        let mut completion = self.completion.lock().await;
        if completion.joined.is_none() {
            let task = completion
                .task
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("shadow writer task unavailable"))?;
            // Poll in place: cancellation/timeout must never discard the original JoinHandle.
            let success = task.await.is_ok();
            completion.joined = Some(success);
            completion.task = None;
            if !success {
                self.healthy.store(false, Ordering::Relaxed);
            }
        }
        let counts = lock(&self.gate).counts;
        anyhow::ensure!(
            completion.joined == Some(true),
            "shadow writer task failed; queued writes may be incomplete"
        );
        anyhow::ensure!(counts.clean(),
            "shadow writer drain incomplete: accepted={}, completed={}, failed={}, rejected={}, overflowed={}",
            counts.accepted, counts.completed, counts.failed, counts.rejected, counts.overflowed);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn is_closed(&self) -> bool {
        lock(&self.gate).sender.is_none()
    }
}

impl Drop for ShadowWriter {
    fn drop(&mut self) {
        self.close();
        // Dropping a JoinHandle detaches it; do not abort accepted FIFO writes.
    }
}

#[cfg(test)]
mod tests;
