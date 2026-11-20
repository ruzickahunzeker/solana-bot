use crate::domain::{sdk_prepared_record::SdkPreparedRecord, submission::SignedSubmission};
use anyhow::{ensure, Context, Result};
use bincode::Options;
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};

/// Historical reconstruction only. Neither a nominal record nor a signature restores live authority.
pub(crate) fn validate_candidate(metadata: &SdkPreparedRecord, message: &[u8]) -> Result<()> {
    match metadata {
        SdkPreparedRecord::DammSell(_) => {
            super::damm_sell_message::validate_candidate(metadata, message)
        }
        SdkPreparedRecord::DammBuy(record) => {
            super::damm_probe::validate_buy_record(record, message)
        }
        SdkPreparedRecord::PumpNative(record) => {
            super::pump_native_message::validate(record, message)
        }
    }
}

/// Verifies the original packet; never acquires a key, RPC client, execution owner or sender.
pub(crate) fn validate_signed_candidate(
    metadata: &SdkPreparedRecord,
    message: &[u8],
    signed: &SignedSubmission,
) -> Result<()> {
    validate_candidate(metadata, message)?;
    ensure!(
        !matches!(metadata, SdkPreparedRecord::PumpNative(_)),
        "historical Pump candidate cannot enter signing"
    );
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
    let wallet = metadata.wallet();
    ensure!(
        bincode::serialize(&transaction)? == signed.bytes
            && transaction.message.serialize() == message
            && transaction.message.header().num_required_signatures == 1
            && transaction
                .message
                .static_account_keys()
                .first()
                .map(|p| p.to_bytes())
                == Some(wallet)
            && transaction.signatures.len() == 1,
        "SDK signed packet differs from the original candidate"
    );
    let signature = transaction.signatures[0];
    ensure!(
        signature != Signature::default()
            && signature.to_string() == signed.signature
            && signature.verify(&wallet, message),
        "SDK signature does not verify the original wallet and message"
    );
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
