use crate::domain::trade::Address;

/// Pure syntax parsing only: no RPC, signer, normalization or pool verification.
pub(crate) trait TradeAddressParser {
    fn parse(&self, value: &str) -> anyhow::Result<Address>;
}
