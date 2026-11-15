use std::time::{SystemTime, UNIX_EPOCH};

pub use crate::api::dto::protocols::{ProtocolCapabilitiesResponse, ProtocolCapability};
use crate::protocol::registry::{AdapterStage, ProtocolDefinition, DEFINITIONS};

pub fn current(stonkfun_sol_enabled: bool) -> ProtocolCapabilitiesResponse {
    ProtocolCapabilitiesResponse {
        schema_version: 1,
        generated_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis()),
        protocols: DEFINITIONS
            .iter()
            .map(|definition| legacy_capability(definition, stonkfun_sol_enabled))
            .collect(),
    }
}

fn legacy_capability(
    definition: &ProtocolDefinition,
    stonkfun_sol_enabled: bool,
) -> ProtocolCapability {
    let existing = definition.pool.is_some() && definition.stage == AdapterStage::Existing;
    ProtocolCapability {
        protocol_id: definition.protocol_id,
        aliases: definition.aliases,
        status: if existing { "live" } else { "observe" },
        execution_backend: if existing {
            "sdk"
        } else {
            "sdk-adapter-pending"
        },
        pool_id_required: definition.legacy_pool_id_required,
        supports_buy: existing,
        supports_sell: existing,
        supports_direct_quote: existing,
        supports_sol_route: existing && definition.legacy_sol_route.enabled(stonkfun_sol_enabled),
        sdk_builder_available: definition.sdk_builder_available,
        token_programs: definition.legacy_token_programs,
    }
}

#[cfg(test)]
mod tests {
    use super::current;
    use std::collections::HashSet;

    #[test]
    fn capability_ids_are_unique_and_staged_protocols_are_closed() {
        let response = current(false);
        let ids = response
            .protocols
            .iter()
            .map(|item| item.protocol_id)
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), response.protocols.len());
        let clmm = response
            .protocols
            .iter()
            .find(|item| item.protocol_id == "raydium-clmm")
            .unwrap();
        assert_eq!(clmm.status, "observe");
        assert!(!clmm.supports_buy);
        assert!(clmm.sdk_builder_available);
    }

    #[test]
    fn stonkfun_sol_capability_reflects_configuration() {
        for enabled in [false, true] {
            let response = current(enabled);
            let stonkfun = response
                .protocols
                .iter()
                .find(|item| item.protocol_id == "stonkfun")
                .unwrap();
            assert_eq!(stonkfun.supports_sol_route, enabled);
        }
    }

    #[test]
    fn only_live_protocols_expose_trade_operations() {
        for capability in current(false).protocols {
            if capability.status == "observe" {
                assert!(!capability.supports_buy);
                assert!(!capability.supports_sell);
                assert!(!capability.supports_direct_quote);
                assert!(!capability.supports_sol_route);
            }
        }
    }
}
