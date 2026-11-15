use crate::{
    application::{
        ordinary_claims::{operation_for_claim, ClaimOperation},
        ports::ordinary_journal::TradeJournalMode,
        trade_admission::AdmittedTrade,
    },
    domain::{
        sdk_candidate::SdkCandidateMetadata,
        trade::{Address, TradeAmount},
    },
};
use anyhow::{ensure, Result};

/// Intent binding only. Persisted nominal metadata never grants signing authority.
pub(crate) fn validate_origin(
    execution: &AdmittedTrade,
    metadata: &SdkCandidateMetadata,
    message: &[u8],
) -> Result<()> {
    ensure!(
        operation_for_claim(execution, TradeJournalMode::Durable)? == ClaimOperation::SdkTrade,
        "SDK preparation requires its original durable operation"
    );
    metadata.validate(message)?;
    let d = metadata.details();
    let c = execution.command();
    let TradeAmount::SellPercent { percent } = c.amount else {
        anyhow::bail!("SDK sell amount missing")
    };
    ensure!(
        Address::from_bytes(d.wallet).to_string() == execution.reservation().wallet_pubkey()
            && d.mint == c.route.mint.to_bytes()
            && d.quote_mint == c.route.quote_mint.to_bytes()
            && d.pool_hint == c.route.pool_id.map(|v| v.to_bytes())
            && d.sell_percent == percent
            && d.slippage_bps == c.execution.slippage_bps
            && d.tip_sol_bits == c.execution.tip_sol.to_bits()
            && !d.simulate
            && d.use_mev == c.execution.use_mev
            && d.requested_mev_provider == c.execution.mev_provider,
        "SDK candidate differs from the original wallet or execution intent"
    );
    Ok(())
}

pub(crate) struct StoredSdkCandidate {
    pub(crate) metadata: SdkCandidateMetadata,
    pub(crate) message: Vec<u8>,
}
