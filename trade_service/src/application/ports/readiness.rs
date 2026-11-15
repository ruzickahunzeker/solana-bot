use crate::domain::readiness::ReadinessSnapshot;
use std::future::Future;

/// A diagnostic observation, without signing, submission or wallet management capabilities.
pub(crate) trait ReadinessSource: Send + Sync {
    fn read(&self) -> impl Future<Output = ReadinessSnapshot> + Send;
}
