use super::{read_http::ReadHttpTransport, read_runtime::SharedReadRuntime};
use crate::domain::readiness::ReadinessWalletMetadata;

pub(crate) struct RpcReadinessProbe {
    metadata: ReadinessWalletMetadata,
    execution_urls: Vec<String>,
}

impl RpcReadinessProbe {
    pub fn new(metadata: ReadinessWalletMetadata, execution_urls: Vec<String>) -> Self {
        Self {
            metadata,
            execution_urls,
        }
    }

    pub async fn ping<T: ReadHttpTransport>(&self, runtime: &SharedReadRuntime<T>) -> bool {
        runtime.readiness_ping(&self.execution_urls).await
    }

    pub fn into_metadata(self) -> ReadinessWalletMetadata {
        self.metadata
    }
}

#[cfg(test)]
#[path = "readiness/legacy_tests.rs"]
mod legacy_tests;
