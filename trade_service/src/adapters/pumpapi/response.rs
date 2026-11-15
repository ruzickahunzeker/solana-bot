use anyhow::{bail, Context};
use base64::Engine;
use serde_json::Value;

use crate::sanitize::redact_urls;

pub(crate) const MAX_RESPONSE_BYTES: usize = 16 * 1024;
pub(crate) const MAX_TRANSACTION_BYTES: usize = 1232;
const MAX_BASE64_BYTES: usize = MAX_TRANSACTION_BYTES.div_ceil(3) * 4;

pub(crate) fn check_body_size(body: &[u8]) -> anyhow::Result<()> {
    if body.len() > MAX_RESPONSE_BYTES {
        bail!("PumpAPI transaction response exceeds {MAX_RESPONSE_BYTES} bytes");
    }
    Ok(())
}

/// Called only after exact raw decoding fails. Whitespace is text syntax, not binary padding.
pub(crate) fn decode_text_transaction(body: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    check_body_size(body)?;
    let trimmed = body.trim_ascii();
    if trimmed.is_empty() {
        bail!("PumpAPI returned an empty transaction response");
    }
    let bytes = match trimmed[0] {
        b'{' => {
            let value: Value =
                serde_json::from_slice(trimmed).context("parse PumpAPI JSON error response")?;
            bail!(
                "PumpAPI returned JSON instead of transaction bytes: {}",
                redact_urls(&value.to_string())
            );
        }
        b'[' => {
            let transactions: Vec<String> =
                serde_json::from_slice(trimmed).context("parse PumpAPI transaction bundle")?;
            if transactions.len() != 1 {
                bail!(
                    "PumpAPI returned {} transactions; first version supports exactly one",
                    transactions.len()
                );
            }
            decode_base64_transaction(&transactions[0])?
        }
        b'"' => {
            let transaction: String =
                serde_json::from_slice(trimmed).context("parse PumpAPI transaction string")?;
            decode_base64_transaction(&transaction)?
        }
        _ => return Ok(None),
    };
    Ok(Some(bytes))
}

fn decode_base64_transaction(value: &str) -> anyhow::Result<Vec<u8>> {
    if value.len() > MAX_BASE64_BYTES {
        bail!("PumpAPI base64 transaction exceeds {MAX_BASE64_BYTES} bytes");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .context("decode PumpAPI base64 transaction")?;
    if bytes.len() > MAX_TRANSACTION_BYTES {
        bail!("PumpAPI transaction exceeds {MAX_TRANSACTION_BYTES} bytes");
    }
    Ok(bytes)
}
