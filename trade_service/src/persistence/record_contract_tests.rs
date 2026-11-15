use crate::models::{ExecutionMethod, Pool, Side, TradeRecord, TradeStatus};
use sha2::{Digest, Sha256};

#[test]
fn legacy_trade_record_v1_bytes_and_roundtrip_before_model_extraction() {
    let mut rows = Vec::new();
    for pool in [
        Pool::Pump,
        Pool::PumpAmm,
        Pool::Bonk,
        Pool::StonkFun,
        Pool::StonkFunSwap,
        Pool::MeteoraDammV2,
        Pool::MeteoraDbc,
        Pool::RaydiumAmmV4,
        Pool::RaydiumCpmm,
    ] {
        for side in [Side::Buy, Side::Sell] {
            for method in [ExecutionMethod::Sdk, ExecutionMethod::Pumpapi] {
                for status in [
                    TradeStatus::Accepted,
                    TradeStatus::WarmingParams,
                    TradeStatus::BuildingTx,
                    TradeStatus::Submitting,
                    TradeStatus::Simulated,
                    TradeStatus::Submitted,
                    TradeStatus::Confirmed,
                    TradeStatus::Failed,
                    TradeStatus::Rejected,
                    TradeStatus::Unknown,
                ] {
                    for populated in [false, true] {
                        let record = TradeRecord {
                            request_id: "tr_record_fixture".into(),
                            wallet_id: " literal-wallet ".into(),
                            wallet_pubkey: "public-identity".into(),
                            side,
                            pool,
                            mint: "synthetic-mint".into(),
                            pool_id: populated.then(|| "pool-id".into()),
                            execution_method: method,
                            status,
                            signature: populated.then(String::new),
                            error: populated.then(|| "quote \" and newline\n".into()),
                            duplicate: populated,
                            created_at_ms: if populated {
                                u128::from(u64::MAX) + 1
                            } else {
                                1
                            },
                            updated_at_ms: if populated { u128::MAX } else { 2 },
                        };
                        let encoded = serde_json::to_string(&record).unwrap();
                        let decoded: TradeRecord = serde_json::from_str(&encoded).unwrap();
                        assert_eq!(serde_json::to_string(&decoded).unwrap(), encoded);
                        assert_eq!(decoded.created_at_ms, record.created_at_ms);
                        assert_eq!(decoded.updated_at_ms, record.updated_at_ms);
                        rows.push(encoded);
                    }
                }
            }
        }
    }
    assert_eq!(rows.len(), 720);
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&rows).unwrap()));
    assert_eq!(
        hash,
        "10fcd4f5bc6dfb242d926533a0f6651728d6e73877a474bb9c4d935d58c89178"
    );
}
