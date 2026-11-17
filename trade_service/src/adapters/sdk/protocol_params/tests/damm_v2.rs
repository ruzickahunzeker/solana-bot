use super::{super::*, read_rpc::*};
use sol_trade_sdk::instruction::utils::{
    meteora_damm_v2::{accounts::METEORA_DAMM_V2, SWAP_MODE_PARTIAL_FILL},
    meteora_damm_v2_types::{pool_account_decode, Pool as DammPool, POOL_DISCRIMINATOR, POOL_SIZE},
};

fn pool_account(pool: &DammPool) -> solana_sdk::account::Account {
    // The SDK's projected current fields are Borsh-skipped but not serde-skipped.
    // Encode the raw compatibility layout explicitly, then verify the SDK decoder.
    let fees = &pool.pool_fees;
    let mut payload = bincode::serialize(&(
        &fees.base_fee,
        fees.protocol_fee_percent,
        fees.partner_fee_percent,
        fees.referral_fee_percent,
        fees.padding_0,
        &fees.dynamic_fee,
        fees.padding_1,
    ))
    .unwrap();
    macro_rules! append {
        ($($field:expr),* $(,)?) => { $(payload.extend(bincode::serialize(&$field).unwrap());)* };
    }
    append!(
        pool.token_a_mint,
        pool.token_b_mint,
        pool.token_a_vault,
        pool.token_b_vault,
        pool.whitelisted_vault,
        pool.partner,
        pool.liquidity,
        pool.padding,
        pool.protocol_a_fee,
        pool.protocol_b_fee,
        pool.partner_a_fee,
        pool.partner_b_fee,
        pool.sqrt_min_price,
        pool.sqrt_max_price,
        pool.sqrt_price,
        pool.activation_point,
        pool.activation_type,
        pool.pool_status,
        pool.token_a_flag,
        pool.token_b_flag,
        pool.collect_fee_mode,
        pool.pool_type,
        pool.padding_0,
        pool.fee_a_per_liquidity,
        pool.fee_b_per_liquidity,
        pool.permanent_lock_liquidity,
        pool.metrics,
        pool.padding_1,
        pool.reward_infos,
    );
    assert_eq!(payload.len(), POOL_SIZE);
    let mut data = POOL_DISCRIMINATOR.to_vec();
    data.extend(payload);
    assert_eq!(pool_account_decode(&data).unwrap(), *pool);
    account(METEORA_DAMM_V2, data)
}

pub(super) fn fixture() -> (ReadRpc, Pubkey, DammPool) {
    let id = Pubkey::new_from_array([51; 32]);
    let pool = DammPool {
        token_a_mint: Pubkey::new_from_array([52; 32]),
        token_b_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
        token_a_vault: Pubkey::new_from_array([53; 32]),
        token_b_vault: Pubkey::new_from_array([54; 32]),
        sqrt_price: (1u128 << 64) + 13,
        liquidity: 1u128 << 96,
        ..Default::default()
    };
    let mut fixture = ReadRpc::default();
    fixture.account(id, pool_account(&pool));
    fixture.account(
        pool.token_a_mint,
        account(spl_token_2022_interface::ID, super::cpmm::mint(6)),
    );
    fixture.account(
        pool.token_b_mint,
        account(spl_token_interface::ID, super::cpmm::mint(9)),
    );
    (fixture, id, pool)
}

#[tokio::test]
async fn damm_real_loader_preserves_accounts_programs_partial_fill_and_build_sysvar() {
    let (fixture, id, pool) = fixture();
    let rpc = fixture.client();
    let raw = MeteoraDammV2Params::from_pool_address_by_rpc(&rpc, &id)
        .await
        .unwrap();
    assert!(!raw.include_rate_limiter_sysvar);
    warmup_on_rpc(
        &rpc,
        Pool::MeteoraDammV2,
        &pool.token_a_mint,
        &pool.token_b_mint,
        Some(&id),
    )
    .await
    .unwrap();
    for (mint, quote) in [
        (pool.token_a_mint, pool.token_b_mint),
        (pool.token_b_mint, pool.token_a_mint),
    ] {
        let params = dex_params(
            &rpc,
            Pool::MeteoraDammV2,
            &mint,
            &quote,
            Some(&id),
            None,
            None,
        )
        .await
        .unwrap();
        let DexParamEnum::MeteoraDammV2(params) = params else {
            panic!("wrong DAMM variant")
        };
        assert_eq!(params.pool, id);
        assert_eq!(params.token_a_mint, pool.token_a_mint);
        assert_eq!(params.token_b_mint, pool.token_b_mint);
        assert_eq!(params.token_a_vault, pool.token_a_vault);
        assert_eq!(params.token_b_vault, pool.token_b_vault);
        assert_eq!(params.token_a_program, spl_token_2022_interface::ID);
        assert_eq!(params.token_b_program, spl_token_interface::ID);
        assert_eq!(params.swap_mode, SWAP_MODE_PARTIAL_FILL);
        assert!(params.include_rate_limiter_sysvar);
        assert!(params.referral_token_account.is_none());
    }
    fixture.assert_read_only();
    assert_eq!(fixture.calls().len(), 8);
}

#[tokio::test]
async fn damm_warmup_quote_only_does_not_expand_build_membership() {
    let (fixture, id, pool) = fixture();
    let rpc = fixture.client();
    let alien = Pubkey::new_from_array([99; 32]);
    warmup_on_rpc(
        &rpc,
        Pool::MeteoraDammV2,
        &alien,
        &pool.token_b_mint,
        Some(&id),
    )
    .await
    .unwrap();
    let error = dex_params(
        &rpc,
        Pool::MeteoraDammV2,
        &alien,
        &pool.token_b_mint,
        Some(&id),
        None,
        None,
    )
    .await
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("does not contain requested mint"));
    fixture.assert_read_only();
}

#[tokio::test]
async fn damm_missing_mints_and_invalid_pool_accounts_are_rejected() {
    for base in [true, false] {
        let (mut fixture, id, pool) = fixture();
        fixture.absent(if base {
            pool.token_a_mint
        } else {
            pool.token_b_mint
        });
        let error = dex_params(
            &fixture.client(),
            Pool::MeteoraDammV2,
            &pool.token_a_mint,
            &pool.token_b_mint,
            Some(&id),
            None,
            None,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert_eq!(
            error,
            if base {
                "Token A mint account not found"
            } else {
                "Token B mint account not found"
            }
        );
        assert_eq!(fixture.calls().len(), 2);
        fixture.assert_read_only();
    }
    for damage in 0..3 {
        let (mut fixture, id, pool) = fixture();
        let mut invalid = pool_account(&pool);
        match damage {
            0 => invalid.owner = Pubkey::default(),
            1 => invalid.data[0] ^= 1,
            _ => invalid.data.truncate(8 + POOL_SIZE - 1),
        }
        fixture.account(id, invalid);
        let error = dex_params(
            &fixture.client(),
            Pool::MeteoraDammV2,
            &pool.token_a_mint,
            &pool.token_b_mint,
            Some(&id),
            None,
            None,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert_eq!(
            error,
            if damage == 0 {
                "Account is not owned by Meteora Damm V2 program"
            } else {
                "Failed to decode pool"
            }
        );
        assert_eq!(fixture.calls().len(), 1);
        fixture.assert_read_only();
    }
}
