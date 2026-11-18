use super::*;
use crate::adapters::rpc::{
    read_http::ReadHttpTransport, read_runtime::SharedReadRuntime,
    sol_transfer_confirmation::SolTransferConfirmationReader,
};

pub(super) async fn confirm<T: ReadHttpTransport>(
    signed: &SignedSolTransfer,
    signature: &Signature,
    runtime: &SharedReadRuntime<T>,
    timeout: Duration,
) -> bool {
    if signed.submission().signature != signature.to_string() {
        return false;
    }
    SolTransferConfirmationReader::new(
        runtime,
        signed.prepared.broadcast.url(),
        *signature,
        signed.prepared.broadcast.commitment(),
    )
    .confirm(timeout)
    .await
    .unwrap_or(false)
}
