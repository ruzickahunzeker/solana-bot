use super::*;
use crate::domain::sdk_prepared_record::SdkPreparedRecord;

/// Linear in-memory permission from the original owner's completed signing CAS.
/// It cannot be cloned, deserialized, or recovered from nominal database metadata.
pub(crate) struct SdkSigningPermit<'a> {
    execution: &'a AdmittedTrade,
    owner: &'a ExecutionOwner,
    metadata: &'a SdkPreparedRecord,
    message: &'a [u8],
}

impl<'a> SdkSigningPermit<'a> {
    pub(super) fn new(
        execution: &'a AdmittedTrade,
        owner: &'a ExecutionOwner,
        metadata: &'a SdkPreparedRecord,
        message: &'a [u8],
    ) -> Result<Self> {
        owner.ensure_binding(execution)?;
        ensure!(
            owner.operation() == ClaimOperation::SdkTrade,
            "SDK signing permit operation mismatch"
        );
        validate_origin(execution, metadata, message)?;
        Ok(Self {
            execution,
            owner,
            metadata,
            message,
        })
    }

    pub(crate) fn execution(&self) -> &AdmittedTrade {
        self.execution
    }

    pub(crate) fn validate(&self, metadata: &SdkPreparedRecord, message: &[u8]) -> Result<()> {
        self.owner.ensure_binding(self.execution)?;
        ensure!(
            *self.metadata == *metadata && self.message == message,
            "SDK signing permit candidate differs from the original journal"
        );
        validate_origin(self.execution, metadata, message)
    }
}
