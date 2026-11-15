use crate::domain::{sdk_candidate::SdkCandidateMetadata, submission::SignedSubmission};
use anyhow::{ensure, Context, Result};
use bincode::Options;
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};

/// Evidence validation only: never signs, refreshes a quote, or authorizes execution.
pub(crate) fn validate_signed_candidate(
    metadata: &SdkCandidateMetadata,
    message: &[u8],
    signed: &SignedSubmission,
) -> Result<()> {
    super::validate_candidate(metadata, message)?;
    ensure!(
        !signed.bytes.is_empty() && signed.bytes.len() <= 1232,
        "SDK signed packet size is invalid"
    );
    let transaction: VersionedTransaction = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(1232)
        .reject_trailing_bytes()
        .deserialize(&signed.bytes)
        .context("decode SDK signed evidence")?;
    transaction
        .sanitize()
        .map_err(|_| anyhow::anyhow!("SDK signed packet failed sanitization"))?;
    ensure!(
        bincode::serialize(&transaction)? == signed.bytes
            && transaction.message.serialize() == message
            && transaction.message.header().num_required_signatures == 1
            && transaction
                .message
                .static_account_keys()
                .first()
                .map(|p| p.to_bytes())
                == Some(metadata.details().wallet)
            && transaction.signatures.len() == 1,
        "SDK signed packet differs from the original candidate"
    );
    let signature = transaction.signatures[0];
    ensure!(
        signature != Signature::default()
            && signature.to_string() == signed.signature
            && signature.verify(&metadata.details().wallet, message),
        "SDK signature does not verify the original wallet and message"
    );
    Ok(())
}
