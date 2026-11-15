use super::reconciliation::ChainCommitment;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Identity is independent of SDK types and of the ordinary TradeRequest ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SubmissionKey {
    pub wallet_id: String,
    pub idempotency_key: String,
}

impl SubmissionKey {
    pub fn new(wallet_id: &str, idempotency_key: &str) -> Result<Self> {
        if wallet_id.trim().is_empty() || wallet_id.len() > 128 {
            bail!("wallet_id is required and must be at most 128 characters");
        }
        if idempotency_key.trim().is_empty() || idempotency_key.len() > 128 {
            bail!("idempotency_key is required and must be at most 128 characters");
        }
        Ok(Self {
            wallet_id: wallet_id.into(),
            idempotency_key: idempotency_key.into(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SubmissionIntent {
    pub key: SubmissionKey,
    pub wallet_pubkey: String,
    pub quote_id: String,
    pub quote: Value,
}

impl SubmissionIntent {
    pub fn fingerprint(&self) -> Result<String> {
        // Explicit domain/version/mode separation; simulations never enter this journal.
        let payload = serde_json::json!({
            "schema_version": 1,
            "operation": "sol-buy-quote",
            "mode": "live",
            "intent": self,
        });
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&canonical_json(&payload))?)
        ))
    }
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut keys = fields.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical_json(&fields[key])))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubmissionStatus {
    Reserved,
    Signed,
    Submitted,
    Confirmed,
    Finalized,
    Unknown,
    Failed,
}

impl SubmissionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Signed => "signed",
            Self::Submitted => "submitted",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
            Self::Unknown => "unknown",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "reserved" => Self::Reserved,
            "signed" => Self::Signed,
            "submitted" => Self::Submitted,
            "confirmed" => Self::Confirmed,
            "finalized" => Self::Finalized,
            "unknown" => Self::Unknown,
            "failed" => Self::Failed,
            _ => bail!("invalid submission status"),
        })
    }
}

/// Do not derive Debug: signed bytes are an executable authorization.
pub(crate) struct SignedSubmission {
    pub signature: String,
    pub bytes: Vec<u8>,
}

pub(crate) struct SubmissionRecord {
    pub submission_id: String,
    pub intent: SubmissionIntent,
    pub status: SubmissionStatus,
    pub preflight: Value,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub commitment: Option<ChainCommitment>,
    pub observed_slot: Option<u64>,
}

pub(crate) enum Reservation {
    Created(SubmissionRecord),
    Existing(SubmissionRecord),
    Conflict,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_validate_lengths_without_trimming_away_identity() {
        for key in ["", "   ", &"x".repeat(129)] {
            assert!(SubmissionKey::new("wallet", key).is_err());
        }
        assert!(SubmissionKey::new("", "key").is_err());
        assert!(SubmissionKey::new(&"w".repeat(129), "key").is_err());
        assert_ne!(
            SubmissionKey::new("wallet", "key").unwrap(),
            SubmissionKey::new("wallet", " key ").unwrap()
        );
    }

    #[test]
    fn canonical_fingerprint_ignores_object_order_but_not_integer_precision_or_route_order() {
        let first = SubmissionIntent {
            key: SubmissionKey::new("wallet", "key").unwrap(),
            wallet_pubkey: "pubkey".into(),
            quote_id: "quote".into(),
            quote: serde_json::from_str(
                r#"{"amount":9007199254740993,"nested":{"b":2,"a":1},"route":[1,2]}"#,
            )
            .unwrap(),
        };
        let mut reordered = first.clone();
        reordered.quote = serde_json::from_str(
            r#"{"route":[1,2],"nested":{"a":1,"b":2},"amount":9007199254740993}"#,
        )
        .unwrap();
        assert_eq!(
            first.fingerprint().unwrap(),
            reordered.fingerprint().unwrap()
        );
        reordered.quote["amount"] = json!(9_007_199_254_740_992_u64);
        assert_ne!(
            first.fingerprint().unwrap(),
            reordered.fingerprint().unwrap()
        );
        reordered = first.clone();
        reordered.quote["route"] = json!([2, 1]);
        assert_ne!(
            first.fingerprint().unwrap(),
            reordered.fingerprint().unwrap()
        );
    }
}
