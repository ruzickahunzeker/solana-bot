use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct ProtocolCapability {
    pub protocol_id: &'static str,
    pub aliases: &'static [&'static str],
    pub status: &'static str,
    pub execution_backend: &'static str,
    pub pool_id_required: bool,
    pub supports_buy: bool,
    pub supports_sell: bool,
    pub supports_direct_quote: bool,
    pub supports_sol_route: bool,
    pub sdk_builder_available: bool,
    /// Known SDK package route family; this is not route-specific build proof.
    pub sdk_package_route_builder: &'static str,
    /// The SOL route adapter actually wired into this service. It may use a
    /// different builder than the package family named above.
    pub service_sol_route_adapter: &'static str,
    pub token_programs: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtocolCapabilitiesResponse {
    pub schema_version: u32,
    pub generated_at_ms: u128,
    pub jupiter_trade_requires_sdk_unsupported: bool,
    pub protocols: Vec<ProtocolCapability>,
}

pub fn current(stonkfun_sol_enabled: bool, simulate_only: bool) -> ProtocolCapabilitiesResponse {
    ProtocolCapabilitiesResponse {
        schema_version: 1,
        generated_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis()),
        jupiter_trade_requires_sdk_unsupported: true,
        protocols: vec![
            available(
                "pump",
                &["pumpfun", "pump-fun"],
                false,
                true,
                true,
                true,
                true,
                simulate_only,
            ),
            available(
                "pump-amm",
                &["pumpswap", "pump-swap"],
                true,
                true,
                true,
                true,
                false,
                simulate_only,
            ),
            available("bonk", &[], false, true, true, true, false, simulate_only),
            available(
                "stonkfun",
                &["stonk-fun"],
                true,
                true,
                true,
                true,
                stonkfun_sol_enabled,
                simulate_only,
            ),
            available(
                "stonkfun-swap",
                &["stonk-fun-swap"],
                true,
                true,
                true,
                true,
                stonkfun_sol_enabled,
                simulate_only,
            ),
            available(
                "meteora-damm-v2",
                &["meteora", "meteora-dammv2"],
                true,
                true,
                true,
                true,
                false,
                simulate_only,
            ),
            staged("meteora-dbc", &["dbc", "dynamic-bonding-curve"], false),
            available(
                "raydium-amm-v4",
                &["raydium-v4", "raydium-ammv4"],
                true,
                true,
                true,
                true,
                false,
                simulate_only,
            ),
            available(
                "raydium-cpmm",
                &["raydium-cp"],
                true,
                true,
                true,
                true,
                false,
                simulate_only,
            ),
            staged(
                "raydium-launchlab",
                &["raydium-launchpad", "raydium-launchlab"],
                true,
            ),
            staged("raydium-clmm", &[], true),
            staged("meteora-launchpad", &[], false),
            staged("meteora-damm-v1", &[], false),
            staged("meteora-dlmm", &[], true),
            staged("orca-whirlpool", &["whirlpool"], true),
        ],
    }
}

fn available(
    protocol_id: &'static str,
    aliases: &'static [&'static str],
    pool_id_required: bool,
    supports_buy: bool,
    supports_sell: bool,
    supports_direct_quote: bool,
    supports_sol_route: bool,
    simulate_only: bool,
) -> ProtocolCapability {
    let (sdk_package_route_builder, service_sol_route_adapter) =
        route_capability(protocol_id, supports_sol_route, simulate_only);
    ProtocolCapability {
        protocol_id,
        aliases,
        status: if simulate_only { "simulate" } else { "live" },
        execution_backend: "sdk",
        pool_id_required,
        supports_buy,
        supports_sell,
        supports_direct_quote,
        supports_sol_route,
        sdk_builder_available: true,
        sdk_package_route_builder,
        service_sol_route_adapter,
        token_programs: &["spl-token"],
    }
}

fn staged(
    protocol_id: &'static str,
    aliases: &'static [&'static str],
    sdk_builder_available: bool,
) -> ProtocolCapability {
    let (sdk_package_route_builder, _) = route_capability(protocol_id, false, true);
    ProtocolCapability {
        protocol_id,
        aliases,
        status: "observe",
        execution_backend: "sdk-adapter-pending",
        pool_id_required: true,
        supports_buy: false,
        supports_sell: false,
        supports_direct_quote: false,
        supports_sol_route: false,
        sdk_builder_available,
        sdk_package_route_builder,
        service_sol_route_adapter: "not_integrated",
        token_programs: &["spl-token"],
    }
}

fn route_capability(
    protocol_id: &str,
    supports_sol_route: bool,
    simulate_only: bool,
) -> (&'static str, &'static str) {
    match protocol_id {
        "pump" => ("PumpFunQuoteRoute", "direct_sdk"),
        "pump-amm" => (
            "PumpSwapQuoteRoute",
            if simulate_only {
                "cpmm_pumpswap_simulate_only"
            } else {
                "read_only_preview"
            },
        ),
        "stonkfun" | "stonkfun-swap" => (
            "StonkFunQuoteRoute",
            if supports_sol_route {
                "explicit_via_sol_request"
            } else {
                "disabled_by_config"
            },
        ),
        // The SDK package exposes these as explicit route legs. Route
        // discovery, snapshots, quotes and execution adapters remain separate.
        "raydium-amm-v4" => ("StonkFunQuoteRoute", "cpmm_amm_v4_simulate_only"),
        "raydium-cpmm" => ("StonkFunQuoteRoute", "cpmm_cpmm_simulate_only"),
        "raydium-clmm" | "orca-whirlpool" | "meteora-dlmm" => {
            ("StonkFunQuoteRoute", "not_integrated")
        }
        _ => ("not_fully_evaluated", "not_integrated"),
    }
}

#[cfg(test)]
mod tests {
    use super::current;
    use crate::{models::Pool, sdk_client::bridge_sdk_capability};
    use std::collections::HashSet;

    #[test]
    fn capability_ids_are_unique_and_staged_protocols_are_closed() {
        let response = current(false, false);
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
        let enabled = current(true, false);
        let stonkfun = enabled
            .protocols
            .iter()
            .find(|item| item.protocol_id == "stonkfun")
            .unwrap();
        assert!(stonkfun.supports_sol_route);

        let disabled = current(false, false);
        let stonkfun = disabled
            .protocols
            .iter()
            .find(|item| item.protocol_id == "stonkfun")
            .unwrap();
        assert!(!stonkfun.supports_sol_route);
    }

    #[test]
    fn only_live_protocols_expose_trade_operations() {
        for capability in current(false, false).protocols {
            if capability.status == "observe" {
                assert!(!capability.supports_buy);
                assert!(!capability.supports_sell);
                assert!(!capability.supports_direct_quote);
                assert!(!capability.supports_sol_route);
            }
        }
    }

    #[test]
    fn simulation_only_marks_available_protocols_as_simulate_and_dbc_observe() {
        let response = current(false, true);
        let pump = response
            .protocols
            .iter()
            .find(|item| item.protocol_id == "pump")
            .unwrap();
        assert_eq!(pump.status, "simulate");
        let dbc = response
            .protocols
            .iter()
            .find(|item| item.protocol_id == "meteora-dbc")
            .unwrap();
        assert_eq!(dbc.status, "observe");
        assert!(!dbc.supports_buy);
        assert!(!dbc.supports_sell);
    }

    #[test]
    fn package_route_families_do_not_imply_service_execution_or_jupiter_fallback() {
        let response = current(true, true);
        assert!(response.jupiter_trade_requires_sdk_unsupported);
        let pump_amm = response
            .protocols
            .iter()
            .find(|item| item.protocol_id == "pump-amm")
            .unwrap();
        assert_eq!(pump_amm.sdk_package_route_builder, "PumpSwapQuoteRoute");
        assert_eq!(
            pump_amm.service_sol_route_adapter,
            "cpmm_pumpswap_simulate_only"
        );
        assert!(!pump_amm.supports_sol_route);
        let stonkfun = response
            .protocols
            .iter()
            .find(|item| item.protocol_id == "stonkfun")
            .unwrap();
        assert_eq!(stonkfun.sdk_package_route_builder, "StonkFunQuoteRoute");
        assert_eq!(
            stonkfun.service_sol_route_adapter,
            "explicit_via_sol_request"
        );
        let clmm = response
            .protocols
            .iter()
            .find(|item| item.protocol_id == "raydium-clmm")
            .unwrap();
        assert_eq!(clmm.sdk_package_route_builder, "StonkFunQuoteRoute");
        assert_eq!(clmm.service_sol_route_adapter, "not_integrated");
        for protocol_id in ["raydium-clmm", "orca-whirlpool", "meteora-dlmm"] {
            let venue = response
                .protocols
                .iter()
                .find(|item| item.protocol_id == protocol_id)
                .unwrap();
            assert_eq!(venue.sdk_package_route_builder, "StonkFunQuoteRoute");
            assert_eq!(venue.service_sol_route_adapter, "not_integrated");
        }
        let amm_v4 = response
            .protocols
            .iter()
            .find(|item| item.protocol_id == "raydium-amm-v4")
            .unwrap();
        assert_eq!(amm_v4.sdk_package_route_builder, "StonkFunQuoteRoute");
        assert_eq!(
            amm_v4.service_sol_route_adapter,
            "cpmm_amm_v4_simulate_only"
        );
        let cpmm = response
            .protocols
            .iter()
            .find(|item| item.protocol_id == "raydium-cpmm")
            .unwrap();
        assert_eq!(cpmm.service_sol_route_adapter, "cpmm_cpmm_simulate_only");
        for (pool, protocol_id) in [
            (Pool::RaydiumCpmm, "raydium-cpmm"),
            (Pool::RaydiumAmmV4, "raydium-amm-v4"),
        ] {
            let summary = response
                .protocols
                .iter()
                .find(|item| item.protocol_id == protocol_id)
                .unwrap();
            let route = bridge_sdk_capability(pool);
            assert_eq!(
                summary.sdk_package_route_builder,
                route.package_builder_family
            );
            assert_eq!(summary.service_sol_route_adapter, route.service_adapter);
        }
    }
}
