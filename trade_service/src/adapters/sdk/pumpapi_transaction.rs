use anyhow::{bail, Context};
use bincode::Options;
use solana_sdk::{message::VersionedMessage, transaction::VersionedTransaction};

use crate::adapters::pumpapi::response::{
    check_body_size, decode_text_transaction, MAX_TRANSACTION_BYTES,
};

/// Structural decoding only. A sanitized message is not an authorized trade.
pub(crate) fn decode_pumpapi_transaction(body: &[u8]) -> anyhow::Result<VersionedTransaction> {
    check_body_size(body)?;
    match decode_packet(body) {
        Ok(transaction) => Ok(transaction),
        Err(raw_error) => match decode_text_transaction(body)? {
            Some(bytes) => decode_packet(&bytes),
            None => Err(raw_error),
        },
    }
}

fn decode_packet(bytes: &[u8]) -> anyhow::Result<VersionedTransaction> {
    if bytes.len() > MAX_TRANSACTION_BYTES {
        bail!("PumpAPI transaction exceeds {MAX_TRANSACTION_BYTES} bytes");
    }
    let transaction: VersionedTransaction = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_TRANSACTION_BYTES as u64)
        .reject_trailing_bytes()
        .deserialize(bytes)
        .context("decode PumpAPI unsigned transaction")?;
    validate_packet_structure(&transaction)?;
    Ok(transaction)
}

pub(super) fn validate_packet_structure(transaction: &VersionedTransaction) -> anyhow::Result<()> {
    if matches!(&transaction.message, VersionedMessage::V1(_)) {
        bail!("PumpAPI message version is not supported for account audit");
    }
    if bincode::serialized_size(transaction).context("size PumpAPI unsigned transaction")?
        > MAX_TRANSACTION_BYTES as u64
    {
        bail!("PumpAPI transaction exceeds {MAX_TRANSACTION_BYTES} bytes");
    }
    transaction
        .sanitize()
        .map_err(|_| anyhow::anyhow!("PumpAPI transaction failed Solana sanitization"))?;
    Ok(())
}
