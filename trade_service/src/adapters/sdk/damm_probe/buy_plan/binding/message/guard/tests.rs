use super::super::tests::{full, profile};
use super::*;

fn protocol(invocation: &Invocation) -> MiddlewareManager {
    let manager = invocation.middleware();
    manager
        .apply_middlewares_process_protocol_instructions(
            invocation.0.profile.protocol.clone(),
            "MeteoraDammV2",
            true,
        )
        .unwrap();
    manager
}

#[tokio::test]
async fn damm_buy_message_guard_rejects_protocol_funding_minimum_program_permission_and_extra_instructions(
) {
    for change in 0..9 {
        let p = profile(SwqosType::Temporal, false, true, false).await;
        let invocation = Invocation::new(p.clone());
        let manager = invocation.middleware();
        let mut actual = p.protocol.clone();
        let swap = actual.len() - 1;
        match change {
            0 => actual[swap].data[8] ^= 1,
            1 => actual[swap].data[16] ^= 1,
            2 => actual[swap].accounts[1].pubkey = Pubkey::new_from_array([99; 32]),
            3 => actual[swap].accounts[6].is_writable = true,
            4 => actual[swap].accounts[9].is_signer = true,
            5 => actual[swap].program_id = Pubkey::new_from_array([99; 32]),
            6 => actual[1].data[4] ^= 1,
            7 => actual.reverse(),
            _ => actual.push(actual[0].clone()),
        }
        assert!(manager
            .apply_middlewares_process_protocol_instructions(actual, "MeteoraDammV2", true)
            .is_err());
        assert!(manager
            .clone()
            .apply_middlewares_process_protocol_instructions(
                p.protocol.clone(),
                "MeteoraDammV2",
                true
            )
            .is_err());
        assert!(manager
            .apply_middlewares_process_full_instructions(full(&p), "MeteoraDammV2", true)
            .is_err());
    }
}

#[tokio::test]
async fn damm_buy_message_guard_rejects_final_tip_fee_order_and_additional_debit_mutations() {
    for change in 0..9 {
        let p = profile(SwqosType::Temporal, false, true, false).await;
        let invocation = Invocation::new(p.clone());
        let manager = protocol(&invocation);
        let mut actual = full(&p);
        match change {
            0 => actual[0].accounts[1].pubkey = Pubkey::new_from_array([99; 32]),
            1 => actual[0].accounts[0].pubkey = Pubkey::new_from_array([99; 32]),
            2 => actual[0].data[4] ^= 1,
            3 => actual[1].data[1] ^= 1,
            4 => actual[2].data[1] ^= 1,
            5 => actual.swap(1, 2),
            6 => {
                actual.remove(0);
            }
            7 => actual.push(actual[0].clone()),
            _ => actual[0].accounts[1].is_writable = false,
        }
        assert!(
            manager
                .apply_middlewares_process_full_instructions(actual, "MeteoraDammV2", true)
                .is_err(),
            "change {change}"
        );
        assert!(manager
            .apply_middlewares_process_full_instructions(full(&p), "MeteoraDammV2", true)
            .is_err());
    }
}

#[tokio::test]
async fn damm_buy_message_guard_requires_order_protocol_direction_and_single_callbacks_across_clones(
) {
    for change in 0..5 {
        let p = profile(SwqosType::Default, false, false, true).await;
        let invocation = Invocation::new(p.clone());
        let manager = invocation.middleware();
        match change {
            0 => assert!(manager
                .apply_middlewares_process_full_instructions(full(&p), "MeteoraDammV2", true)
                .is_err()),
            1 => assert!(manager
                .apply_middlewares_process_protocol_instructions(
                    p.protocol.clone(),
                    "MeteoraDammV2",
                    false
                )
                .is_err()),
            2 => assert!(manager
                .apply_middlewares_process_protocol_instructions(p.protocol.clone(), "Other", true)
                .is_err()),
            3 | 4 => {
                protocol(&invocation);
                if change == 3 {
                    assert!(manager
                        .clone()
                        .apply_middlewares_process_protocol_instructions(
                            p.protocol.clone(),
                            "MeteoraDammV2",
                            true
                        )
                        .is_err());
                } else {
                    manager
                        .apply_middlewares_process_full_instructions(
                            full(&p),
                            "MeteoraDammV2",
                            true,
                        )
                        .unwrap();
                    assert!(manager
                        .clone()
                        .apply_middlewares_process_full_instructions(
                            full(&p),
                            "MeteoraDammV2",
                            true
                        )
                        .is_err());
                }
            }
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn damm_buy_message_guard_cancellation_revokes_late_sdk_worker_callbacks() {
    let p = profile(SwqosType::Temporal, false, true, false).await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let copy = p.clone();
    let rpc = crate::adapters::rpc::damm_sell_simulation::tests::MemorySimulation::new().client();
    let task = tokio::spawn(async move {
        copy.execute(&rpc, |manager| async move {
            tx.send(manager).ok();
            std::future::pending().await
        })
        .await
    });
    let manager = rx.await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    let error = manager
        .apply_middlewares_process_protocol_instructions(p.protocol.clone(), "MeteoraDammV2", true)
        .unwrap_err();
    assert!(error.to_string().contains("cancelled or completed"));
}

#[tokio::test]
async fn damm_buy_message_guard_does_not_renew_expired_quote_before_final_callback() {
    let p = profile(SwqosType::Default, false, true, false).await;
    let invocation = Invocation::new(p.clone());
    let manager = protocol(&invocation);
    let actual = full(&p);
    tokio::time::sleep(std::time::Duration::from_millis(3100)).await;
    let error = manager
        .apply_middlewares_process_full_instructions(actual, "MeteoraDammV2", true)
        .unwrap_err();
    assert!(error.to_string().contains("expired"));
}

#[tokio::test]
async fn damm_buy_message_guard_rechecks_interior_fee_strategy_changes_before_sdk_callback() {
    let p = profile(SwqosType::Temporal, false, true, false).await;
    let invocation = Invocation::new(p.clone());
    let manager = protocol(&invocation);
    let actual = full(&p);
    p.request.gas_fee_strategy.set(
        SwqosType::Temporal,
        TradeType::Buy,
        GasFeeStrategyType::Normal,
        400_000,
        1_000,
        p.quote.intent.tip_sol,
    );
    assert!(manager
        .apply_middlewares_process_full_instructions(actual, "MeteoraDammV2", true)
        .is_err());
}

#[tokio::test]
async fn damm_buy_message_guard_concurrent_final_callbacks_allow_only_one_candidate() {
    let p = profile(SwqosType::Default, false, false, true).await;
    let invocation = Invocation::new(p.clone());
    let manager = protocol(&invocation);
    let start = Arc::new(std::sync::Barrier::new(2));
    let accepted = std::thread::scope(|scope| {
        let mut tasks = vec![];
        for manager in [manager.clone(), manager] {
            let start = start.clone();
            let actual = full(&p);
            tasks.push(scope.spawn(move || {
                start.wait();
                manager
                    .apply_middlewares_process_full_instructions(actual, "MeteoraDammV2", true)
                    .is_ok()
            }));
        }
        tasks
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|accepted| *accepted)
            .count()
    });
    assert_eq!(accepted, 1);
}

#[tokio::test]
async fn damm_buy_message_guard_preserves_all_signature_evidence_on_unattested_or_mismatched_reports(
) {
    let p = profile(SwqosType::Default, false, true, false).await;
    for frozen in [false, true] {
        let invocation = Invocation::new(p.clone());
        if frozen {
            protocol(&invocation)
                .apply_middlewares_process_full_instructions(full(&p), "MeteoraDammV2", true)
                .unwrap();
        }
        for signatures in [
            vec![],
            vec![Signature::from([7; 64]).to_string()],
            vec!["original-one".into(), "original-two".into()],
        ] {
            let error = invocation
                .finish(Ok(SdkLaneReport {
                    success: true,
                    signatures: signatures.clone(),
                    error: None,
                }))
                .err()
                .unwrap();
            assert_eq!(error.signatures, signatures);
            let error = invocation
                .finish(Err(SdkInvocationFailure {
                    signatures: signatures.clone(),
                    message: "original unknown".into(),
                }))
                .err()
                .unwrap();
            assert_eq!(error.signatures, signatures);
            assert_eq!(error.message, "original unknown");
        }
        let report = invocation
            .finish(Ok(SdkLaneReport {
                success: false,
                signatures: vec![],
                error: Some("original rejection".into()),
            }))
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert!(!report.success && report.signatures.is_empty());
    }
}
