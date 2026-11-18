use super::*;
use crate::adapters::rpc::pump_native_probe::PumpNativeProbePacket;
use crate::adapters::sdk::{
    damm_instructions::tip_accounts,
    pump_curve_evidence::native_funding::probe::observation::{
        post_bank::tests::response_with_bank, shared::NativeBankObservation, tests::observed,
    },
};
use serde_json::json;
use sol_trade_sdk::swqos::TradeType;
use solana_sdk::{
    hash::Hash,
    message::{Message, VersionedMessage},
    transaction::Transaction,
};
use std::time::{Duration, Instant};

async fn source(side: TradeType, balance: u64) -> NativeSwapConstraints {
    let (probe, mut response) = observed(side).await;
    let original = response["value"]["preBalances"][0].as_u64().unwrap();
    let post = response["value"]["postBalances"][0].as_u64().unwrap();
    response["value"]["preBalances"][0] = json!(balance);
    response["value"]["postBalances"][0] = json!(u64::try_from(
        i128::from(post) + i128::from(balance) - i128::from(original)
    )
    .unwrap());
    let response = response_with_bank(&probe, response);
    let bank = probe.inspect_post_bank(&response).unwrap();
    let observation = probe.inspect_observation(response).unwrap();
    NativeBankObservation { observation, bank }
        .swap_constraints(1_500)
        .unwrap()
}

#[tokio::test]
async fn native_envelope_owns_complete_zero_signature_message_and_preserves_swap_minimum() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal, SwqosType::Jito] {
            for tip in [0.0, 0.0001] {
                let source = source(side, 50_000_000).await;
                let expected_swap = source.packet().unwrap().message.instructions[0]
                    .data
                    .clone();
                let destination = (provider != SwqosType::Default && tip > 0.0)
                    .then(|| tip_accounts(provider)[0]);
                let template = source.prepare_envelope(provider, tip, destination).unwrap();
                let packet = template.packet().unwrap();
                let probe = &template.source.source.observation.probe;
                let expected = Message::new_with_blockhash(
                    &template.instructions,
                    Some(&probe.funding.keys[6]),
                    &probe.transaction.message.recent_blockhash,
                );
                assert_eq!(packet.message, VersionedMessage::Legacy(expected.clone()));
                assert_eq!(packet.signatures, [Signature::default()]);
                assert_eq!(expected.header.num_required_signatures, 1);
                assert_eq!(expected.instructions.last().unwrap().data, expected_swap);
                let original = VersionedMessage::Legacy(probe.transaction.message.clone());
                let reserved = std::collections::HashSet::new();
                for (before, key) in original.static_account_keys().iter().enumerate() {
                    let after = packet
                        .message
                        .static_account_keys()
                        .iter()
                        .position(|actual| actual == key)
                        .unwrap();
                    assert_eq!(packet.message.is_signer(after), original.is_signer(before));
                    assert_eq!(
                        packet
                            .message
                            .is_maybe_writable_with_reserved_addresses(after, Some(&reserved)),
                        original.is_maybe_writable_with_reserved_addresses(before, Some(&reserved)),
                    );
                }
                assert_eq!(
                    template.instructions.len(),
                    if destination.is_some() { 4 } else { 3 }
                );
                let budget = usize::from(destination.is_some());
                let mut price = vec![3];
                price.extend(1_000u64.to_le_bytes());
                let mut limit = vec![2];
                limit.extend(300_000u32.to_le_bytes());
                assert_eq!(
                    template.instructions[budget].program_id.to_string(),
                    "ComputeBudget111111111111111111111111111111"
                );
                assert_eq!(
                    template.instructions[budget + 1].program_id,
                    template.instructions[budget].program_id
                );
                assert_eq!(template.instructions[budget].data, price);
                assert_eq!(template.instructions[budget + 1].data, limit);
                assert!(template.instructions[budget].accounts.is_empty());
                assert!(template.instructions[budget + 1].accounts.is_empty());
                if let Some(destination) = destination {
                    assert_eq!(
                        template.instructions[0],
                        solana_system_interface::instruction::transfer(
                            &probe.funding.keys[6],
                            &destination,
                            100_000,
                        )
                    );
                }
                assert!(bincode::serialized_size(packet).unwrap() <= 1232);
                let discovery_packet = Transaction {
                    signatures: packet.signatures.clone(),
                    message: expected,
                };
                assert!(PumpNativeProbePacket::new(
                    &discovery_packet,
                    &probe.evidence_accounts(),
                    probe.funding.curve.slot,
                    Instant::now() + Duration::from_secs(1),
                )
                .is_err());
            }
        }
    }
}

#[tokio::test]
async fn native_envelope_prefunds_priority_and_tip_without_crediting_sell_proceeds() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let base_debit = if matches!(side, TradeType::Buy) {
            1_005_000
        } else {
            5_000
        };
        let ceiling = base_debit + 300 + 100_000;
        for missing in [false, true] {
            let source = source(side, ceiling - u64::from(missing)).await;
            let result = source.prepare_envelope(
                SwqosType::Temporal,
                0.0001,
                Some(tip_accounts(SwqosType::Temporal)[0]),
            );
            if missing {
                assert!(result.err().unwrap().to_string().contains("cannot prefund"));
            } else {
                let template = result.unwrap();
                assert_eq!(template.native.maximum_debit, ceiling);
                assert_eq!(template.native.remaining_without_output, 0);
                template.packet().unwrap();
            }
        }
    }
}

#[tokio::test]
async fn native_envelope_rejects_nonfinite_overflow_and_wrong_provider_tip_routes() {
    for tip in [
        f64::NAN,
        f64::INFINITY,
        -0.001,
        f64::MAX,
        u64::MAX as f64 / 1_000_000_000.0,
    ] {
        assert!(source(TradeType::Buy, 50_000_000)
            .await
            .prepare_envelope(
                SwqosType::Temporal,
                tip,
                Some(tip_accounts(SwqosType::Temporal)[0])
            )
            .is_err());
    }
    for (provider, tip, destination) in [
        (SwqosType::Temporal, 0.0001, None),
        (
            SwqosType::Temporal,
            0.0001,
            Some(Pubkey::new_from_array([87; 32])),
        ),
        (
            SwqosType::Temporal,
            0.0001,
            Some(tip_accounts(SwqosType::Jito)[0]),
        ),
        (
            SwqosType::Default,
            0.0001,
            Some(tip_accounts(SwqosType::Temporal)[0]),
        ),
        (
            SwqosType::Temporal,
            0.0,
            Some(tip_accounts(SwqosType::Temporal)[0]),
        ),
    ] {
        assert!(source(TradeType::Buy, 50_000_000)
            .await
            .prepare_envelope(provider, tip, destination)
            .is_err());
    }
    let constraints = source(TradeType::Buy, 50_000_000).await;
    let wallet = constraints.source.observation.probe.funding.keys[6];
    assert!(constraints
        .prepare_envelope(SwqosType::Temporal, 0.0001, Some(wallet))
        .err()
        .unwrap()
        .to_string()
        .contains("aliases"));
}

#[tokio::test]
async fn native_envelope_rejects_changed_packet_instruction_fee_and_expired_source() {
    for damage in 0..10 {
        let mut template = source(TradeType::Sell, 50_000_000)
            .await
            .prepare_envelope(
                SwqosType::Temporal,
                0.0001,
                Some(tip_accounts(SwqosType::Temporal)[0]),
            )
            .unwrap();
        let VersionedMessage::Legacy(message) = &mut template.transaction.message else {
            panic!("expected legacy")
        };
        match damage {
            0 => template.transaction.signatures[0] = Signature::from([1; 64]),
            1 => message.instructions[0].data[4] ^= 1,
            2 => message.instructions[1].data[1] ^= 1,
            3 => message.instructions.push(message.instructions[0].clone()),
            4 => message.recent_blockhash = Hash::new_from_array([77; 32]),
            5 => template.instructions.swap(0, 1),
            6 => template.native.maximum_debit += 1,
            7 => template.tip_sol = 0.0002,
            8 => template.destination = Some(tip_accounts(SwqosType::Temporal)[1]),
            _ => {
                template.source.source.observation.probe.funding.started = Instant::now()
                    - crate::domain::preparation_balance::MAX_BALANCE_AGE
                    - Duration::from_secs(1)
            }
        }
        assert!(template.packet().is_err(), "damage {damage}");
    }
}
