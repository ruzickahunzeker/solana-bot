use serde::Serialize;

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
