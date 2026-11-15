use crate::domain::readiness::{ReadinessSnapshot, WalletReadinessObservation};
use std::future::Future;

/// A diagnostic observation, without signing, submission or wallet management capabilities.
pub(crate) trait ReadinessSource: Send + Sync {
    fn read(&self) -> impl Future<Output = ReadinessSnapshot> + Send;
}

pub(crate) trait ReadinessWalletObserver: Send + Sync {
    fn observe_wallets(&self) -> impl Future<Output = WalletReadinessObservation> + Send;
}
