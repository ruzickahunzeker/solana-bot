use crate::{
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::meteora_dbc::{
            accounts, get_event_authority_pda, SWAP2_DISCRIMINATOR, SWAP_MODE_EXACT_IN,
        },
    },
    trading::core::{
        params::{MeteoraDbcParams, SwapParams},
        traits::InstructionBuilder,
    },
};
use anyhow::{anyhow, bail, Context};
use dynamic_bonding_curve_sdk::quote_exact_in::quote_exact_in;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signer::Signer,
};

pub struct MeteoraDbcInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for MeteoraDbcInstructionBuilder {
    async fn build_buy_instructions(
        &self,
        params: &SwapParams,
    ) -> anyhow::Result<Vec<Instruction>> {
        let protocol = dbc_params(params)?;
        validate_quote_mint(protocol)?;
        build_exact_in(params, protocol, false, true).await
    }

    async fn build_sell_instructions(
        &self,
        params: &SwapParams,
    ) -> anyhow::Result<Vec<Instruction>> {
        let protocol = dbc_params(params)?;
        validate_quote_mint(protocol)?;
        build_exact_in(params, protocol, true, false).await
    }
}

fn dbc_params(params: &SwapParams) -> anyhow::Result<&MeteoraDbcParams> {
    params
        .protocol_params
        .as_any()
        .downcast_ref::<MeteoraDbcParams>()
        .ok_or_else(|| anyhow!("Invalid protocol params for MeteoraDbc"))
}

fn validate_quote_mint(params: &MeteoraDbcParams) -> anyhow::Result<()> {
    if params.quote_mint != crate::constants::WSOL_TOKEN_ACCOUNT
        && params.quote_mint != crate::constants::USDC_TOKEN_ACCOUNT
    {
        bail!("Meteora DBC quote mint must be WSOL or USDC");
    }
    Ok(())
}

async fn build_exact_in(
    params: &SwapParams,
    protocol: &MeteoraDbcParams,
    swap_base_for_quote: bool,
    is_buy: bool,
) -> anyhow::Result<Vec<Instruction>> {
    let amount_in = params.input_amount.filter(|amount| *amount > 0).context(if is_buy {
        "Meteora DBC buy amount is not set"
    } else {
        "Meteora DBC sell amount is not set"
    })?;
    let minimum_amount_out =
        quote_minimum_amount_out(params, protocol, swap_base_for_quote, amount_in).await?;

    let (input_mint, input_program, output_mint, output_program) = if swap_base_for_quote {
        (
            protocol.base_mint,
            protocol.base_token_program,
            protocol.quote_mint,
            protocol.quote_token_program,
        )
    } else {
        (
            protocol.quote_mint,
            protocol.quote_token_program,
            protocol.base_mint,
            protocol.base_token_program,
        )
    };
    let payer = params.payer.pubkey();
    let input_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &payer,
            &input_mint,
            &input_program,
            params.open_seed_optimize,
        );
    let output_token_account =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &payer,
            &output_mint,
            &output_program,
            params.open_seed_optimize,
        );

    let mut instructions = Vec::with_capacity(6);
    if params.create_input_mint_ata {
        push_create_or_wrap_user_token_account(
            &mut instructions,
            &payer,
            &input_mint,
            &input_program,
            amount_in,
            params.open_seed_optimize,
        );
    }
    if params.create_output_mint_ata {
        push_create_user_token_account(
            &mut instructions,
            &payer,
            &output_mint,
            &output_program,
            params.open_seed_optimize,
        );
    }

    let mut metas = Vec::with_capacity(16);
    metas.extend([
        accounts::POOL_AUTHORITY_META,
        AccountMeta::new_readonly(protocol.config, false),
        AccountMeta::new(protocol.pool, false),
        AccountMeta::new(input_token_account, false),
        AccountMeta::new(output_token_account, false),
        AccountMeta::new(protocol.base_vault, false),
        AccountMeta::new(protocol.quote_vault, false),
        AccountMeta::new_readonly(protocol.base_mint, false),
        AccountMeta::new_readonly(protocol.quote_mint, false),
        AccountMeta::new(payer, true),
        AccountMeta::new_readonly(protocol.base_token_program, false),
        AccountMeta::new_readonly(protocol.quote_token_program, false),
    ]);
    metas.push(match protocol.referral_token_account {
        Some(referral) => AccountMeta::new(referral, false),
        None => accounts::PROGRAM_META,
    });
    metas.extend([
        AccountMeta::new_readonly(get_event_authority_pda(), false),
        accounts::PROGRAM_META,
    ]);
    if protocol.include_instructions_sysvar {
        metas.push(accounts::SYSVAR_INSTRUCTIONS_META);
    }

    let mut data = [0_u8; 25];
    data[..8].copy_from_slice(&SWAP2_DISCRIMINATOR);
    data[8..16].copy_from_slice(&amount_in.to_le_bytes());
    data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
    data[24] = SWAP_MODE_EXACT_IN;
    instructions.push(Instruction::new_with_bytes(accounts::PROGRAM, &data, metas));

    if params.close_output_mint_ata {
        push_close_wsol_if_needed(&mut instructions, &payer, &output_mint);
    }
    if params.close_input_mint_ata {
        push_close_wsol_if_needed(&mut instructions, &payer, &input_mint);
    }
    Ok(instructions)
}

async fn quote_minimum_amount_out(
    params: &SwapParams,
    protocol: &MeteoraDbcParams,
    swap_base_for_quote: bool,
    amount_in: u64,
) -> anyhow::Result<u64> {
    let rpc = params.rpc.as_ref().context("Meteora DBC quote requires an RPC client")?;
    let current_slot = rpc.get_slot().await?;
    let current_timestamp = if protocol.pool_config.activation_type == 1 {
        rpc.get_block_time(current_slot)
            .await
            .context("failed to fetch the current DBC block time")?
            .max(0) as u64
    } else {
        chrono::Utc::now().timestamp().max(0) as u64
    };
    let quote = quote_exact_in(
        &protocol.virtual_pool.pool_state,
        &protocol.pool_config,
        swap_base_for_quote,
        current_timestamp,
        current_slot,
        amount_in,
        protocol.referral_token_account.is_some(),
        false,
    )
    .map_err(|error| anyhow!("Meteora DBC quote failed: {error}"))?;
    let slippage_bps = params.slippage_basis_points.unwrap_or(0);
    if slippage_bps > 10_000 {
        bail!("slippage_basis_points cannot exceed 10000");
    }
    let minimum = (u128::from(quote.output_amount)
        * u128::from(10_000_u64.saturating_sub(slippage_bps))
        / 10_000) as u64;
    Ok(minimum.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap2_data_layout_is_exact_in() {
        let amount_in = 1_000_000_u64;
        let minimum_out = 42_u64;
        let mut data = [0_u8; 25];
        data[..8].copy_from_slice(&SWAP2_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_out.to_le_bytes());
        data[24] = SWAP_MODE_EXACT_IN;
        assert_eq!(&data[..8], &[65, 75, 63, 76, 235, 91, 91, 136]);
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), amount_in);
        assert_eq!(u64::from_le_bytes(data[16..24].try_into().unwrap()), minimum_out);
        assert_eq!(data[24], 0);
    }
}
