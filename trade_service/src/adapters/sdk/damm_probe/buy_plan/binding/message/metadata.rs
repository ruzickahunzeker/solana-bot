use super::*;
use crate::{
    adapters::sdk::damm_instructions::{pool_snapshot, relay, relay_name},
    domain::sdk_buy_candidate::{BuyFunding, DammBuyCandidateRecord, DammBuyDetails},
};
use sol_trade_sdk::{
    constants::{USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    trading::core::params::MeteoraDammV2Params,
};

pub(super) fn capture(
    profile: &DammBuyMessage,
    destination: Option<Pubkey>,
    message: &[u8],
) -> Result<DammBuyCandidateRecord> {
    profile.recheck()?;
    let q = &profile.quote;
    let a = &q.plan.accounts;
    let (native, (input_decimals, output_decimals)) = q.evidence.record_funding();
    let (account_slot, simulation_slot, _) = q.evidence.context();
    let a_in = a.input_mint == a.pool.token_a_mint;
    let (input_program, output_program) = if a_in {
        (a.pool.token_a_program, a.pool.token_b_program)
    } else {
        (a.pool.token_b_program, a.pool.token_a_program)
    };
    let tip = if !q.intent.simulate && q.context.provider != SwqosType::Default {
        tip_lamports(q.intent.tip_sol)
    } else {
        0
    };
    DammBuyCandidateRecord::new(
        DammBuyDetails {
            wallet: a.wallet.to_bytes(),
            mint: a.output_mint.to_bytes(),
            quote_mint: a.input_mint.to_bytes(),
            pool_hint: q.intent.pool_id.map(|p| p.to_bytes()),
            pool: pool_snapshot(&a.pool),
            funding: match q.plan.funding {
                Funding::NativeSol => BuyFunding::NativeSol,
                Funding::ExistingUsdc => BuyFunding::ExistingQuote,
            },
            input_account: a.input_ata.to_bytes(),
            input_program: input_program.to_bytes(),
            input_decimals,
            output_account: a.output_ata.to_bytes(),
            output_program: output_program.to_bytes(),
            output_decimals,
            input_units: a.amount,
            quoted_net_output_units: q.output_units(),
            minimum_output_units: q.minimum,
            account_slot,
            simulation_slot,
            native: native.clone(),
            priority_fee_lamports: 300,
            tip_lamports: tip,
            create_output_account: profile.request.create_mint_ata,
            output_initialized: q.evidence.output_initialized(),
            slippage_bps: q.intent.slippage_bps,
            tip_sol_bits: q.intent.tip_sol.to_bits(),
            relay: relay_name(q.context.provider).into(),
            tip_destination: destination.map(|p| p.to_bytes()),
            recent_blockhash: q.intent.recent_blockhash.to_bytes(),
            simulate: q.intent.simulate,
            use_mev: q.intent.use_mev,
            requested_mev_provider: q.intent.mev_provider,
            wait_tx_confirmed: q.intent.wait_tx_confirmed,
            mev_submit_verify_ms: q.intent.mev_submit_verify_ms,
        },
        message,
    )
}

/// Nominal reconstruction only. It performs no RPC and cannot recreate fresh quote/signing authority.
pub(super) fn validate_record(record: &DammBuyCandidateRecord, message: &[u8]) -> Result<()> {
    record.validate(message)?;
    let d = record.details();
    let key = Pubkey::new_from_array;
    let provider = relay(&d.relay)?;
    let expected_tip = if !d.simulate && provider != SwqosType::Default {
        tip_lamports(f64::from_bits(d.tip_sol_bits))
    } else {
        0
    };
    ensure!(
        d.priority_fee_lamports == 300 && d.tip_lamports == expected_tip,
        "SDK buy candidate priority fee or tip mismatch"
    );
    let (funding, decimals) = match key(d.quote_mint) {
        WSOL_TOKEN_ACCOUNT => (BuyFunding::NativeSol, 9),
        USDC_TOKEN_ACCOUNT => (BuyFunding::ExistingQuote, 6),
        _ => anyhow::bail!("SDK buy candidate requires canonical Quote"),
    };
    ensure!(
        d.funding == funding
            && d.input_decimals == decimals
            && key(d.input_program) == spl_token_interface::ID,
        "SDK buy candidate input funding, program or decimals mismatch"
    );
    for (mint, decimals) in [(WSOL_TOKEN_ACCOUNT, 9), (USDC_TOKEN_ACCOUNT, 6)] {
        if key(d.mint) == mint {
            ensure!(
                d.output_decimals == decimals && key(d.output_program) == spl_token_interface::ID,
                "SDK buy candidate canonical output metadata mismatch"
            );
        }
    }
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
    let hash = solana_sdk::hash::Hash::new_from_array(d.recent_blockhash);
    let plan = super::super::super::prepare(
        key(d.wallet),
        &pool,
        key(d.quote_mint),
        key(d.mint),
        d.input_units,
        hash,
        false,
    )?;
    let a = &plan.accounts;
    let a_in = a.input_mint == pool.token_a_mint;
    let (input_program, output_program) = if a_in {
        (pool.token_a_program, pool.token_b_program)
    } else {
        (pool.token_b_program, pool.token_a_program)
    };
    ensure!(
        a.input_ata.to_bytes() == d.input_account
            && a.output_ata.to_bytes() == d.output_account
            && input_program.to_bytes() == d.input_program
            && output_program.to_bytes() == d.output_program,
        "SDK buy candidate account or program identity mismatch"
    );
    let instructions = protocol_for(&plan, d.minimum_output_units, d.create_output_account);
    let full = full_instructions(
        key(d.wallet),
        provider,
        d.simulate,
        f64::from_bits(d.tip_sol_bits),
        d.tip_destination.map(key),
        &instructions,
    )?;
    ensure!(
        compile_candidate(key(d.wallet), &full, hash)?
            .message
            .serialize()
            == message,
        "SDK buy record does not reconstruct its original message"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
