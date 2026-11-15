//! V1 capability and validation policies captured before Registry activation.
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    models::{Pool, WarmupRequest, SOL_MINT, USD1_MINT, USDC_MINT, WSOL_MINT},
    validation::validate_warmup,
};

#[test]
fn protocol_capability_v1_snapshot() {
    let mut values = vec![];
    for enabled in [false, true] {
        let response = crate::protocol_capabilities::current(enabled);
        values
            .push(json!({"schema_version":response.schema_version,"protocols":response.protocols}));
    }
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&values).unwrap())),
        "6bda4c6b2f50544fe81239dabf21ed9c3920f20b3d360c3ecafb010a44e1ce5e"
    );
}

#[test]
fn protocol_validation_v1_snapshot() {
    let mut values = vec![];
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
        for quote in [
            None,
            Some(SOL_MINT),
            Some(WSOL_MINT),
            Some(USDC_MINT),
            Some(USD1_MINT),
            Some("11111111111111111111111111111111"),
        ] {
            for id in [None, Some("11111111111111111111111111111111")] {
                let request = WarmupRequest {
                    pool,
                    mint: "11111111111111111111111111111111".into(),
                    pool_id: id.map(str::to_string),
                    quote_mint: quote.map(str::to_string),
                };
                let outcome = match validate_warmup(&request) {
                    Ok(n) => {
                        json!({"mint":n.mint.to_string(),"quote_mint":n.quote_mint.to_string(),"pool_id":n.pool_id.map(|v|v.to_string())})
                    }
                    Err(e) => json!({"error":e.to_string()}),
                };
                values.push(json!({"pool":pool,"quote":quote,"pool_id":id,"outcome":outcome}));
            }
        }
    }
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&values).unwrap())),
        "ee62bedc8d35b5837b895e5f8b58e658c21e80cf8c7e298e0d4012de765fed7e"
    );
}
