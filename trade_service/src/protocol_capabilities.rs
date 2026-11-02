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
    pub token_programs: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtocolCapabilitiesResponse {
    pub schema_version: u32,
    pub generated_at_ms: u128,
    pub protocols: Vec<ProtocolCapability>,
}

pub fn current(stonkfun_sol_enabled: bool) -> ProtocolCapabilitiesResponse {
    ProtocolCapabilitiesResponse {
        schema_version: 1,
        generated_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis()),
        protocols: vec![
            live(
                "pump",
                &["pumpfun", "pump-fun"],
                false,
                true,
                true,
                true,
                true,
            ),
            live(
                "pump-amm",
                &["pumpswap", "pump-swap"],
                true,
                true,
                true,
                true,
                false,
            ),
            live("bonk", &[], false, true, true, true, false),
            live(
                "stonkfun",
                &["stonk-fun"],
                true,
                true,
                true,
                true,
                stonkfun_sol_enabled,
            ),
            live(
                "stonkfun-swap",
                &["stonk-fun-swap"],
                true,
                true,
                true,
                true,
                stonkfun_sol_enabled,
            ),
            live(
                "meteora-damm-v2",
                &["meteora", "meteora-dammv2"],
                true,
                true,
                true,
                true,
                false,
            ),
            live(
                "meteora-dbc",
                &["dbc", "dynamic-bonding-curve"],
                true,
                true,
                true,
                true,
                false,
            ),
            live(
                "raydium-amm-v4",
                &["raydium-v4", "raydium-ammv4"],
                true,
                true,
                true,
                true,
                false,
            ),
            live(
                "raydium-cpmm",
                &["raydium-cp"],
                true,
                true,
                true,
                true,
                false,
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

fn live(
    protocol_id: &'static str,
    aliases: &'static [&'static str],
    pool_id_required: bool,
    supports_buy: bool,
    supports_sell: bool,
    supports_direct_quote: bool,
    supports_sol_route: bool,
) -> ProtocolCapability {
    ProtocolCapability {
        protocol_id,
        aliases,
        status: "live",
        execution_backend: "sdk",
        pool_id_required,
        supports_buy,
        supports_sell,
        supports_direct_quote,
        supports_sol_route,
        sdk_builder_available: true,
        token_programs: &["spl-token"],
    }
}

fn staged(
    protocol_id: &'static str,
    aliases: &'static [&'static str],
    sdk_builder_available: bool,
) -> ProtocolCapability {
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
        token_programs: &["spl-token"],
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
        let enabled = current(true);
        let stonkfun = enabled
            .protocols
            .iter()
            .find(|item| item.protocol_id == "stonkfun")
            .unwrap();
        assert!(stonkfun.supports_sol_route);

        let disabled = current(false);
        let stonkfun = disabled
            .protocols
            .iter()
            .find(|item| item.protocol_id == "stonkfun")
            .unwrap();
        assert!(!stonkfun.supports_sol_route);
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
