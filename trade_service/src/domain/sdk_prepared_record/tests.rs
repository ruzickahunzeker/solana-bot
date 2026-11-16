use super::*;
use crate::adapters::sdk::{
    damm_probe::test_support::buy_record_fixture, damm_sell_message::candidate_fixture,
};
use sol_trade_sdk::swqos::SwqosType;

#[test]
fn sdk_prepared_record_preserves_legacy_sell_json_bytes_and_message_matrix() {
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        for simulate in [false, true] {
            for percent in [1, 50, 100] {
                let candidate = candidate_fixture(provider, simulate, percent);
                let message = candidate.message_bytes();
                let original = candidate.metadata().sell().unwrap();
                let legacy_json = original.encode(&message).unwrap();
                let record = SdkPreparedRecord::decode(&legacy_json, &message).unwrap();
                assert_eq!(record.sell().unwrap(), original);
                assert_eq!(
                    record.encode(&message).unwrap().as_bytes(),
                    legacy_json.as_bytes()
                );
                assert_eq!(&record, candidate.metadata());
            }
        }
    }
}

#[tokio::test]
async fn sdk_prepared_record_buy_round_trip_is_nominal_not_sell_authority() {
    let (record, message) = buy_record_fixture().await;
    let original = record.encode(&message).unwrap();
    let decoded = SdkPreparedRecord::decode(&original, &message).unwrap();
    assert!(decoded.sell().is_err());
    assert_eq!(decoded.encode(&message).unwrap(), original);
    assert_eq!(decoded, SdkPreparedRecord::DammBuy(record));
    assert!(
        crate::adapters::sdk::damm_sell_message::validate_candidate(&decoded, &message).is_err()
    );
}

#[tokio::test]
async fn sdk_prepared_record_dispatch_rejects_unknown_missing_or_cross_direction_tags() {
    let (buy, buy_message) = buy_record_fixture().await;
    let sell = candidate_fixture(SwqosType::Default, false, 100);
    let sell_message = sell.message_bytes();
    for (json, message, other) in [
        (
            buy.encode(&buy_message).unwrap(),
            &buy_message,
            "damm-v2-sell-v1",
        ),
        (
            sell.metadata().encode(&sell_message).unwrap(),
            &sell_message,
            "damm-v2-buy-v1",
        ),
    ] {
        for change in [
            "version",
            "profile",
            "missing_version",
            "missing_profile",
            "other",
        ] {
            let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
            match change {
                "version" => value["version"] = serde_json::json!(2),
                "profile" => value["profile"] = serde_json::json!("unrecognized"),
                "other" => value["profile"] = serde_json::json!(other),
                key => {
                    value
                        .as_object_mut()
                        .unwrap()
                        .remove(key.strip_prefix("missing_").unwrap());
                }
            }
            assert!(
                SdkPreparedRecord::decode(&value.to_string(), message).is_err(),
                "{change}"
            );
        }
    }
}

#[tokio::test]
async fn sdk_prepared_record_strict_payloads_reject_duplicate_and_unknown_fields() {
    let (buy, buy_message) = buy_record_fixture().await;
    let sell = candidate_fixture(SwqosType::Default, false, 100);
    let sell_message = sell.message_bytes();
    for (json, message) in [
        (buy.encode(&buy_message).unwrap(), &buy_message),
        (
            sell.metadata().encode(&sell_message).unwrap(),
            &sell_message,
        ),
    ] {
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let fields = [
            "\"version\":1,".to_owned(),
            format!("\"profile\":{},", value["profile"]),
            format!("\"details\":{},", value["details"]),
            format!("\"message_sha256\":{},", value["message_sha256"]),
            "\"future_signing_authority\":true,".to_owned(),
        ];
        for field in fields {
            let changed = format!("{{{field}{}", &json[1..]);
            assert!(SdkPreparedRecord::decode(&changed, message).is_err());
        }
        let mut changed = value;
        changed["details"]["pool"]["unknown"] = serde_json::json!(1);
        assert!(SdkPreparedRecord::decode(&changed.to_string(), message).is_err());
    }
}

#[tokio::test]
async fn sdk_prepared_record_limits_and_digest_apply_before_any_journal_permission() {
    let (buy, buy_message) = buy_record_fixture().await;
    let sell = candidate_fixture(SwqosType::Default, false, 100);
    let sell_message = sell.message_bytes();
    for (json, message) in [
        (buy.encode(&buy_message).unwrap(), &buy_message),
        (
            sell.metadata().encode(&sell_message).unwrap(),
            &sell_message,
        ),
    ] {
        assert!(SdkPreparedRecord::decode(&json, &[]).is_err());
        assert!(SdkPreparedRecord::decode(&json, &[1; 1233]).is_err());
        let mut changed = message.clone();
        changed[0] ^= 1;
        assert!(SdkPreparedRecord::decode(&json, &changed).is_err());
        assert!(SdkPreparedRecord::decode(&format!("{json}{{}}"), message).is_err());
        assert!(
            SdkPreparedRecord::decode(&format!("{}{json}", " ".repeat(65_537)), message).is_err()
        );
    }
}
