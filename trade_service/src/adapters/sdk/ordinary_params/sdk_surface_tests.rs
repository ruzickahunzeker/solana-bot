//! Locked public SDK surface only, not protocol/quote/signing acceptance.
use super::dex_type;
use crate::protocol::registry::{AdapterStage, DEFINITIONS};
use sol_trade_sdk::trading::factory::{DexType, TradeFactory};
use std::sync::Arc;

fn expected_name(dex: DexType) -> &'static str {
    // No wildcard: an added SDK variant needs an explicit compatibility review.
    match dex {
        DexType::PumpFun => "PumpFun",
        DexType::PumpSwap => "PumpSwap",
        DexType::LaunchLab => "LaunchLab",
        DexType::Bonk => "Bonk",
        DexType::StonkFun => "StonkFun",
        DexType::RaydiumCpmm => "RaydiumCpmm",
        DexType::RaydiumAmmV4 => "RaydiumAmmV4",
        DexType::MeteoraDammV2 => "MeteoraDammV2",
        DexType::MeteoraDbc => "MeteoraDbc",
        DexType::RaydiumClmm => "RaydiumClmm",
        DexType::OrcaWhirlpool => "OrcaWhirlpool",
        DexType::MeteoraDlmm => "MeteoraDlmm",
    }
}

#[test]
fn sdk_surface_contract_pins_public_factory_variants_names_and_shared_builder_ownership() {
    let types = [
        DexType::PumpFun,
        DexType::PumpSwap,
        DexType::LaunchLab,
        DexType::Bonk,
        DexType::StonkFun,
        DexType::RaydiumCpmm,
        DexType::RaydiumAmmV4,
        DexType::MeteoraDammV2,
        DexType::MeteoraDbc,
        DexType::RaydiumClmm,
        DexType::OrcaWhirlpool,
        DexType::MeteoraDlmm,
    ];
    for dex in types {
        // GenericTradeExecutor::new stores only this builder and name; never call swap.
        let first = TradeFactory::create_executor(dex);
        let second = TradeFactory::create_executor(dex);
        assert_eq!(first.protocol_name(), expected_name(dex));
        assert!(Arc::ptr_eq(&first, &second));
    }
}

#[test]
fn sdk_surface_contract_binds_service_projection_without_opening_observed_protocols() {
    let mut existing = 0;
    let mut pending = 0;
    let mut absent = 0;
    for definition in &DEFINITIONS {
        let expected = match definition.protocol_id {
            "pump" => Some(DexType::PumpFun),
            "pump-amm" => Some(DexType::PumpSwap),
            "bonk" => Some(DexType::Bonk),
            "stonkfun" | "stonkfun-swap" => Some(DexType::StonkFun),
            "meteora-damm-v2" => Some(DexType::MeteoraDammV2),
            "meteora-dbc" => Some(DexType::MeteoraDbc),
            "raydium-amm-v4" => Some(DexType::RaydiumAmmV4),
            "raydium-cpmm" => Some(DexType::RaydiumCpmm),
            "raydium-launchlab" => Some(DexType::LaunchLab),
            "raydium-clmm" => Some(DexType::RaydiumClmm),
            "meteora-dlmm" => Some(DexType::MeteoraDlmm),
            "orca-whirlpool" => Some(DexType::OrcaWhirlpool),
            "meteora-launchpad" | "meteora-damm-v1" => None,
            id => panic!("new registry identity needs reviewed SDK evidence: {id}"),
        };
        assert_eq!(definition.sdk_builder_available, expected.is_some());
        if let Some(pool) = definition.pool {
            existing += 1;
            assert_eq!(definition.stage, AdapterStage::Existing);
            assert_eq!(Some(dex_type(pool)), expected);
        } else {
            assert_eq!(definition.stage, AdapterStage::ObserveOnly);
            if expected.is_some() {
                pending += 1;
            } else {
                absent += 1;
            }
            assert!(
                serde_json::from_value::<crate::domain::trade::Pool>(serde_json::json!(
                    definition.protocol_id
                ))
                .is_err()
            );
            for enabled in [false, true] {
                let response = crate::protocol_capabilities::current(enabled);
                let capability = response
                    .protocols
                    .iter()
                    .find(|item| item.protocol_id == definition.protocol_id)
                    .unwrap();
                assert_eq!(capability.status, "observe");
                assert_eq!(capability.execution_backend, "sdk-adapter-pending");
                assert_eq!(capability.sdk_builder_available, expected.is_some());
                assert!(
                    !capability.supports_buy
                        && !capability.supports_sell
                        && !capability.supports_direct_quote
                        && !capability.supports_sol_route
                );
            }
        }
    }
    assert_eq!((existing, pending, absent), (9, 4, 2));
}
