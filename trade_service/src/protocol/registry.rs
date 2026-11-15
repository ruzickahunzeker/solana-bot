//! Existing service policies and legacy capability claims, not chain acceptance evidence.
use crate::domain::trade::Pool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdapterStage {
    Existing,
    ObserveOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolIdPolicy {
    Forbidden,
    Optional,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuotePolicy {
    WsolOrUsdc,
    WsolOrUsd1,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacySolRouteClaim {
    Always,
    StonkfunFeature,
    Never,
}

impl LegacySolRouteClaim {
    pub fn enabled(self, stonkfun_enabled: bool) -> bool {
        match self {
            Self::Always => true,
            Self::StonkfunFeature => stonkfun_enabled,
            Self::Never => false,
        }
    }
}

pub(crate) struct ProtocolDefinition {
    pub pool: Option<Pool>,
    pub protocol_id: &'static str,
    // Discovery/display aliases are not extra accepted TradeRequest aliases.
    pub aliases: &'static [&'static str],
    pub stage: AdapterStage,
    pub pool_id_policy: PoolIdPolicy,
    pub quote_policy: Option<QuotePolicy>,
    // V1 claims are frozen separately from actual validation/acceptance rules.
    pub legacy_pool_id_required: bool,
    pub legacy_sol_route: LegacySolRouteClaim,
    pub sdk_builder_available: bool,
    pub legacy_token_programs: &'static [&'static str],
}

const fn existing(
    pool: Pool,
    protocol_id: &'static str,
    aliases: &'static [&'static str],
    pool_id_policy: PoolIdPolicy,
    quote_policy: QuotePolicy,
    legacy_sol_route: LegacySolRouteClaim,
) -> ProtocolDefinition {
    ProtocolDefinition {
        pool: Some(pool),
        protocol_id,
        aliases,
        stage: AdapterStage::Existing,
        pool_id_policy,
        quote_policy: Some(quote_policy),
        legacy_pool_id_required: !matches!(pool_id_policy, PoolIdPolicy::Forbidden),
        legacy_sol_route,
        sdk_builder_available: true,
        legacy_token_programs: &["spl-token"],
    }
}

const fn observed(
    protocol_id: &'static str,
    aliases: &'static [&'static str],
    sdk_builder_available: bool,
) -> ProtocolDefinition {
    ProtocolDefinition {
        pool: None,
        protocol_id,
        aliases,
        stage: AdapterStage::ObserveOnly,
        pool_id_policy: PoolIdPolicy::Required,
        quote_policy: None,
        legacy_pool_id_required: true,
        legacy_sol_route: LegacySolRouteClaim::Never,
        sdk_builder_available,
        legacy_token_programs: &["spl-token"],
    }
}

pub(crate) static DEFINITIONS: [ProtocolDefinition; 15] = [
    existing(
        Pool::Pump,
        "pump",
        &["pumpfun", "pump-fun"],
        PoolIdPolicy::Forbidden,
        QuotePolicy::WsolOrUsdc,
        LegacySolRouteClaim::Always,
    ),
    existing(
        Pool::PumpAmm,
        "pump-amm",
        &["pumpswap", "pump-swap"],
        PoolIdPolicy::Optional,
        QuotePolicy::Any,
        LegacySolRouteClaim::Never,
    ),
    existing(
        Pool::Bonk,
        "bonk",
        &[],
        PoolIdPolicy::Forbidden,
        QuotePolicy::WsolOrUsd1,
        LegacySolRouteClaim::Never,
    ),
    existing(
        Pool::StonkFun,
        "stonkfun",
        &["stonk-fun"],
        PoolIdPolicy::Required,
        QuotePolicy::Any,
        LegacySolRouteClaim::StonkfunFeature,
    ),
    existing(
        Pool::StonkFunSwap,
        "stonkfun-swap",
        &["stonk-fun-swap"],
        PoolIdPolicy::Required,
        QuotePolicy::Any,
        LegacySolRouteClaim::StonkfunFeature,
    ),
    existing(
        Pool::MeteoraDammV2,
        "meteora-damm-v2",
        &["meteora", "meteora-dammv2"],
        PoolIdPolicy::Required,
        QuotePolicy::WsolOrUsdc,
        LegacySolRouteClaim::Never,
    ),
    existing(
        Pool::MeteoraDbc,
        "meteora-dbc",
        &["dbc", "dynamic-bonding-curve"],
        PoolIdPolicy::Required,
        QuotePolicy::WsolOrUsdc,
        LegacySolRouteClaim::Never,
    ),
    existing(
        Pool::RaydiumAmmV4,
        "raydium-amm-v4",
        &["raydium-v4", "raydium-ammv4"],
        PoolIdPolicy::Required,
        QuotePolicy::WsolOrUsdc,
        LegacySolRouteClaim::Never,
    ),
    existing(
        Pool::RaydiumCpmm,
        "raydium-cpmm",
        &["raydium-cp"],
        PoolIdPolicy::Required,
        QuotePolicy::Any,
        LegacySolRouteClaim::Never,
    ),
    observed(
        "raydium-launchlab",
        &["raydium-launchpad", "raydium-launchlab"],
        true,
    ),
    observed("raydium-clmm", &[], true),
    observed("meteora-launchpad", &[], false),
    observed("meteora-damm-v1", &[], false),
    observed("meteora-dlmm", &[], true),
    observed("orca-whirlpool", &["whirlpool"], true),
];

pub(crate) fn for_pool(pool: Pool) -> &'static ProtocolDefinition {
    // Exhaustive at compile time; tests also bind each entry to its canonical Pool.
    &DEFINITIONS[match pool {
        Pool::Pump => 0,
        Pool::PumpAmm => 1,
        Pool::Bonk => 2,
        Pool::StonkFun => 3,
        Pool::StonkFunSwap => 4,
        Pool::MeteoraDammV2 => 5,
        Pool::MeteoraDbc => 6,
        Pool::RaydiumAmmV4 => 7,
        Pool::RaydiumCpmm => 8,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn registry_identities_are_unique_and_match_wire_pools() {
        let mut ids = HashSet::new();
        let mut pools = HashSet::new();
        for definition in &DEFINITIONS {
            assert!(ids.insert(definition.protocol_id));
            if let Some(pool) = definition.pool {
                let name = serde_json::to_value(pool).unwrap();
                assert_eq!(name, definition.protocol_id);
                assert!(pools.insert(name.to_string()));
                assert!(std::ptr::eq(for_pool(pool), definition));
                assert_eq!(definition.stage, AdapterStage::Existing);
                assert!(definition.quote_policy.is_some());
            } else {
                assert_eq!(definition.stage, AdapterStage::ObserveOnly);
                assert!(definition.quote_policy.is_none());
                assert!(
                    serde_json::from_value::<Pool>(serde_json::json!(definition.protocol_id))
                        .is_err()
                );
            }
        }
        assert_eq!(pools.len(), 9);
    }

    #[test]
    fn display_aliases_cannot_collide_with_other_protocol_identities() {
        let mut owners = HashMap::new();
        for definition in &DEFINITIONS {
            assert!(owners
                .insert(definition.protocol_id, definition.protocol_id)
                .is_none());
        }
        for definition in &DEFINITIONS {
            for alias in definition.aliases {
                if let Some(owner) = owners.insert(*alias, definition.protocol_id) {
                    assert_eq!(owner, definition.protocol_id, "{alias}");
                }
            }
        }
    }

    #[test]
    fn display_aliases_do_not_expand_trade_request_deserialization() {
        for alias in [
            "pumpfun",
            "pump-fun",
            "pumpswap",
            "pump-swap",
            "meteora",
            "dbc",
            "dynamic-bonding-curve",
            "raydium-v4",
            "raydium-cp",
            "raydium-launchpad",
            "whirlpool",
        ] {
            assert!(
                serde_json::from_value::<Pool>(serde_json::json!(alias)).is_err(),
                "{alias}"
            );
        }
        for (alias, pool) in [
            ("stonk-fun", Pool::StonkFun),
            ("stonk-fun-swap", Pool::StonkFunSwap),
        ] {
            assert_eq!(
                serde_json::from_value::<Pool>(serde_json::json!(alias)).unwrap(),
                pool
            );
        }
    }

    #[test]
    fn legacy_claims_are_not_mistaken_for_validation_or_chain_acceptance() {
        let pump_amm = for_pool(Pool::PumpAmm);
        assert!(pump_amm.legacy_pool_id_required);
        assert_eq!(pump_amm.pool_id_policy, PoolIdPolicy::Optional);
        assert_eq!(
            for_pool(Pool::MeteoraDbc).legacy_token_programs,
            &["spl-token"]
        );
        for definition in &DEFINITIONS {
            if definition.stage == AdapterStage::ObserveOnly {
                assert_eq!(definition.legacy_sol_route, LegacySolRouteClaim::Never);
            }
        }
    }
}
