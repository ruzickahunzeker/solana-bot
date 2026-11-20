use super::*;
use crate::{
    config::TradePersistenceMode,
    domain::{
        preparation_balance::MAX_BALANCE_AGE, sdk_prepared_record::SdkPreparedRecord,
        trade::MevProvider,
    },
};
use serde_json::{json, Value};
use sol_trade_sdk::swqos::TradeType;
use solana_sdk::signature::Signature;
use std::time::{Duration, Instant};

#[tokio::test]
async fn pump_native_record_round_trips_original_identity_integer_budget_and_frozen_message_only() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for provider in [SwqosType::Default, SwqosType::Temporal] {
            let (f, observation) =
                super::super::tests::admitted(TradePersistenceMode::Durable, side, provider).await;
            let bound = observation.bind_intent(&f.execution).unwrap();
            let message = bound.packet().unwrap().message.serialize();
            let record = bound.record().unwrap();
            bound.check_record(&record, &message).unwrap();
            let encoded = record.encode(&message).unwrap();
            let decoded = PumpNativeCandidateRecord::decode(&encoded, &message).unwrap();
            assert_eq!(record, decoded);
            assert_eq!(decoded.encode(&message).unwrap(), encoded);
            let details = decoded.details();
            assert_eq!(details.request_id, f.execution.reservation().request_id());
            assert_eq!(
                details.request_hash,
                f.execution.reservation().request_hash()
            );
            assert_eq!(
                details.idempotency_key,
                f.execution.reservation().idempotency_key()
            );
            assert!(details.idempotency_key.starts_with(' '));
            assert_eq!(
                (details.source_slot, details.quote_slot, details.final_slot),
                (175, 176, 177)
            );
            assert_eq!(details.network_fee_lamports, 5_300);
            assert_eq!(details.priority_fee_lamports, 300);
            assert_eq!(details.final_epoch, 21);
            assert_eq!(details.relay, provider.as_str());
            assert_eq!(
                details.tip_lamports,
                if provider == SwqosType::Default {
                    0
                } else {
                    100_000
                }
            );
            assert_eq!(bound.packet().unwrap().signatures, [Signature::default()]);
            // The original journal decoder still rejects this test-gated nominal profile.
            assert!(SdkPreparedRecord::decode(&encoded, &message).is_err());
            assert!(!encoded.contains("started") && !encoded.contains("signature"));
        }
    }
}

#[tokio::test]
async fn pump_native_record_rejects_invalid_schema_amounts_context_fees_and_changed_message() {
    let (f, observation) = super::super::tests::admitted(
        TradePersistenceMode::Durable,
        TradeType::Sell,
        SwqosType::Temporal,
    )
    .await;
    let bound = observation.bind_intent(&f.execution).unwrap();
    let message = bound.packet().unwrap().message.serialize();
    let encoded = bound.record().unwrap().encode(&message).unwrap();
    for damage in 0..21 {
        let mut value: Value = serde_json::from_str(&encoded).unwrap();
        match damage {
            0 => value["version"] = json!(2),
            1 => value["profile"] = json!("damm-v2-sell-v1"),
            2 => value["unknown"] = json!(true),
            3 => value["details"]["unknown"] = json!(true),
            4 => value["details"]["input_units"] = json!(0),
            5 => value["details"]["sell_percent"] = json!(100),
            6 => value["details"]["source_token_units"] = json!(u64::MAX),
            7 => value["details"]["source_token_units"] = Value::Null,
            8 => value["details"]["minimum_output_units"] = json!(1),
            9 => value["details"]["final_output_units"] = json!(1),
            10 => value["details"]["slippage_bps"] = json!(10_000),
            11 => value["details"]["source_slot"] = json!(0),
            12 => value["details"]["final_slot"] = json!(175),
            13 => value["details"]["tip_sol_bits"] = json!(f64::NAN.to_bits()),
            14 => value["details"]["tip_lamports"] = json!(100_001),
            15 => value["details"]["tip_destination"] = Value::Null,
            16 => value["details"]["remaining_native_lamports"] = json!(0),
            17 => value["details"]["maximum_native_debit"] = json!(u64::MAX),
            18 => value["details"]["network_fee_lamports"] = json!(300),
            19 => value["details"]["recent_blockhash"] = json!(vec![0; 32]),
            _ => value["details"]["wallet"] = json!(vec![0; 32]),
        }
        assert!(
            PumpNativeCandidateRecord::decode(&value.to_string(), &message).is_err(),
            "{damage}"
        );
    }
    let duplicate = encoded.replacen("\"version\":1", "\"version\":1,\"version\":1", 1);
    assert_ne!(duplicate, encoded);
    assert!(PumpNativeCandidateRecord::decode(&duplicate, &message).is_err());
    assert!(PumpNativeCandidateRecord::decode(&format!("{encoded}{{}}"), &message).is_err());
    assert!(PumpNativeCandidateRecord::decode(&" ".repeat(65_537), &message).is_err());
    for changed in [Vec::new(), vec![0; 1233], {
        let mut bytes = message.clone();
        bytes[0] ^= 1;
        bytes
    }] {
        assert!(PumpNativeCandidateRecord::decode(&encoded, &changed).is_err());
        assert!(bound
            .check_record(&bound.record().unwrap(), &changed)
            .is_err());
    }
}

#[tokio::test]
async fn pump_native_record_nominally_valid_changes_do_not_replace_original_admitted_evidence() {
    let (f, observation) = super::super::tests::admitted(
        TradePersistenceMode::Durable,
        TradeType::Buy,
        SwqosType::Default,
    )
    .await;
    let bound = observation.bind_intent(&f.execution).unwrap();
    let message = bound.packet().unwrap().message.serialize();
    let record = bound.record().unwrap();
    for field in [
        "request",
        "wallet_id",
        "key",
        "hash",
        "wallet",
        "mint",
        "curve",
        "curve_vault",
        "ata",
        "program",
        "decimals",
        "creator",
        "fee",
        "creator_vault",
        "buyback",
        "cashback",
        "epoch",
        "use_mev",
        "requested_provider",
        "budget",
    ] {
        let mut details = record.details().clone();
        match field {
            "request" => details.request_id.push('x'),
            "wallet_id" => details.wallet_id.push('x'),
            "key" => details.idempotency_key.push('x'),
            "hash" => details.request_hash.push('x'),
            "wallet" => details.wallet = [91; 32],
            "mint" => details.mint = [91; 32],
            "curve" => details.curve = [91; 32],
            "curve_vault" => details.curve_vault = [91; 32],
            "ata" => details.user_ata = [91; 32],
            "program" => details.base_program = [91; 32],
            "decimals" => details.base_decimals += 1,
            "creator" => details.creator = [91; 32],
            "fee" => details.protocol_fee = [91; 32],
            "creator_vault" => details.creator_vault = [91; 32],
            "buyback" => details.buyback = [91; 32],
            "cashback" => details.cashback = !details.cashback,
            "epoch" => details.final_epoch += 1,
            "use_mev" => details.use_mev = None,
            "requested_provider" => details.requested_mev_provider = Some(MevProvider::Auto),
            "budget" => {
                details.native_before_lamports += 1;
                details.remaining_native_lamports += 1;
            }
            _ => unreachable!(),
        }
        let changed = PumpNativeCandidateRecord::new(details, &message).unwrap();
        changed.validate(&message).unwrap();
        assert_ne!(changed, record);
        assert!(bound.check_record(&changed, &message).is_err(), "{field}");
    }
}

#[tokio::test]
async fn pump_native_record_decode_never_renews_original_age_or_yields_a_signing_record() {
    let (f, observation) = super::super::tests::admitted(
        TradePersistenceMode::Durable,
        TradeType::Buy,
        SwqosType::Temporal,
    )
    .await;
    let mut bound = observation.bind_intent(&f.execution).unwrap();
    let message = bound.packet().unwrap().message.serialize();
    let encoded = bound.record().unwrap().encode(&message).unwrap();
    bound
        .observation
        .template
        .source
        .source
        .observation
        .probe
        .funding
        .started = Instant::now() - MAX_BALANCE_AGE - Duration::from_secs(1);
    let historical = PumpNativeCandidateRecord::decode(&encoded, &message).unwrap();
    assert!(bound.check_record(&historical, &message).is_err());
    assert!(bound.record().is_err());
    assert!(bound.packet().is_err());
    assert!(SdkPreparedRecord::decode(&encoded, &message).is_err());
}
