use super::*;
use sol_trade_sdk::common::{
    fast_fn::{
        create_associated_token_account_idempotent_fast,
        get_associated_token_address_with_program_id_fast,
    },
    spl_token::close_account,
};

pub(super) fn protocol(
    wallet: Pubkey,
    request: &TradeSellParams,
    amount: u64,
    minimum: u64,
) -> Result<Vec<Instruction>> {
    let DexParamEnum::MeteoraDammV2(pool) = &request.extension_params else {
        return Err(reject_balance("DAMM final protocol parameters are missing"));
    };
    let output = output_mint(request.output_token_type);
    let a_in = request.mint == pool.token_a_mint;
    ensure!(
        pool.token_a_mint != pool.token_b_mint
            && ((a_in && output == pool.token_b_mint)
                || (request.mint == pool.token_b_mint && output == pool.token_a_mint)),
        "DAMM final input/output do not match opposite pool sides"
    );
    ensure!(
        [pool.token_a_program, pool.token_b_program]
            .iter()
            .all(
                |program| [spl_token_interface::ID, spl_token_2022_interface::ID].contains(program)
            ),
        "DAMM final token program is unsupported"
    );
    let (input_program, output_program) = if a_in {
        (pool.token_a_program, pool.token_b_program)
    } else {
        (pool.token_b_program, pool.token_a_program)
    };
    ensure!(
        output_program == spl_token_interface::ID,
        "DAMM canonical output asset has an unexpected token program"
    );
    let input_ata =
        get_associated_token_address_with_program_id_fast(&wallet, &request.mint, &input_program);
    let output_ata =
        get_associated_token_address_with_program_id_fast(&wallet, &output, &output_program);
    let mut instructions =
        create_associated_token_account_idempotent_fast(&wallet, &wallet, &output, &output_program);
    instructions.push(super::super::damm_instructions::swap_instruction(
        wallet, pool, input_ata, output_ata, amount, minimum,
    ));
    if request.close_mint_token_ata {
        instructions.push(close_account(
            &input_program,
            &input_ata,
            &wallet,
            &wallet,
            &[&wallet],
        )?);
    }
    Ok(instructions)
}
