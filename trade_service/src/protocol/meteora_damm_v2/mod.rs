use anyhow::{bail, Context};

pub(crate) fn minimum_output(quoted: u64, slippage_bps: u64) -> anyhow::Result<u64> {
    if quoted == 0 || slippage_bps >= 10_000 {
        bail!("invalid DAMM v2 quote or slippage");
    }
    let minimum = (quoted as u128)
        .checked_mul((10_000 - slippage_bps) as u128)
        .context("DAMM v2 minimum output overflow")?
        / 10_000;
    u64::try_from(minimum)
        .ok()
        .filter(|value| *value > 0)
        .context("DAMM v2 minimum output rounds to zero")
}
