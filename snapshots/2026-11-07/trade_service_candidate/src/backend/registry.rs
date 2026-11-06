//! Runtime capability catalog. It contains no SDK-specific types.

use std::collections::BTreeMap;

use anyhow::{bail, Result};

use super::types::BackendCapabilities;
use crate::models::Pool;

#[derive(Debug, Default, Clone)]
pub struct BackendCatalog {
    backends: BTreeMap<String, BackendCapabilities>,
    protocol_to_backend: BTreeMap<String, String>,
}

impl BackendCatalog {
    /// Catalog for the SDK 11 adapter. The list is deliberately explicit so a
    /// newly added protocol cannot reach the executor until its adapter is
    /// registered and tested.
    pub fn sdk11() -> Result<Self> {
        let mut catalog = Self::default();
        catalog.register(
            "sdk-11",
            BackendCapabilities {
                protocols: vec![
                    "pump".to_string(),
                    "pump-amm".to_string(),
                    "bonk".to_string(),
                    "stonkfun".to_string(),
                    "stonkfun-swap".to_string(),
                    "meteora-damm-v2".to_string(),
                    "raydium-amm-v4".to_string(),
                    "raydium-cpmm".to_string(),
                ],
                supports_buy: true,
                supports_sell: true,
                supports_simulate: true,
                supports_live: false,
            },
        )?;
        Ok(catalog)
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        capabilities: BackendCapabilities,
    ) -> Result<()> {
        let name = name.into();
        if name.trim().is_empty() {
            bail!("backend name is empty");
        }
        if self.backends.contains_key(&name) {
            bail!("backend already registered: {name}");
        }
        for protocol in &capabilities.protocols {
            if self.protocol_to_backend.contains_key(protocol) {
                bail!("protocol already registered: {protocol}");
            }
        }
        let protocols = capabilities.protocols.clone();
        self.backends.insert(name.clone(), capabilities);
        for protocol in protocols {
            self.protocol_to_backend.insert(protocol, name.clone());
        }
        Ok(())
    }

    pub fn backend_for_protocol(&self, protocol: &str) -> Option<&str> {
        self.protocol_to_backend.get(protocol).map(String::as_str)
    }

    pub fn capabilities(&self, name: &str) -> Option<&BackendCapabilities> {
        self.backends.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.backends.keys().map(String::as_str)
    }

    pub fn backend_for_pool(&self, pool: Pool) -> Option<&str> {
        self.backend_for_protocol(protocol_id(pool))
    }
}

pub fn protocol_id(pool: Pool) -> &'static str {
    match pool {
        Pool::Pump => "pump",
        Pool::PumpAmm => "pump-amm",
        Pool::Bonk => "bonk",
        Pool::StonkFun => "stonkfun",
        Pool::StonkFunSwap => "stonkfun-swap",
        Pool::MeteoraDammV2 => "meteora-damm-v2",
        Pool::MeteoraDbc => "meteora-dbc",
        Pool::RaydiumAmmV4 => "raydium-amm-v4",
        Pool::RaydiumCpmm => "raydium-cpmm",
    }
}

pub fn pool_for_protocol(protocol: &str) -> Option<Pool> {
    match protocol {
        "pump" => Some(Pool::Pump),
        "pump-amm" => Some(Pool::PumpAmm),
        "bonk" => Some(Pool::Bonk),
        "stonkfun" => Some(Pool::StonkFun),
        "stonkfun-swap" => Some(Pool::StonkFunSwap),
        "meteora-damm-v2" => Some(Pool::MeteoraDammV2),
        "meteora-dbc" => Some(Pool::MeteoraDbc),
        "raydium-amm-v4" => Some(Pool::RaydiumAmmV4),
        "raydium-cpmm" => Some(Pool::RaydiumCpmm),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(protocols: &[&str]) -> BackendCapabilities {
        BackendCapabilities {
            protocols: protocols.iter().map(|v| (*v).to_string()).collect(),
            supports_buy: true,
            supports_sell: true,
            supports_simulate: true,
            supports_live: false,
        }
    }

    #[test]
    fn routes_protocol_to_backend() {
        let mut catalog = BackendCatalog::default();
        catalog
            .register("sdk11", caps(&["pump", "pump-amm"]))
            .unwrap();
        assert_eq!(catalog.backend_for_protocol("pump"), Some("sdk11"));
        assert!(catalog.capabilities("sdk11").unwrap().supports_simulate);
    }

    #[test]
    fn rejects_duplicate_protocols() {
        let mut catalog = BackendCatalog::default();
        catalog.register("sdk11", caps(&["pump"])).unwrap();
        assert!(catalog.register("legacy", caps(&["pump"])).is_err());
    }

    #[test]
    fn sdk11_catalog_keeps_dbc_out_of_execution() {
        let catalog = BackendCatalog::sdk11().unwrap();
        assert_eq!(catalog.backend_for_pool(Pool::Pump), Some("sdk-11"));
        assert_eq!(catalog.backend_for_pool(Pool::RaydiumCpmm), Some("sdk-11"));
        assert_eq!(catalog.backend_for_pool(Pool::MeteoraDbc), None);
        assert_eq!(protocol_id(Pool::MeteoraDbc), "meteora-dbc");
    }
}
