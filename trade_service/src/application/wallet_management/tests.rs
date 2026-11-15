use super::{test_support::*, *};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

fn service(backend: &Backend) -> WalletManagementService<Backend, Backend> {
    WalletManagementService::new(backend.clone(), backend.clone())
}

#[test]
fn loaded_and_managed_views_are_different_public_snapshots() {
    let backend = Backend::new();
    let view = service(&backend).loaded();
    assert_eq!(view.default_wallet_id, " runtime-default ");
    assert_eq!(view.wallets[0].wallet_id, "runtime-wallet");
    assert_eq!(backend.events(), ["loaded"]);
}

#[tokio::test]
async fn managed_list_keeps_disabled_wallets_repository_order_and_false_created() {
    let backend = Backend::new();
    backend.state.lock().unwrap().wallet.enabled = false;
    let result = service(&backend).managed().await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].wallet.wallet_id, "managed-wallet");
    assert!(!result[0].wallet.enabled);
    assert!(!result[0].created);
    assert_eq!(backend.events(), ["list"]);
}

#[tokio::test]
async fn creation_keeps_raw_identity_and_label_and_passes_one_owned_opaque_material_to_install() {
    let backend = Backend::new();
    let view = service(&backend).create(input(None)).await.unwrap();
    assert!(view.created);
    assert_eq!(view.wallet.owner_type, " telegram ");
    assert_eq!(view.wallet.owner_id, " 42 ");
    assert_eq!(view.wallet.label, " new label ");
    assert_eq!(backend.events(), ["create", "install:managed-wallet"]);
    assert_eq!(backend.drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        backend.state.lock().unwrap().inputs,
        [(
            " telegram ".into(),
            " 42 ".into(),
            " new label ".into(),
            None
        )]
    );
}

#[tokio::test]
async fn import_preserves_existing_or_restored_created_flag_and_does_not_trim_input_material() {
    for secret in ["", " ", " fake-import-not-a-key "] {
        for created in [false, true] {
            let backend = Backend::new();
            backend.state.lock().unwrap().import_created = created;
            let view = service(&backend).create(input(Some(secret))).await.unwrap();
            assert_eq!(view.created, created);
            assert_eq!(backend.events(), ["import", "install:managed-wallet"]);
            assert_eq!(
                backend.state.lock().unwrap().inputs[0].3.as_deref(),
                Some(secret)
            );
            assert_eq!(backend.drops.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn disabled_result_does_not_install_but_still_drops_owned_material() {
    for secret in [None, Some("fake-import-not-a-key")] {
        let backend = Backend::new();
        backend.state.lock().unwrap().wallet.enabled = false;
        let view = service(&backend).create(input(secret)).await.unwrap();
        assert!(!view.wallet.enabled);
        assert_eq!(
            backend.events(),
            [if secret.is_some() { "import" } else { "create" }]
        );
        assert_eq!(backend.drops.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn repository_errors_keep_original_kinds_and_do_not_enter_a_runtime_path() {
    for op in ["list", "create", "import", "rename", "disable", "export"] {
        let backend = Backend::new();
        backend.fail(op);
        let service = service(&backend);
        let result = match op {
            "list" => service.managed().await.map(|_| ()),
            "create" => service.create(input(None)).await.map(|_| ()),
            "import" => service
                .create(input(Some("fake-import-not-a-key")))
                .await
                .map(|_| ()),
            "rename" => service.rename("managed-wallet", " x ").await.map(|_| ()),
            "disable" => service.delete("managed-wallet").await.map(|_| ()),
            "export" => service.export("managed-wallet").await.map(|_| ()),
            _ => unreachable!(),
        };
        let error = result.unwrap_err();
        assert_eq!(
            error.kind,
            if op == "list" {
                WalletFailureKind::Unavailable
            } else {
                WalletFailureKind::InvalidRequest
            }
        );
        assert_eq!(error.message, format!("synthetic {op} failure"));
        assert_eq!(backend.events().len(), 1);
        assert_eq!(backend.drops.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn install_failure_only_compensates_new_or_restored_wallets_and_keeps_original_error() {
    for (secret, created) in [
        (None, true),
        (Some("fake-import-not-a-key"), false),
        (Some("fake-import-not-a-key"), true),
    ] {
        for compensation_failed in [false, true] {
            let backend = Backend::new();
            backend.state.lock().unwrap().import_created = created;
            backend.fail("install");
            if compensation_failed {
                backend.fail("disable");
            }
            let error = service(&backend).create(input(secret)).await.unwrap_err();
            assert_eq!(error.kind, WalletFailureKind::LoadFailed);
            assert_eq!(error.message, "synthetic install failure");
            let mut events = vec![
                if secret.is_some() { "import" } else { "create" },
                "install:managed-wallet",
            ];
            if created {
                events.push("disable:managed-wallet");
            }
            assert_eq!(backend.events(), events);
            assert_eq!(backend.drops.load(Ordering::SeqCst), 1);
            assert_eq!(
                backend.state.lock().unwrap().wallet.enabled,
                !created || compensation_failed
            );
        }
    }
}

#[tokio::test]
async fn rename_trims_only_label_and_delete_keeps_disable_before_runtime_remove() {
    let backend = Backend::new();
    let renamed = service(&backend)
        .rename(" literal-id ", "  renamed  ")
        .await
        .unwrap();
    assert_eq!(renamed.wallet.label, "renamed");
    assert!(!renamed.created);
    let deleted = service(&backend).delete(" literal-id ").await.unwrap();
    assert!(!deleted.wallet.enabled);
    assert!(!deleted.created);
    assert_eq!(
        backend.events(),
        [
            "rename: literal-id :renamed",
            "disable: literal-id ",
            "remove: literal-id "
        ]
    );
}

#[tokio::test]
async fn default_delete_failure_and_partial_runtime_remove_failure_are_not_silently_rolled_back() {
    let backend = Backend::new();
    backend.state.lock().unwrap().wallet.is_default = true;
    let error = service(&backend)
        .delete("managed-wallet")
        .await
        .unwrap_err();
    assert_eq!(error.kind, WalletFailureKind::InvalidRequest);
    assert_eq!(error.message, "default wallet cannot be deleted");
    assert_eq!(backend.events(), ["disable:managed-wallet"]);
    assert!(backend.state.lock().unwrap().wallet.enabled);
    let backend = Backend::new();
    backend.fail("remove");
    let error = service(&backend)
        .delete("managed-wallet")
        .await
        .unwrap_err();
    assert_eq!(error.kind, WalletFailureKind::RemoveFailed);
    assert!(!backend.state.lock().unwrap().wallet.enabled);
    assert_eq!(
        backend.events(),
        ["disable:managed-wallet", "remove:managed-wallet"]
    );
}

#[tokio::test]
async fn explicit_export_keeps_literal_requested_id_and_is_the_only_secret_output() {
    let backend = Backend::new();
    let result = service(&backend).export(" literal-id ").await.unwrap();
    assert_eq!(result.wallet_id, " literal-id ");
    assert_eq!(result.private_key_base58, EXPORT_MARKER);
    assert_eq!(backend.events(), ["export: literal-id "]);
    assert_eq!(backend.drops.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_drops_opaque_material_without_claiming_a_completed_disable_or_rollback() {
    let mut backend = Backend::new();
    let gate = Arc::new(tokio::sync::Notify::new());
    backend.install_gate = Some(gate.clone());
    let service = Arc::new(service(&backend));
    let task = tokio::spawn(async move { service.create(input(None)).await });
    tokio::time::timeout(Duration::from_secs(1), gate.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(backend.drops.load(Ordering::SeqCst), 1);
    assert!(backend.state.lock().unwrap().wallet.enabled);
    assert_eq!(backend.events(), ["create", "install:managed-wallet"]);
}
