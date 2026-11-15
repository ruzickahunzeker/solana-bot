use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Clone)]
pub(crate) struct TradeTasks(Arc<Inner>);

struct Inner {
    state: Mutex<State>,
    changed: Notify,
}

struct State {
    accepting: bool,
    next: u64,
    tasks: BTreeMap<u64, Option<String>>,
    abandoned: usize,
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl Default for TradeTasks {
    fn default() -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(State {
                accepting: true,
                next: 0,
                tasks: BTreeMap::new(),
                abandoned: 0,
            }),
            changed: Notify::new(),
        }))
    }
}

#[derive(Debug)]
pub(crate) struct DrainReport {
    pub(crate) timed_out: bool,
    pub(crate) active: usize,
    pub(crate) admitting: usize,
    pub(crate) abandoned: usize,
    pub(crate) request_ids: Vec<String>,
}

impl DrainReport {
    pub(crate) fn clean(&self) -> bool {
        !self.timed_out && self.active == 0 && self.abandoned == 0
    }
}

/// Process-local lifecycle ticket only, never a reservation or execution owner.
#[must_use]
pub(crate) struct DispatchTicket {
    inner: Arc<Inner>,
    id: u64,
    completed: bool,
}

impl TradeTasks {
    /// Register before the first acceptance await, closing the intake-to-spawn race.
    pub(crate) fn register(&self) -> Option<DispatchTicket> {
        let mut state = self.0.state();
        if !state.accepting {
            return None;
        }
        let id = state.next;
        state.next = id.checked_add(1)?;
        state.tasks.insert(id, None);
        Some(DispatchTicket {
            inner: self.0.clone(),
            id,
            completed: false,
        })
    }

    pub(crate) fn close(&self) {
        self.0.state().accepting = false;
    }

    pub(crate) async fn drain(&self, deadline: Duration) -> DrainReport {
        self.close();
        let timed_out = tokio::time::timeout(deadline, self.wait_empty())
            .await
            .is_err();
        let state = self.0.state();
        DrainReport {
            timed_out,
            active: state.tasks.len(),
            admitting: state.tasks.values().filter(|id| id.is_none()).count(),
            abandoned: state.abandoned,
            request_ids: state
                .tasks
                .values()
                .filter_map(Clone::clone)
                .take(32)
                .collect(),
        }
    }

    async fn wait_empty(&self) {
        loop {
            // Register before observing the map: the last task may finish between both operations.
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.0.state().tasks.is_empty() {
                return;
            }
            changed.await;
        }
    }
}

impl DispatchTicket {
    /// A ticket obtained before close may still dispatch its original accepted request.
    pub(crate) fn spawn(
        self,
        request_id: String,
        future: impl Future<Output = ()> + Send + 'static,
    ) {
        *self
            .inner
            .state()
            .tasks
            .get_mut(&self.id)
            .expect("live task ticket missing") = Some(request_id);
        tokio::spawn(async move {
            let mut ticket = self;
            future.await;
            ticket.completed = true;
        });
    }
}

impl Drop for DispatchTicket {
    fn drop(&mut self) {
        let mut state = self.inner.state();
        let request_id = state.tasks.remove(&self.id).flatten();
        let abandoned = request_id.is_some() && !self.completed;
        if abandoned {
            state.abandoned = state.abandoned.saturating_add(1);
        }
        let empty = state.tasks.is_empty();
        drop(state);
        if abandoned {
            tracing::error!(request_id, "ordinary trade task dropped without completion; preserve original journal, do not resubmit");
        }
        if empty {
            self.inner.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests;
