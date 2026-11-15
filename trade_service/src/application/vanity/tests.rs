use super::{test_support::*, *};
use std::{sync::Arc, time::Duration};

#[test]
fn validates_base58_vanity_patterns() {
    assert!(validate_pattern("9999").is_ok());
    assert!(validate_pattern("8888").is_ok());
    assert!(validate_pattern("0abc").is_err());
    assert!(validate_pattern("abcde").is_err());
}

#[test]
fn pattern_policy_keeps_original_ascii_alphabet_length_and_error_precedence() {
    for pattern in ["", "abcde", "\u{00e9}", "\u{1f600}", "0abcde"] {
        assert_eq!(
            validate_pattern(pattern).unwrap_err().to_string(),
            "pattern must contain 1..4 Base58 characters"
        );
    }
    for pattern in ["0", "O", "I", "l", " a", "a ", "a\n", "ab_", "-9"] {
        assert_eq!(
            validate_pattern(pattern).unwrap_err().to_string(),
            "pattern contains invalid Base58 characters (0, O, I and l are not allowed)"
        );
    }
    for character in "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".chars() {
        for len in 1..=4 {
            assert!(validate_pattern(&character.to_string().repeat(len)).is_ok());
        }
    }
}

#[test]
fn prepared_command_clamps_only_attempt_budget_trims_label_and_preserves_raw_owner_and_side() {
    for (limit, expected) in [
        (None, 30_000_000),
        (Some(0), 1),
        (Some(1), 1),
        (Some(999), 999),
        (Some(30_000_000), 30_000_000),
        (Some(30_000_001), 30_000_000),
        (Some(u64::MAX), 30_000_000),
    ] {
        for side in [VanityMatchSide::Prefix, VanityMatchSide::Suffix] {
            let expected_side = side.as_str();
            let command = prepare(VanityCreateInput {
                owner_type: " telegram ".into(),
                owner_id: " 42 ".into(),
                label: "\u{00a0}label \n".into(),
                match_side: side,
                pattern: "9Abz".into(),
                max_attempts: limit,
            })
            .unwrap();
            assert_eq!(command.owner_type(), " telegram ");
            assert_eq!(command.owner_id(), " 42 ");
            assert_eq!(command.label(), "label");
            assert_eq!(command.match_side(), expected_side);
            assert_eq!(command.pattern(), "9Abz");
            assert_eq!(command.max_attempts(), expected);
        }
    }
}

#[test]
fn cancellation_policy_requires_exact_owner_before_any_active_or_terminal_status() {
    let worker = Worker::new();
    let mut job = worker.state.lock().unwrap().job.clone();
    for status in [
        "queued",
        "running",
        "cancel_requested",
        "completed",
        "failed",
        "cancelled",
        "exhausted",
        "interrupted",
        "unexpected",
    ] {
        job.status = status.into();
        assert_eq!(
            cancellation_allowed(&job, "telegram", "42").unwrap(),
            ["queued", "running", "cancel_requested"].contains(&status)
        );
        for (owner_type, owner_id) in [
            ("telegram", "99"),
            (" Telegram", "42"),
            ("system", "global"),
            ("telegram", "42 "),
        ] {
            assert_eq!(
                cancellation_allowed(&job, owner_type, owner_id)
                    .unwrap_err()
                    .to_string(),
                "wallet generation job does not belong to this owner"
            );
        }
    }
}

#[tokio::test]
async fn create_uses_one_validated_owned_command_and_rejects_invalid_patterns_before_worker() {
    let worker = Worker::new();
    let service = VanityService::new(worker.clone());
    for pattern in ["abcde", "0", ""] {
        let mut request = input();
        request.pattern = pattern.into();
        assert_eq!(
            service.create(request).await.unwrap_err().kind,
            VanityFailureKind::InvalidRequest
        );
        assert!(worker.events().is_empty());
    }
    let job = service.create(input()).await.unwrap();
    assert_eq!(job.label, "new vanity");
    assert_eq!(job.max_attempts, 30_000_000);
    assert_eq!(worker.events(), ["create"]);
    assert_eq!(
        worker.state.lock().unwrap().prepared,
        [PreparedView {
            owner_type: "telegram".into(),
            owner_id: "42".into(),
            label: "new vanity".into(),
            match_side: "prefix".into(),
            pattern: "9999".into(),
            max_attempts: 30_000_000
        }]
    );
}

#[tokio::test]
async fn get_and_cancel_keep_literal_context_single_port_call_and_distinct_owner_error_contracts() {
    let worker = Worker::new();
    let service = VanityService::new(worker.clone());
    assert_eq!(
        service
            .get(" literal-id ", "telegram", "42")
            .await
            .unwrap()
            .job_id,
        "public-job"
    );
    let denied = service
        .get(" literal-id ", "system", "global")
        .await
        .unwrap_err();
    assert_eq!(denied.kind, VanityFailureKind::OwnerMismatch);
    assert_eq!(denied.message, "wallet generation job owner mismatch");
    let denied = service
        .cancel(" literal-id ", "system", "global")
        .await
        .unwrap_err();
    assert_eq!(denied.kind, VanityFailureKind::InvalidRequest);
    assert_eq!(
        denied.message,
        "wallet generation job does not belong to this owner"
    );
    assert_eq!(
        service
            .cancel(" literal-id ", "telegram", "42")
            .await
            .unwrap()
            .status,
        "cancel_requested"
    );
    assert_eq!(
        worker.events(),
        [
            "get: literal-id ",
            "get: literal-id ",
            "cancel: literal-id :system:global",
            "cancel: literal-id :telegram:42"
        ]
    );
}

#[tokio::test]
async fn worker_failures_keep_create_cancel_bad_request_and_get_not_found_without_retries() {
    for op in ["create", "get", "cancel"] {
        let worker = Worker::new();
        worker.state.lock().unwrap().failure = Some(op);
        let service = VanityService::new(worker.clone());
        let result = match op {
            "create" => service.create(input()).await,
            "get" => service.get("id", "telegram", "42").await,
            "cancel" => service.cancel("id", "telegram", "42").await,
            _ => unreachable!(),
        };
        let error = result.unwrap_err();
        assert_eq!(
            error.kind,
            if op == "get" {
                VanityFailureKind::NotFound
            } else {
                VanityFailureKind::InvalidRequest
            }
        );
        assert_eq!(error.message, format!("synthetic {op} failure"));
        assert_eq!(worker.events().len(), 1);
    }
}

#[tokio::test]
async fn cancelling_a_request_does_not_imply_the_queued_job_was_cancelled_or_released() {
    let mut worker = Worker::new();
    let gate = Arc::new(tokio::sync::Notify::new());
    worker.create_gate = Some(gate.clone());
    let service = Arc::new(VanityService::new(worker.clone()));
    let task = tokio::spawn(async move { service.create(input()).await });
    tokio::time::timeout(Duration::from_secs(1), gate.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(worker.state.lock().unwrap().job.status, "queued");
    assert_eq!(worker.events(), ["create"]);
}
