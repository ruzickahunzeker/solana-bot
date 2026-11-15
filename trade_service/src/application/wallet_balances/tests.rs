use super::{test_support::*, *};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

fn wallet(
    id: &str,
    pubkey: &str,
    owner_type: &str,
    owner_id: &str,
    enabled: bool,
) -> WalletMetadata {
    WalletMetadata {
        wallet_id: id.into(),
        pubkey: pubkey.into(),
        owner_type: owner_type.into(),
        owner_id: owner_id.into(),
        enabled,
    }
}

fn metadata(trace: Trace) -> Metadata {
    Metadata {
        trace,
        error: None,
        wallets: vec![
            wallet("owned", "pub-2", "telegram", "42", true),
            wallet("disabled", "invalid", "telegram", "42", false),
            wallet("other", "invalid", "telegram", "99", true),
            wallet("global", "pub-1", "system", "global", true),
        ],
    }
}

fn service(metadata: Metadata, runtime: Runtime, trace: Trace) -> impl WalletBalanceQueries {
    WalletBalanceService::new(metadata, runtime, Parser(trace.clone()), move || {
        trace.lock().unwrap().push("clock".into());
        123
    })
}

#[tokio::test]
async fn quote_uses_literal_wallet_identity_and_exact_integer_balance_after_ordered_read_gates() {
    let trace: Trace = Arc::default();
    let mut metadata = metadata(trace.clone());
    metadata.wallets[0].wallet_id = " owned ".into();
    let service = service(metadata, Runtime::new(trace.clone()), trace.clone());
    let view = service
        .quote(" owned ", "mint", "telegram", "42")
        .await
        .unwrap();
    assert_eq!(view.wallet_id, " owned ");
    assert_eq!(view.quote_mint, Address::from_bytes([9; 32]));
    assert_eq!(
        view.balance,
        QuoteBalance {
            decimals: 6,
            balance_units: u64::MAX,
            balance_lamports: 42
        }
    );
    assert_eq!(
        *trace.lock().unwrap(),
        vec![
            "get: owned ".into(),
            "parse:mint".into(),
            "reader: owned ".into(),
            format!("quote:{}", Address::from_bytes([9; 32]))
        ]
    );
}

#[tokio::test]
async fn quote_visibility_preserves_enabled_and_exact_owner_or_system_global_rules() {
    for (owner_type, owner_id, enabled, allowed) in [
        ("telegram", "42", true, true),
        ("telegram", "42", false, false),
        ("telegram", "99", true, false),
        ("other", "42", true, false),
        ("system", "global", true, true),
        ("system", "global", false, false),
        ("system", "42", true, false),
        ("telegram", "global", true, false),
        (" telegram", "42", true, false),
        ("telegram", "42 ", true, false),
    ] {
        let trace: Trace = Arc::default();
        let metadata = Metadata {
            wallets: vec![wallet("owned", "pub-2", owner_type, owner_id, enabled)],
            error: None,
            trace: trace.clone(),
        };
        let service = service(metadata, Runtime::new(trace.clone()), trace.clone());
        let result = service.quote("owned", "mint", "telegram", "42").await;
        assert_eq!(result.is_ok(), allowed);
        if !allowed {
            assert_eq!(result.unwrap_err().kind, BalanceFailureKind::OwnerMismatch);
            assert_eq!(*trace.lock().unwrap(), ["get:owned"]);
        }
    }
}

#[tokio::test]
async fn quote_error_precedence_and_only_backend_url_redaction_preserve_v1_contract() {
    for (stage, kind, message, events) in [
        (
            "metadata",
            BalanceFailureKind::NotFound,
            "synthetic metadata failure",
            vec!["get:owned"],
        ),
        (
            "missing",
            BalanceFailureKind::NotFound,
            "wallet not found",
            vec!["get:missing"],
        ),
        (
            "parse",
            BalanceFailureKind::InvalidMint,
            "invalid quote mint",
            vec!["get:owned", "parse:invalid"],
        ),
        (
            "loaded",
            BalanceFailureKind::Unavailable,
            "wallet is not loaded",
            vec!["get:owned", "parse:mint", "reader:owned"],
        ),
    ] {
        let trace: Trace = Arc::default();
        let mut metadata = metadata(trace.clone());
        let mut runtime = Runtime::new(trace.clone());
        if stage == "metadata" {
            metadata.error = Some("synthetic metadata failure");
        }
        if stage == "loaded" {
            runtime.loaded = false;
        }
        let service = service(metadata, runtime, trace.clone());
        let error = service
            .quote(
                if stage == "missing" {
                    "missing"
                } else {
                    "owned"
                },
                if stage == "parse" { "invalid" } else { "mint" },
                "telegram",
                "42",
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.message, message);
        assert_eq!(*trace.lock().unwrap(), events);
    }
    let trace: Trace = Arc::default();
    let mut runtime = Runtime::new(trace.clone());
    runtime.quote_error = Some("synthetic RPC failure https://rpc.example.invalid/private-token");
    let service = service(metadata(trace.clone()), runtime, trace.clone());
    let error = service
        .quote("owned", "mint", "telegram", "42")
        .await
        .unwrap_err();
    assert_eq!(error.kind, BalanceFailureKind::Backend);
    assert!(!error.message.contains("private-token"));
    assert!(!trace.lock().unwrap().iter().any(|event| event == "clock"));
}

#[tokio::test]
async fn native_reads_only_visible_addresses_in_repository_order_and_timestamps_after_rpc() {
    let trace: Trace = Arc::default();
    let mut runtime = Runtime::new(trace.clone());
    runtime.balances = vec![11, 22];
    let inputs = runtime.inputs.clone();
    let service = service(metadata(trace.clone()), runtime, trace.clone());
    let result = service.native("telegram", "42").await.unwrap();
    assert_eq!(result.fetched_at_ms, 123);
    assert_eq!(
        result
            .wallets
            .iter()
            .map(|wallet| (
                wallet.wallet_id.as_str(),
                wallet.pubkey.as_str(),
                wallet.balance_lamports,
                wallet.error.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("owned", "pub-2", Some(11), None),
            ("global", "pub-1", Some(22), None),
        ]
    );
    assert_eq!(
        *inputs.lock().unwrap(),
        [vec![
            Address::from_bytes([2; 32]),
            Address::from_bytes([1; 32])
        ]]
    );
    assert_eq!(
        *trace.lock().unwrap(),
        [
            "list",
            "parse:pub-2",
            "parse:pub-1",
            "default",
            "batch:2",
            "clock"
        ]
    );
}

#[tokio::test]
async fn native_metadata_failures_and_first_visible_parse_error_never_obtain_a_reader() {
    for repository_failed in [true, false] {
        let trace: Trace = Arc::default();
        let mut metadata = metadata(trace.clone());
        if repository_failed {
            metadata.error = Some("synthetic repository failure");
        } else {
            metadata.wallets[0].pubkey = "invalid".into();
        }
        let service = service(metadata, Runtime::new(trace.clone()), trace.clone());
        let error = service.native("telegram", "42").await.unwrap_err();
        assert_eq!(
            error.kind,
            if repository_failed {
                BalanceFailureKind::Unavailable
            } else {
                BalanceFailureKind::InvalidMetadata
            }
        );
        assert_eq!(
            *trace.lock().unwrap(),
            if repository_failed {
                vec!["list"]
            } else {
                vec!["list", "parse:invalid"]
            }
        );
    }
}

#[tokio::test]
async fn native_backend_failure_is_an_ok_snapshot_with_original_per_wallet_error_text() {
    let trace: Trace = Arc::default();
    let mut runtime = Runtime::new(trace.clone());
    let message = "synthetic RPC failure https://rpc.example.invalid/non-secret-marker";
    runtime.native_error = Some(message);
    let service = service(metadata(trace.clone()), runtime, trace.clone());
    let result = service.native("telegram", "42").await.unwrap();
    assert_eq!(result.fetched_at_ms, 123);
    assert_eq!(result.wallets.len(), 2);
    assert!(result.wallets.iter().all(
        |wallet| wallet.balance_lamports.is_none() && wallet.error.as_deref() == Some(message)
    ));
    assert_eq!(trace.lock().unwrap().last().unwrap(), "clock");
}

#[tokio::test]
async fn native_balance_cardinality_mismatch_keeps_every_visible_wallet_as_an_error() {
    for balances in [vec![], vec![7], vec![7, 8, 9]] {
        let trace: Trace = Arc::default();
        let mut runtime = Runtime::new(trace.clone());
        runtime.balances = balances;
        let service = service(metadata(trace.clone()), runtime, trace.clone());
        let result = service.native("telegram", "42").await.unwrap();
        assert_eq!(result.wallets.len(), 2, "RPC cannot drop visible wallets");
        assert_eq!(result.wallets[0].wallet_id, "owned");
        assert_eq!(result.wallets[1].wallet_id, "global");
        assert!(result
            .wallets
            .iter()
            .all(|row| row.balance_lamports.is_none()
                && row.error.as_deref()
                    == Some("wallet balances unavailable: RPC account count mismatch")));
        assert_eq!(trace.lock().unwrap().last().unwrap(), "clock");
    }
}

#[tokio::test]
async fn native_valid_cardinality_and_empty_queries_preserve_order_and_clock_contract() {
    for values in [vec![], vec![7], vec![7, 8], vec![7, 8, 9]] {
        let trace: Trace = Arc::default();
        let mut runtime = Runtime::new(trace.clone());
        runtime.balances = values.clone();
        let service = service(metadata(trace.clone()), runtime, trace.clone());
        let result = service.native("telegram", "42").await.unwrap();
        assert_eq!(result.wallets.len(), 2);
        if values.len() == 2 {
            assert_eq!(
                result
                    .wallets
                    .iter()
                    .map(|row| row.balance_lamports.unwrap())
                    .collect::<Vec<_>>(),
                values
            );
        } else {
            assert!(result
                .wallets
                .iter()
                .all(|row| row.balance_lamports.is_none() && row.error.is_some()));
        }
    }
    let trace: Trace = Arc::default();
    let service = service(
        Metadata {
            wallets: vec![],
            error: None,
            trace: trace.clone(),
        },
        Runtime::new(trace.clone()),
        trace.clone(),
    );
    assert!(service
        .native("telegram", "42")
        .await
        .unwrap()
        .wallets
        .is_empty());
    assert_eq!(
        *trace.lock().unwrap(),
        ["list", "default", "batch:0", "clock"]
    );
}

#[tokio::test]
async fn cancelling_a_balance_read_drops_the_reader_future_without_clock_retry_or_background_work()
{
    for native in [true, false] {
        let trace: Trace = Arc::default();
        let mut runtime = Runtime::new(trace.clone());
        let gate = Arc::new(Gate::default());
        runtime.gate = Some(gate.clone());
        let service = Arc::new(service(metadata(trace.clone()), runtime, trace.clone()));
        let task = tokio::spawn(async move {
            if native {
                service.native("telegram", "42").await.map(|_| ())
            } else {
                service
                    .quote("owned", "mint", "telegram", "42")
                    .await
                    .map(|_| ())
            }
        });
        tokio::time::timeout(Duration::from_secs(1), gate.entered.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(gate.dropped.load(Ordering::SeqCst));
        let events = trace.lock().unwrap();
        assert!(!events.iter().any(|event| event == "clock"));
        assert_eq!(
            events
                .iter()
                .filter(|event| event.starts_with("batch:") || event.starts_with("quote:"))
                .count(),
            1
        );
    }
}
