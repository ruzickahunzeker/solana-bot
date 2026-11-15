use super::{super::*, cpmm, pool_bytes, read_rpc::*, PoolState};
use sol_trade_sdk::{
    instruction::utils::bonk::{self, accounts},
    trading::core::params::{StonkFunMemeLeg, StonkFunSolHop},
};
use solana_client::rpc_request::RpcRequest;

pub(super) fn curve(fixture: &mut ReadRpc, platform: Pubkey, quote: Pubkey) -> (Pubkey, PoolState) {
    let pool_id = Pubkey::new_from_array([40; 32]);
    let state = PoolState {
        base_mint: Pubkey::new_from_array([41; 32]),
        quote_mint: quote,
        global_config: Pubkey::new_from_array([42; 32]),
        platform_config: platform,
        creator: Pubkey::new_from_array([43; 32]),
        base_vault: Pubkey::new_from_array([44; 32]),
        quote_vault: Pubkey::new_from_array([45; 32]),
        virtual_base: 1_000_000,
        virtual_quote: 2_000_000,
        real_base: 3_000,
        real_quote: 4_000,
        total_base_sell: 500_000,
        ..Default::default()
    };
    // LaunchLab exposes a narrow offset-based fee decoder, not full config codecs.
    // Pin that current vendor contract and check it before feeding the RPC loader.
    let mut global = vec![0; 35];
    global[..8].copy_from_slice(&[149, 8, 156, 202, 160, 252, 176, 217]);
    global[27..35].copy_from_slice(&3100u64.to_le_bytes());
    let mut platform_data = vec![0; 728];
    platform_data[..8].copy_from_slice(&[160, 78, 128, 0, 248, 83, 230, 160]);
    platform_data[104..112].copy_from_slice(&700u64.to_le_bytes());
    platform_data[720..728].copy_from_slice(&200u64.to_le_bytes());
    let fees = bonk::decode_fee_config(&global, &platform_data).unwrap();
    assert_eq!(fees.curve_type, 0);
    assert_eq!(fees.trade_fee_rate, 3100);
    assert_eq!(fees.platform_fee_rate, 700);
    assert_eq!(fees.creator_fee_rate, 200);
    fixture.account(pool_id, account(accounts::BONK, pool_bytes(&state)));
    fixture.account(state.global_config, account(accounts::BONK, global));
    fixture.account(
        state.platform_config,
        account(accounts::BONK, platform_data),
    );
    fixture.account(
        state.base_mint,
        account(spl_token_interface::ID, cpmm::mint(6)),
    );
    fixture.account(
        state.quote_mint,
        account(spl_token_interface::ID, cpmm::mint(9)),
    );
    fixture.epoch(17);
    (pool_id, state)
}

fn assert_curve(params: &StonkFunParams, id: Pubkey, state: &PoolState) {
    assert_eq!(params.pool_state, id);
    assert_eq!(params.base_vault, state.base_vault);
    assert_eq!(params.quote_vault, state.quote_vault);
    assert_eq!(params.mint_token_program, spl_token_interface::ID);
    assert_eq!(params.quote_token_program, spl_token_interface::ID);
    assert_eq!(params.quote_mint, state.quote_mint);
    assert_eq!(params.global_config, state.global_config);
    assert_eq!(params.platform_config, state.platform_config);
    assert_eq!(params.virtual_base, state.virtual_base as u128);
    assert_eq!(params.virtual_quote, state.virtual_quote as u128);
    assert_eq!(params.real_base, state.real_base as u128);
    assert_eq!(params.real_quote, state.real_quote as u128);
    assert_eq!(params.total_base_sell, state.total_base_sell as u128);
    assert_eq!(
        (
            params.curve_type,
            params.trade_fee_rate,
            params.platform_fee_rate,
            params.creator_fee_rate
        ),
        (0, 3100, 700, 200)
    );
    assert_eq!(
        params.platform_associated_account,
        bonk::get_platform_associated_account_for_quote(&state.platform_config, &state.quote_mint)
            .unwrap()
    );
    assert_eq!(
        params.creator_associated_account,
        bonk::get_creator_associated_account_for_quote(&state.creator, &state.quote_mint).unwrap()
    );
    assert_eq!(params.base_transfer_fee.basis_points, 0);
    assert_eq!(params.quote_transfer_fee.basis_points, 0);
}

#[tokio::test]
async fn stonkfun_real_curve_loader_accepts_both_platforms_and_retains_fees() {
    for platform in [
        accounts::STONKFUN_STANDARD_PLATFORM_CONFIG,
        accounts::STONKFUN_REWARD_PLATFORM_CONFIG,
    ] {
        let mut fixture = ReadRpc::default();
        let (id, state) = curve(&mut fixture, platform, Pubkey::new_from_array([46; 32]));
        let rpc = fixture.client();
        warmup_on_rpc(
            &rpc,
            Pool::StonkFun,
            &state.base_mint,
            &state.quote_mint,
            Some(&id),
        )
        .await
        .unwrap();
        let params = dex_params(
            &rpc,
            Pool::StonkFun,
            &state.base_mint,
            &state.quote_mint,
            Some(&id),
            None,
            None,
        )
        .await
        .unwrap();
        let DexParamEnum::StonkFun(params) = params else {
            panic!("wrong direct curve variant")
        };
        assert_curve(&params, id, &state);
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 14);
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|(method, params)| *method == RpcRequest::GetAccountInfo
                    && params[0] == id.to_string())
                .count(),
            4
        );
    }
}

#[tokio::test]
async fn stonkfun_curve_via_sol_uses_real_target_then_explicit_cpmm_hop() {
    let (mut fixture, hop_id, hop, _) = cpmm::fixture();
    let (id, state) = curve(
        &mut fixture,
        accounts::STONKFUN_STANDARD_PLATFORM_CONFIG,
        hop.token0_mint,
    );
    let params = dex_params(
        &fixture.client(),
        Pool::StonkFun,
        &state.base_mint,
        &state.quote_mint,
        Some(&id),
        Some(Pool::RaydiumCpmm),
        Some(&hop_id),
    )
    .await
    .unwrap();
    let DexParamEnum::StonkFunViaSol(params) = params else {
        panic!("wrong SOL curve variant")
    };
    let StonkFunMemeLeg::Curve(target) = params.meme_leg else {
        panic!("wrong target leg")
    };
    assert_curve(&target, id, &state);
    let StonkFunSolHop::RaydiumCpmm(params) = params.sol_hop else {
        panic!("wrong hop leg")
    };
    assert_eq!(params.pool_state, hop_id);
    assert_eq!(params.base_mint, state.quote_mint);
    assert_eq!(params.quote_mint.to_string(), WSOL_MINT);
    assert_eq!(
        (params.base_reserve, params.quote_reserve),
        (1_000_000, 2_000_000)
    );
    let calls = fixture.calls();
    assert_eq!(calls.len(), 13);
    assert_eq!(calls[0].1[0], id.to_string());
    assert_eq!(calls[7].1[0], hop_id.to_string());
    fixture.assert_read_only();
}

#[tokio::test]
async fn stonkfun_reversed_curve_pair_fails_before_fee_reads_or_sol_hop() {
    let mut fixture = ReadRpc::default();
    let (id, state) = curve(
        &mut fixture,
        accounts::STONKFUN_STANDARD_PLATFORM_CONFIG,
        Pubkey::new_from_array([46; 32]),
    );
    let error = dex_params(
        &fixture.client(),
        Pool::StonkFun,
        &state.quote_mint,
        &state.base_mint,
        Some(&id),
        Some(Pool::RaydiumCpmm),
        Some(&Pubkey::new_from_array([99; 32])),
    )
    .await
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("does not contain requested mint"));
    assert_eq!(fixture.calls().len(), 1);
    fixture.assert_read_only();
}
