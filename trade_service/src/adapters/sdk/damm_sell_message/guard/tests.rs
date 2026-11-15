use super::*;
use crate::adapters::sdk::damm_sell_message::candidate::tip_accounts;
use crate::adapters::sdk::damm_sell_message::tests::{fixture, key};
use sol_trade_sdk::{
    constants::swqos::NOZOMI_TIP_ACCOUNTS,
    trading::{
        common::extend_compute_budget_instructions, core::transaction_pool::compile_message,
    },
};
use solana_sdk::{message::v1, transaction::VersionedTransaction};

fn full(profile: &DammSellMessageProfile) -> Vec<Instruction> {
    let mut instructions = vec![];
    if !profile.intent.simulate
        && profile.context.provider != SwqosType::Default
        && profile.intent.tip_sol > 0.0
    {
        let lamports = (profile.intent.tip_sol * 1_000_000_000.0)
            .min(u64::MAX as f64)
            .round() as u64;
        instructions.push(solana_system_interface::instruction::transfer(
            &profile.context.wallet,
            &tip_accounts(profile.context.provider)[0],
            lamports,
        ));
    }
    extend_compute_budget_instructions(&mut instructions, 1_000, 300_000);
    instructions.extend_from_slice(&profile.protocol);
    instructions
}

fn protocol(invocation: &Invocation) -> MiddlewareManager {
    let manager = invocation.middleware();
    let actual = invocation.0.profile.protocol.clone();
    assert_eq!(
        manager
            .apply_middlewares_process_protocol_instructions(actual.clone(), "MeteoraDammV2", false)
            .unwrap(),
        actual
    );
    manager
}

#[test]
fn damm_sell_message_guard_passes_original_instructions_and_freezes_compiled_bytes() {
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        for simulate in [false, true] {
            let profile = fixture(provider, simulate, 100).profile().unwrap();
            let invocation = Invocation::new(profile.clone());
            let manager = protocol(&invocation);
            let full = full(&profile);
            assert_eq!(
                manager
                    .apply_middlewares_process_full_instructions(
                        full.clone(),
                        "MeteoraDammV2",
                        false
                    )
                    .unwrap(),
                full
            );
            let expected = solana_sdk::message::VersionedMessage::Legacy(
                solana_sdk::message::Message::new_with_blockhash(
                    &full,
                    Some(&profile.context.wallet),
                    &profile.intent.recent_blockhash,
                ),
            )
            .serialize();
            let stage = invocation.0.stage.lock().unwrap();
            let Stage::Frozen(candidate) = &*stage else {
                panic!("candidate was not frozen")
            };
            assert_eq!(candidate.message_bytes(), expected);
            let offset = usize::from(provider != SwqosType::Default && !simulate);
            assert_eq!(
                full[offset].data,
                [vec![3], 1_000u64.to_le_bytes().to_vec()].concat()
            );
            assert_eq!(
                full[offset + 1].data,
                [vec![2], 300_000u32.to_le_bytes().to_vec()].concat()
            );
        }
    }
}

#[test]
fn damm_sell_message_guard_provider_destinations_are_selected_without_network_or_default_tip() {
    for provider in [
        SwqosType::Jito,
        SwqosType::NextBlock,
        SwqosType::ZeroSlot,
        SwqosType::Temporal,
        SwqosType::Bloxroute,
        SwqosType::Node1,
        SwqosType::FlashBlock,
        SwqosType::BlockRazor,
        SwqosType::Astralane,
        SwqosType::Stellium,
        SwqosType::Lightspeed,
        SwqosType::Soyas,
        SwqosType::Speedlanding,
        SwqosType::Helius,
        SwqosType::Solami,
        SwqosType::LunarLander,
        SwqosType::Glaive,
    ] {
        let profile = fixture(provider, false, 50).profile().unwrap();
        for destination in tip_accounts(provider) {
            let invocation = Invocation::new(profile.clone());
            let manager = protocol(&invocation);
            let mut instructions = full(&profile);
            instructions[0] = solana_system_interface::instruction::transfer(
                &profile.context.wallet,
                destination,
                1_000_000,
            );
            manager
                .apply_middlewares_process_full_instructions(instructions, "MeteoraDammV2", false)
                .unwrap();
        }
    }
    assert!(tip_accounts(SwqosType::Default).is_empty());
}

#[test]
fn damm_sell_message_guard_rejects_protocol_data_account_flag_order_and_appended_changes() {
    for change in 0..7 {
        let profile = fixture(SwqosType::Temporal, false, 100).profile().unwrap();
        let invocation = Invocation::new(profile.clone());
        let manager = invocation.middleware();
        let mut instructions = profile.protocol.clone();
        match change {
            0 => instructions[1].data[16] ^= 1,
            1 => instructions[1].accounts[1].pubkey = key(30),
            2 => instructions[1].accounts[6].is_writable = true,
            3 => instructions[1].accounts[9].is_signer = true,
            4 => instructions.reverse(),
            5 => instructions.push(instructions[0].clone()),
            6 => instructions[1].program_id = key(30),
            _ => unreachable!(),
        }
        assert!(manager
            .apply_middlewares_process_protocol_instructions(instructions, "MeteoraDammV2", false)
            .is_err());
        assert!(manager
            .apply_middlewares_process_protocol_instructions(
                profile.protocol.clone(),
                "MeteoraDammV2",
                false
            )
            .is_err());
        assert!(manager
            .apply_middlewares_process_full_instructions(full(&profile), "MeteoraDammV2", false)
            .is_err());
    }
}

#[test]
fn damm_sell_message_guard_rejects_tip_budget_extra_instructions_and_fee_order_changes() {
    for change in 0..8 {
        let profile = fixture(SwqosType::Temporal, false, 50).profile().unwrap();
        let invocation = Invocation::new(profile.clone());
        let manager = protocol(&invocation);
        let mut instructions = full(&profile);
        match change {
            0 => instructions[0].accounts[1].pubkey = key(30),
            1 => instructions[0].data[4] ^= 1,
            2 => instructions[1].data[1] ^= 1,
            3 => instructions[2].data[1] ^= 1,
            4 => instructions.swap(1, 2),
            5 => instructions.push(instructions[1].clone()),
            6 => {
                instructions.remove(0);
            }
            7 => instructions[0].accounts[0].pubkey = key(30),
            _ => unreachable!(),
        }
        assert!(
            manager
                .apply_middlewares_process_full_instructions(instructions, "MeteoraDammV2", false)
                .is_err(),
            "change {change}"
        );
    }
}

#[test]
fn damm_sell_message_guard_has_ordered_single_use_callbacks_shared_across_clones() {
    for change in 0..5 {
        let profile = fixture(SwqosType::Default, true, 50).profile().unwrap();
        let invocation = Invocation::new(profile.clone());
        let manager = invocation.middleware();
        match change {
            0 => assert!(manager
                .apply_middlewares_process_full_instructions(full(&profile), "MeteoraDammV2", false)
                .is_err()),
            1 => assert!(manager
                .apply_middlewares_process_protocol_instructions(
                    profile.protocol.clone(),
                    "MeteoraDammV2",
                    true
                )
                .is_err()),
            2 => assert!(manager
                .apply_middlewares_process_protocol_instructions(
                    profile.protocol.clone(),
                    "Other",
                    false
                )
                .is_err()),
            3 | 4 => {
                manager
                    .apply_middlewares_process_protocol_instructions(
                        profile.protocol.clone(),
                        "MeteoraDammV2",
                        false,
                    )
                    .unwrap();
                let clone = manager.clone();
                if change == 3 {
                    assert!(clone
                        .apply_middlewares_process_protocol_instructions(
                            profile.protocol.clone(),
                            "MeteoraDammV2",
                            false
                        )
                        .is_err());
                } else {
                    manager
                        .apply_middlewares_process_full_instructions(
                            full(&profile),
                            "MeteoraDammV2",
                            false,
                        )
                        .unwrap();
                    assert!(clone
                        .apply_middlewares_process_full_instructions(
                            full(&profile),
                            "MeteoraDammV2",
                            false
                        )
                        .is_err());
                }
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn damm_sell_message_guard_checks_compiled_signers_and_wire_size_even_for_bad_expected_profiles() {
    for extra_signer in [false, true] {
        let mut profile = fixture(SwqosType::Default, true, 50).profile().unwrap();
        let expected = Arc::get_mut(&mut profile).unwrap();
        if extra_signer {
            expected.protocol[1]
                .accounts
                .push(solana_sdk::instruction::AccountMeta::new_readonly(
                    key(30),
                    true,
                ));
        } else {
            expected.protocol[1].data.resize(1400, 1);
        }
        let invocation = Invocation::new(profile.clone());
        let manager = protocol(&invocation);
        let error = manager
            .apply_middlewares_process_full_instructions(full(&profile), "MeteoraDammV2", false)
            .unwrap_err();
        assert!(error.to_string().contains(if extra_signer {
            "unexpected signer"
        } else {
            "wire limit"
        }));
    }
}

#[test]
fn damm_sell_message_guard_report_keeps_all_signatures_and_rejects_unattested_success() {
    let profile = fixture(SwqosType::Temporal, false, 50).profile().unwrap();
    let invocation = Invocation::new(profile);
    let signatures = vec!["known-one".to_string(), "known-two".to_string()];
    let failure = invocation
        .finish(Ok(SdkLaneReport {
            success: true,
            signatures: signatures.clone(),
            error: None,
        }))
        .err()
        .unwrap();
    assert_eq!(failure.signatures, signatures);
    let original = invocation
        .finish(Err(SdkInvocationFailure {
            signatures: signatures.clone(),
            message: "SDK outcome uncertain".into(),
        }))
        .err()
        .unwrap();
    assert_eq!(original.signatures, signatures);
    assert_eq!(original.message, "SDK outcome uncertain");
    let failed = invocation
        .finish(Ok(SdkLaneReport {
            success: false,
            signatures: vec![],
            error: Some("before build".into()),
        }))
        .unwrap_or_else(|failure| panic!("{}", failure.message));
    assert!(!failed.success);
    assert_eq!(failed.error.as_deref(), Some("before build"));
}

#[test]
fn damm_sell_message_guard_verifies_public_rfc8032_vector_without_producing_signatures() {
    let wallet = Pubkey::new_from_array([
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ]);
    let signature = Signature::from([
        0xe5, 0x56, 0x43, 0x00, 0xc3, 0x60, 0xac, 0x72, 0x90, 0x86, 0xe2, 0xcc, 0x80, 0x6e, 0x82,
        0x8a, 0x84, 0x87, 0x7f, 0x1e, 0xb8, 0xe5, 0xd9, 0x74, 0xd8, 0x73, 0xe0, 0x65, 0x22, 0x49,
        0x01, 0x55, 0x5f, 0xb8, 0x82, 0x15, 0x90, 0xa3, 0x3b, 0xac, 0xc6, 0x1e, 0x39, 0x70, 0x1c,
        0xf9, 0xb4, 0x6b, 0xd2, 0x5b, 0xf5, 0xf0, 0x59, 0x5b, 0xbe, 0x24, 0x65, 0x51, 0x41, 0x43,
        0x8e, 0x7a, 0x10, 0x0b,
    ])
    .to_string();
    verify_report(wallet, &[], std::slice::from_ref(&signature)).unwrap();
    assert!(verify_report(wallet, b"changed", std::slice::from_ref(&signature)).is_err());
    assert!(verify_report(key(30), &[], std::slice::from_ref(&signature)).is_err());
    assert!(verify_report(wallet, &[], &[signature.clone(), signature]).is_err());
    assert!(verify_report(wallet, &[], &[]).is_err());
}

#[tokio::test]
async fn damm_sell_message_guard_cancelled_invocation_revokes_late_callbacks() {
    let profile = fixture(SwqosType::Default, false, 50).profile().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let copy = profile.clone();
    let business = profile.protocol.clone();
    let task = tokio::spawn(async move {
        copy.invoke(|manager| async move {
            manager
                .apply_middlewares_process_protocol_instructions(business, "MeteoraDammV2", false)
                .unwrap();
            let _ = tx.send(manager);
            std::future::pending::<Result<SdkLaneReport, SdkInvocationFailure>>().await
        })
        .await
    });
    let manager = rx.await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    let error = manager
        .apply_middlewares_process_full_instructions(full(&profile), "MeteoraDammV2", false)
        .unwrap_err();
    assert!(error.to_string().contains("cancelled or completed"));
}

#[tokio::test]
async fn damm_sell_message_guard_does_not_renew_balance_before_full_callback() {
    let profile = fixture(SwqosType::Default, true, 50).profile().unwrap();
    let invocation = Invocation::new(profile.clone());
    let manager = protocol(&invocation);
    tokio::time::sleep(std::time::Duration::from_millis(3100)).await;
    let error = manager
        .apply_middlewares_process_full_instructions(full(&profile), "MeteoraDammV2", false)
        .unwrap_err();
    assert!(error.to_string().contains("expired"));
}

#[test]
fn damm_sell_message_guard_concurrent_full_callbacks_allow_only_one_candidate() {
    let profile = fixture(SwqosType::Default, true, 50).profile().unwrap();
    let invocation = Invocation::new(profile.clone());
    let first = protocol(&invocation);
    let second = first.clone();
    let start = Arc::new(std::sync::Barrier::new(2));
    let accepted = std::thread::scope(|scope| {
        let mut tasks = vec![];
        for manager in [first, second] {
            let start = start.clone();
            let instructions = full(&profile);
            tasks.push(scope.spawn(move || {
                start.wait();
                manager
                    .apply_middlewares_process_full_instructions(
                        instructions,
                        "MeteoraDammV2",
                        false,
                    )
                    .is_ok()
            }));
        }
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .filter(|accepted| *accepted)
            .count()
    });
    assert_eq!(accepted, 1);
}

#[test]
fn damm_sell_message_guard_preserves_sdk_tip_rounding_zero_transfer_and_saturation() {
    for (tip, lamports) in [
        (0.0, 0),
        (0.4e-9, 0),
        (0.6e-9, 1),
        (0.001, 1_000_000),
        (f64::MAX, u64::MAX),
    ] {
        let mut f = fixture(SwqosType::Temporal, false, 50);
        f.intent.tip_sol = tip;
        f.request.gas_fee_strategy =
            crate::adapters::sdk::ordinary_params::default_fee_strategy(SwqosType::Temporal, tip);
        let profile = f.profile().unwrap();
        let invocation = Invocation::new(profile.clone());
        let manager = protocol(&invocation);
        let instructions = full(&profile);
        if tip > 0.0 {
            assert_eq!(
                instructions[0],
                solana_system_interface::instruction::transfer(
                    &profile.context.wallet,
                    &NOZOMI_TIP_ACCOUNTS[0],
                    lamports,
                )
            );
        } else {
            assert_eq!(instructions.len(), profile.protocol.len() + 2);
        }
        manager
            .apply_middlewares_process_full_instructions(instructions, "MeteoraDammV2", false)
            .unwrap();
    }
}

#[test]
fn damm_sell_message_guard_wire_size_matches_sdk_wincode_serializer_without_signing() {
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        for simulate in [false, true] {
            for percent in [50, 100] {
                let profile = fixture(provider, simulate, percent).profile().unwrap();
                let message = compile_message(
                    &profile.context.wallet,
                    &full(&profile),
                    &[],
                    profile.intent.recent_blockhash,
                    TradeTransactionVersion::V0,
                    v1::TransactionConfig::empty(),
                )
                .unwrap();
                let transaction = VersionedTransaction {
                    signatures: vec![Signature::default()],
                    message,
                };
                let (sdk, signature) =
                    sol_trade_sdk::swqos::serialization::serialize_transaction_bincode_sync(
                        &transaction,
                    )
                    .unwrap();
                let bytes = bincode::serialize(&transaction).unwrap();
                assert_eq!(sdk.as_ref(), bytes);
                assert_eq!(
                    bincode::serialized_size(&transaction).unwrap(),
                    sdk.as_ref().len() as u64
                );
                assert_eq!(signature, Signature::default());
            }
        }
    }
}

#[test]
fn damm_sell_message_guard_preserves_mismatched_signature_evidence_after_frozen_candidate() {
    let profile = fixture(SwqosType::Default, true, 50).profile().unwrap();
    let invocation = Invocation::new(profile.clone());
    protocol(&invocation)
        .apply_middlewares_process_full_instructions(full(&profile), "MeteoraDammV2", false)
        .unwrap();
    let signatures = vec![Signature::from([7; 64]).to_string()];
    let error = invocation
        .finish(Ok(SdkLaneReport {
            success: true,
            signatures: signatures.clone(),
            error: None,
        }))
        .err()
        .unwrap();
    assert_eq!(error.signatures, signatures);
    assert!(error.message.contains("does not bind"));
}
