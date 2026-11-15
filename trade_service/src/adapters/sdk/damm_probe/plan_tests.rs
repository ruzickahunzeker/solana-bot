use super::*;
use sol_trade_sdk::{
    constants::{USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    trading::core::params::MeteoraDammV2Params,
};
use solana_sdk::signature::Signature;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn pool() -> MeteoraDammV2Params {
    MeteoraDammV2Params::new(
        key(1),
        key(2),
        key(3),
        key(4),
        USDC_TOKEN_ACCOUNT,
        spl_token_2022_interface::ID,
        spl_token_interface::ID,
    )
}
fn prepare(
    pool: &MeteoraDammV2Params,
    input: Pubkey,
    output: Pubkey,
    amount: u64,
    seed: bool,
) -> Result<PreparedDammProbe> {
    prepare_damm_probe(
        key(5),
        pool,
        input,
        output,
        amount,
        Hash::new_from_array([6; 32]),
        seed,
    )
}

#[test]
fn public_plan_preserves_input_parameters_and_never_has_a_nonzero_signature() {
    let source = pool();
    let original_pool = source.pool;
    let prepared = prepare(&source, key(4), USDC_TOKEN_ACCOUNT, 123, false).unwrap();
    assert_eq!(source.pool, original_pool);
    assert_eq!(prepared.transaction().signatures, [Signature::default()]);
    assert_eq!(
        prepared
            .transaction()
            .message
            .header
            .num_required_signatures,
        1
    );
    assert_eq!(prepared.transaction().message.account_keys[0], key(5));
    assert_eq!(
        prepared.transaction().message.recent_blockhash,
        Hash::new_from_array([6; 32])
    );
    assert_eq!(prepared.transaction().message.instructions.len(), 2);
    assert_eq!(
        prepared.transaction().message.instructions[1].data[8..16],
        123u64.to_le_bytes()
    );
    assert_eq!(
        prepared.transaction().message.instructions[1].data[16..24],
        1u64.to_le_bytes()
    );
}

#[test]
fn unsupported_profile_and_legacy_invalid_amount_direction_mode_errors_fail_without_capabilities() {
    let mut source = pool();
    for (input, output, amount, expected) in [
        (key(4), USDC_TOKEN_ACCOUNT, 0, "Token amount is not set"),
        (
            key(99),
            USDC_TOKEN_ACCOUNT,
            1,
            "Input/output mints must match opposite DAMM v2 pool sides",
        ),
        (
            key(4),
            key(4),
            1,
            "Input/output mints must match opposite DAMM v2 pool sides",
        ),
    ] {
        assert_eq!(
            prepare(&source, input, output, amount, false)
                .err()
                .unwrap()
                .to_string(),
            expected
        );
    }
    source.swap_mode = 255;
    assert_eq!(
        prepare(&source, key(4), USDC_TOKEN_ACCOUNT, 1, false)
            .err()
            .unwrap()
            .to_string(),
        "Unsupported MeteoraDammV2 swap_mode 255"
    );
    source.swap_mode = 1;
    assert_eq!(
        prepare(&source, key(4), USDC_TOKEN_ACCOUNT, 1, true)
            .err()
            .unwrap()
            .to_string(),
        "DAMM v2 unsigned quote probe does not support seed-optimized accounts"
    );
    source.token_b_mint = key(98);
    assert_eq!(
        prepare(&source, key(4), key(98), 1, false)
            .err()
            .unwrap()
            .to_string(),
        "Pool must contain WSOL or USDC"
    );
}

#[test]
fn public_plan_binds_wallet_blockhash_pool_assets_amount_referral_and_sysvar() {
    let source = pool();
    let base = prepare(&source, key(4), USDC_TOKEN_ACCOUNT, 123, false).unwrap();
    let base_message = &base.transaction().message;
    for change in 0..10 {
        let mut candidate = source.clone();
        let mut wallet = key(5);
        let mut hash = Hash::new_from_array([6; 32]);
        let mut amount = 123;
        match change {
            0 => candidate.pool = key(90),
            1 => candidate.token_a_vault = key(90),
            2 => candidate.token_b_program = spl_token_2022_interface::ID,
            3 => candidate.referral_token_account = Some(key(90)),
            4 => candidate.include_rate_limiter_sysvar = true,
            5 => candidate.swap_mode = 2,
            6 => wallet = key(90),
            7 => hash = Hash::new_from_array([90; 32]),
            8 => amount = 124,
            _ => candidate.token_b_mint = WSOL_TOKEN_ACCOUNT,
        }
        let next = prepare_damm_probe(
            wallet,
            &candidate,
            key(4),
            candidate.token_b_mint,
            amount,
            hash,
            false,
        )
        .unwrap();
        assert_ne!(&next.transaction().message, base_message);
        assert_eq!(next.transaction().signatures, [Signature::default()]);
    }
}
