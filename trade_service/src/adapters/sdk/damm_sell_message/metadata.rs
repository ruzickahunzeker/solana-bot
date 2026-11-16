use super::*;
use crate::adapters::sdk::damm_instructions::{relay, relay_name};
use crate::domain::sdk_candidate::{DammCandidateDetails, DammCandidatePool, SdkCandidateMetadata};
use crate::domain::sdk_prepared_record::SdkPreparedRecord;
use sol_trade_sdk::trading::core::params::MeteoraDammV2Params;
use solana_sdk::hash::Hash;

pub(super) fn pool_snapshot(request: &TradeSellParams) -> Result<DammCandidatePool> {
    let DexParamEnum::MeteoraDammV2(p) = &request.extension_params else {
        anyhow::bail!("DAMM candidate pool parameters missing");
    };
    Ok(crate::adapters::sdk::damm_instructions::pool_snapshot(p))
}

pub(super) fn capture(
    profile: &DammSellMessageProfile,
    destination: Option<Pubkey>,
    message: &[u8],
) -> Result<SdkPreparedRecord> {
    Ok(SdkCandidateMetadata::damm_sell(details(profile, destination)?, message)?.into())
}

pub(super) fn details(
    profile: &DammSellMessageProfile,
    destination: Option<Pubkey>,
) -> Result<DammCandidateDetails> {
    let wallet = Address::from_bytes(profile.context.wallet.to_bytes());
    let mint = Address::from_bytes(profile.intent.mint.to_bytes());
    let input = profile.holding.checked_input(wallet, mint)?;
    Ok(DammCandidateDetails {
        wallet: wallet.to_bytes(),
        mint: mint.to_bytes(),
        quote_mint: profile.intent.quote_mint.to_bytes(),
        pool_hint: profile.intent.pool_id.map(|p| p.to_bytes()),
        pool: profile
            .pool_metadata
            .clone()
            .context("DAMM candidate pool snapshot missing")?,
        balance_units: profile.balance,
        balance_slot: profile.holding.checked_slot(wallet, mint)?,
        input_account: input.account.to_bytes(),
        input_program: input.program.to_bytes(),
        input_decimals: input.decimals,
        input_units: profile.input_amount()?,
        sell_percent: profile.percent,
        minimum_output_units: profile.minimum,
        slippage_bps: profile.intent.slippage_bps,
        tip_sol_bits: profile.intent.tip_sol.to_bits(),
        relay: relay_name(profile.context.provider).into(),
        tip_destination: destination.map(|v| v.to_bytes()),
        recent_blockhash: profile.intent.recent_blockhash.to_bytes(),
        simulate: profile.intent.simulate,
        use_mev: profile.intent.use_mev,
        requested_mev_provider: profile.intent.mev_provider,
        wait_tx_confirmed: profile.intent.wait_tx_confirmed,
        mev_submit_verify_ms: profile.intent.mev_submit_verify_ms,
    })
}

/// Reconstructs nominal parameters without RPC or a signer; not independent chain authorization.
pub(crate) fn validate_candidate(metadata: &SdkPreparedRecord, message: &[u8]) -> Result<()> {
    metadata.validate(message)?;
    let d = metadata.sell()?.details();
    let key = Pubkey::new_from_array;
    let provider = relay(&d.relay)?;
    let p = &d.pool;
    let mut pool = MeteoraDammV2Params::new(
        key(p.pool),
        key(p.token_a_vault),
        key(p.token_b_vault),
        key(p.token_a_mint),
        key(p.token_b_mint),
        key(p.token_a_program),
        key(p.token_b_program),
    )
    .with_swap_mode(0)
    .with_rate_limiter_sysvar(p.rate_limiter_sysvar);
    pool.referral_token_account = p.referral.map(key);
    let intent = ExecuteTrade {
        pool: Pool::MeteoraDammV2,
        mint: key(d.mint),
        quote_mint: key(d.quote_mint),
        pool_id: d.pool_hint.map(key),
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: None,
        sell_percent: Some(d.sell_percent),
        slippage_bps: d.slippage_bps,
        tip_sol: f64::from_bits(d.tip_sol_bits),
        recent_blockhash: Hash::new_from_array(d.recent_blockhash),
        wait_tx_confirmed: d.wait_tx_confirmed,
        mev_submit_verify_ms: d.mev_submit_verify_ms,
        simulate: d.simulate,
        use_mev: d.use_mev,
        mev_provider: d.requested_mev_provider,
    };
    let request = super::super::ordinary_params::ordinary_sell_params(
        &intent,
        DexParamEnum::MeteoraDammV2(pool),
        Some(d.minimum_output_units),
        d.sell_percent,
        provider,
    )?;
    ensure!(
        output_mint(request.output_token_type) == key(d.quote_mint)
            && [WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT].contains(&key(d.quote_mint)),
        "SDK candidate output asset mismatch"
    );
    let program = if d.mint == p.token_a_mint {
        p.token_a_program
    } else {
        p.token_b_program
    };
    let account = sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
        &key(d.wallet),
        &key(d.mint),
        &key(program),
    );
    ensure!(
        d.input_program == program && d.input_account == account.to_bytes(),
        "SDK candidate raw input identity mismatch"
    );
    let protocol = instructions::protocol(
        key(d.wallet),
        &request,
        d.input_units,
        d.minimum_output_units,
    )?;
    let full = candidate::full_instructions(
        key(d.wallet),
        provider,
        d.simulate,
        intent.tip_sol,
        d.tip_destination.map(key),
        &protocol,
    )?;
    let expected = candidate::compile_candidate(key(d.wallet), &full, intent.recent_blockhash)?;
    ensure!(
        expected.message.serialize() == message,
        "SDK candidate metadata does not reconstruct the original message"
    );
    Ok(())
}

#[cfg(test)]
pub(crate) fn candidate_fixture(
    provider: SwqosType,
    simulate: bool,
    percent: u64,
) -> DammSellCandidate {
    let fixture = super::tests::fixture(provider, simulate, percent);
    let destination = if !simulate && provider != SwqosType::Default {
        Some(candidate::tip_accounts(provider)[0])
    } else {
        None
    };
    DammSellCandidate::build(fixture.profile().unwrap(), destination).unwrap()
}

#[cfg(test)]
pub(crate) fn candidate_fixture_for_wallet(wallet: Pubkey) -> DammSellCandidate {
    candidate_fixture_for_wallet_with_mev(wallet, Some(false), None)
}

#[cfg(test)]
pub(crate) fn candidate_fixture_for_wallet_with_mev(
    wallet: Pubkey,
    use_mev: Option<bool>,
    requested: Option<crate::domain::trade::MevProvider>,
) -> DammSellCandidate {
    let mut f = super::tests::case_for_wallet(
        SwqosType::Default,
        false,
        100,
        WSOL_TOKEN_ACCOUNT,
        false,
        spl_token_interface::ID,
        wallet,
    );
    f.intent.use_mev = use_mev;
    f.intent.mev_provider = requested;
    DammSellCandidate::build(f.profile().unwrap(), None).unwrap()
}

#[cfg(test)]
mod tests;
