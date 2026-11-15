use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn existing_executor_status_mapping_fingerprint() {
    let mut rows = Vec::new();
    for success in [false, true] {
        for signature in [None, Some("signature"), Some("")] {
            for error in [
                None,
                Some("warning"),
                Some("landed with transaction error: Custom(1)"),
                Some("submit signature not visible"),
                Some("sendTransaction failed: rejected"),
                Some("SendTransaction failed"),
                Some("prefix sendTransaction failed suffix"),
                Some(""),
            ] {
                for simulate in [false, true] {
                    let (status, actual_signature, actual_error) = map_status(
                        Ok(ExecuteResult {
                            success,
                            signature: signature.map(str::to_owned),
                            error: error.map(str::to_owned),
                        }),
                        simulate,
                    );
                    rows.push(json!({"input":{"success":success,"signature":signature,"error":error,"simulate":simulate},
                        "output":{"status":status,"signature":actual_signature,"error":actual_error}}));
                }
            }
        }
    }
    for simulate in [false, true] {
        let (status, signature, error) =
            map_status(Err(anyhow::anyhow!("outer").context("context")), simulate);
        rows.push(json!({"input":{"failure":"context","simulate":simulate},"output":{"status":status,"signature":signature,"error":error}}));
    }
    assert_eq!(rows.len(), 98);
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&rows).unwrap()));
    assert_eq!(
        hash,
        "017be4bb035ed56cdf073b646057ba8d8c92407a144e979d5f6be5c77d3f5227"
    );
}

#[test]
fn existing_executor_does_not_promote_unknown_signature_outcomes() {
    for simulate in [false, true] {
        let (status, signature, error) = map_status(
            Ok(ExecuteResult {
                success: false,
                signature: Some("original".into()),
                error: Some("timeout".into()),
            }),
            simulate,
        );
        assert_eq!(status, TradeStatus::Unknown);
        assert_eq!(signature.as_deref(), Some("original"));
        assert_eq!(error.as_deref(), Some("timeout"));
    }
    let (status, signature, _) = map_status(
        Ok(ExecuteResult {
            success: true,
            signature: None,
            error: None,
        }),
        false,
    );
    assert_eq!(status, TradeStatus::Unknown);
    assert!(signature.is_none());
}

#[test]
fn existing_executor_preserves_known_failure_markers_and_original_signature() {
    for marker in [
        "landed with transaction error",
        "submit signature not visible",
        "sendTransaction failed",
    ] {
        let (status, signature, error) = map_status(
            Ok(ExecuteResult {
                success: false,
                signature: Some("original".into()),
                error: Some(marker.into()),
            }),
            false,
        );
        assert_eq!(status, TradeStatus::Failed);
        assert_eq!(signature.as_deref(), Some("original"));
        assert_eq!(error.as_deref(), Some(marker));
    }
}
