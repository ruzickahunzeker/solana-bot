use crate::domain::trade::Address;

/// A read-only lease on an already loaded wallet, not permission to sign.
pub(crate) trait LoadedWalletIdentity: Send + Sync {
    fn pubkey(&self) -> Address;
}

pub(crate) trait LoadedWalletDirectory: Send + Sync {
    fn resolve_id(&self, requested: Option<&str>) -> String;
    fn lookup(&self, wallet_id: &str) -> Option<Box<dyn LoadedWalletIdentity>>;
}
