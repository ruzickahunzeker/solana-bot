use std::{future::Future, pin::Pin};

use crate::{application::ports::readiness::ReadinessSource, domain::readiness::ReadinessReport};

// One boxed future at the HTTP composition boundary permits safe source injection.
pub trait ReadinessQuery: Send + Sync {
    fn inspect(&self) -> Pin<Box<dyn Future<Output = ReadinessReport> + Send + '_>>;
}

pub(crate) struct ReadinessService<S> {
    source: S,
}

impl<S: ReadinessSource> ReadinessService<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }
}

impl<S: ReadinessSource> ReadinessQuery for ReadinessService<S> {
    fn inspect(&self) -> Pin<Box<dyn Future<Output = ReadinessReport> + Send + '_>> {
        Box::pin(async move {
            let snapshot = self.source.read().await;
            // Preserve V1: paused and database health are independent diagnostic fields.
            ReadinessReport {
                ready: snapshot.has_blockhash && snapshot.rpc_ok,
                snapshot,
            }
        })
    }
}
