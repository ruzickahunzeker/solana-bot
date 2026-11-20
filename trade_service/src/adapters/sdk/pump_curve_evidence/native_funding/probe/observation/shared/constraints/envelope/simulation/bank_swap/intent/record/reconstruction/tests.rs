use super::*;
use crate::{config::TradePersistenceMode, domain::trade::MevProvider};
use sol_trade_sdk::swqos::TradeType;
use solana_sdk::{message::VersionedMessage, signature::Signature};

async fn fixture(side: TradeType, provider: SwqosType) -> (PumpNativeCandidateRecord, Vec<u8>) {
    let (f, observation) =
        super::super::super::tests::admitted(TradePersistenceMode::Durable, side, provider).await;
    let bound = observation.bind_intent(&f.execution).unwrap();
    (
        bound.record().unwrap(),
        bound.packet().unwrap().message.serialize(),
    )
}

#[tokio::test]
async fn pump_native_reconstruction_matches_original_full_unsigned_message_without_live_state() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (record, message) = fixture(side, provider).await;
            let encoded = record.encode(&message).unwrap();
            let historical = PumpNativeCandidateRecord::decode(&encoded, &message).unwrap();
            let packet = reconstruct(&historical).unwrap();
            assert_eq!(packet.message.serialize(), message);
            assert_eq!(packet.signatures, [Signature::default()]);
            assert_eq!(packet.message.header().num_required_signatures, 1);
            assert_eq!(
                packet.message.static_account_keys()[0].to_bytes(),
                record.details().wallet
            );
            validate(&historical, &message).unwrap();
            crate::adapters::sdk::pump_native_message::assert_native_history_only(
                &encoded, &message,
            );
        }
    }
}

#[tokio::test]
async fn pump_native_reconstruction_rejects_every_rehashed_account_instruction_and_header_change() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let (record, message) = fixture(side, SwqosType::Temporal).await;
        let VersionedMessage::Legacy(original) =
            bincode::deserialize::<VersionedMessage>(&message).unwrap()
        else {
            unreachable!()
        };
        for index in 0..original.account_keys.len() {
            let mut changed = original.clone();
            changed.account_keys[index] = Pubkey::new_from_array([91; 32]);
            let changed = VersionedMessage::Legacy(changed).serialize();
            let rehashed =
                PumpNativeCandidateRecord::new(record.details().clone(), &changed).unwrap();
            rehashed.validate(&changed).unwrap();
            assert!(validate(&rehashed, &changed).is_err(), "account {index}");
        }
        for index in 0..original.instructions.len() {
            for damage in 0..2 {
                let mut changed = original.clone();
                if damage == 0 {
                    changed.instructions[index].data[0] ^= 1;
                } else {
                    changed.instructions[index].program_id_index ^= 1;
                }
                let changed = VersionedMessage::Legacy(changed).serialize();
                let rehashed =
                    PumpNativeCandidateRecord::new(record.details().clone(), &changed).unwrap();
                assert!(
                    validate(&rehashed, &changed).is_err(),
                    "instruction {index}/{damage}"
                );
            }
        }
        for damage in 0..6 {
            let mut changed = original.clone();
            match damage {
                0 => changed.header.num_required_signatures += 1,
                1 => changed.header.num_readonly_unsigned_accounts += 1,
                2 => changed.recent_blockhash = Hash::new_from_array([91; 32]),
                3 => changed.instructions.swap(0, 1),
                4 => {
                    let instruction = changed.instructions.last().unwrap().clone();
                    changed.instructions.push(instruction);
                }
                _ => {
                    changed.instructions.pop();
                }
            }
            let changed = VersionedMessage::Legacy(changed).serialize();
            let rehashed =
                PumpNativeCandidateRecord::new(record.details().clone(), &changed).unwrap();
            assert!(
                validate(&rehashed, &changed).is_err(),
                "header/shape {damage}"
            );
        }
    }
}

#[tokio::test]
async fn pump_native_reconstruction_binds_canonical_pdas_program_locked_fee_and_requested_provider()
{
    let (record, message) = fixture(TradeType::Buy, SwqosType::Temporal).await;
    for field in [
        "mint",
        "curve",
        "vault",
        "ata",
        "program",
        "creator",
        "creator_vault",
        "protocol_fee",
        "buyback",
        "cashback",
        "priority",
        "relay",
        "provider",
        "tip",
        "alias",
        "tip_alias",
    ] {
        let mut details = record.details().clone();
        match field {
            "mint" => details.mint = [91; 32],
            "curve" => details.curve = [91; 32],
            "vault" => details.curve_vault = [91; 32],
            "ata" => details.user_ata = [91; 32],
            "program" => details.base_program = [91; 32],
            "creator" => details.creator = [91; 32],
            "creator_vault" => details.creator_vault = [91; 32],
            "protocol_fee" => details.protocol_fee = [91; 32],
            "buyback" => details.buyback = [91; 32],
            "cashback" => details.cashback = !details.cashback,
            "priority" => details.priority_fee_lamports += 1,
            "relay" => details.relay = "UnknownProvider".into(),
            "provider" => details.requested_mev_provider = Some(MevProvider::Jito),
            "tip" => details.tip_destination = Some([91; 32]),
            "alias" => details.protocol_fee = details.wallet,
            "tip_alias" => details.protocol_fee = details.tip_destination.unwrap(),
            _ => unreachable!(),
        }
        let changed = PumpNativeCandidateRecord::new(details, &message).unwrap();
        changed.validate(&message).unwrap();
        assert!(validate(&changed, &message).is_err(), "{field}");
    }
}

#[tokio::test]
async fn pump_native_reconstruction_consistency_is_not_original_account_truth_or_signing_authority()
{
    let (f, observation) = super::super::super::tests::admitted(
        TradePersistenceMode::Durable,
        TradeType::Buy,
        SwqosType::Default,
    )
    .await;
    let bound = observation.bind_intent(&f.execution).unwrap();
    let message = bound.packet().unwrap().message.serialize();
    let original = bound.record().unwrap();
    let mut details = original.details().clone();
    details.protocol_fee = [91; 32];
    let nominal = PumpNativeCandidateRecord::new(details.clone(), &message).unwrap();
    let changed_message = reconstruct(&nominal).unwrap().message.serialize();
    assert_ne!(changed_message, message);
    let coherent = PumpNativeCandidateRecord::new(details, &changed_message).unwrap();
    validate(&coherent, &changed_message).unwrap();
    // Pure historical consistency cannot prove membership in the actual bank's fee set.
    assert!(bound.check_record(&coherent, &changed_message).is_err());
    assert!(bound.check_record(&coherent, &message).is_err());
    crate::adapters::sdk::pump_native_message::assert_native_history_only(
        &coherent.encode(&changed_message).unwrap(),
        &changed_message,
    );
    bound.check_record(&original, &message).unwrap();
}
