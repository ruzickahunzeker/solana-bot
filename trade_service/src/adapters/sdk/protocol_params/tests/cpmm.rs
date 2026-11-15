use super::{super::*, read_rpc::*};
use sol_trade_sdk::instruction::utils::{
    raydium_cpmm::accounts::RAYDIUM_CPMM,
    raydium_cpmm_types::{
        self, AmmConfig, PoolState, AMM_CONFIG_DISCRIMINATOR, AMM_CONFIG_SIZE,
        POOL_STATE_DISCRIMINATOR, POOL_STATE_SIZE,
    },
};
use solana_client::rpc_request::RpcRequest;
use solana_program_pack::Pack;
use spl_token_interface::state::{Account as TokenAccount, AccountState, Mint};

fn address(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

pub(super) fn mint(decimals: u8) -> Vec<u8> {
    let mut data = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            decimals,
            supply: 1_000_000_000,
            is_initialized: true,
            ..Default::default()
        },
        &mut data,
    )
    .unwrap();
    data
}

pub(super) fn vault(mint: Pubkey, amount: u64) -> Vec<u8> {
    let mut data = vec![0; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner: address(15),
            amount,
            state: AccountState::Initialized,
            ..Default::default()
        },
        &mut data,
    )
    .unwrap();
    data
}

fn pool_account(pool: &PoolState) -> solana_sdk::account::Account {
    let bytes = bincode::serialize(pool).unwrap();
    assert_eq!(bytes.len(), POOL_STATE_SIZE);
    assert_eq!(
        raydium_cpmm_types::pool_state_decode(&bytes).unwrap(),
        *pool
    );
    let mut data = POOL_STATE_DISCRIMINATOR.to_vec();
    data.extend(bytes);
    account(RAYDIUM_CPMM, data)
}

pub(super) fn fixture() -> (ReadRpc, Pubkey, PoolState, AmmConfig) {
    let pool_id = address(1);
    let pool = PoolState {
        amm_config: address(2),
        pool_creator: address(3),
        token0_vault: address(4),
        token1_vault: address(5),
        lp_mint: address(6),
        token0_mint: address(7),
        token1_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
        token0_program: spl_token_interface::ID,
        token1_program: spl_token_interface::ID,
        observation_key: address(8),
        mint0_decimals: 6,
        mint1_decimals: 9,
        protocol_fees_token0: 11,
        fund_fees_token0: 13,
        creator_fees_token0: 17,
        protocol_fees_token1: 19,
        fund_fees_token1: 23,
        creator_fees_token1: 29,
        creator_fee_on: 2,
        enable_creator_fee: true,
        ..Default::default()
    };
    let config = AmmConfig {
        trade_fee_rate: 3100,
        protocol_fee_rate: 120_001,
        fund_fee_rate: 40_002,
        creator_fee_rate: 800,
        ..Default::default()
    };
    let bytes = bincode::serialize(&config).unwrap();
    assert_eq!(bytes.len(), AMM_CONFIG_SIZE);
    assert_eq!(
        raydium_cpmm_types::amm_config_decode(&bytes).unwrap(),
        config
    );
    let mut config_data = AMM_CONFIG_DISCRIMINATOR.to_vec();
    config_data.extend(bytes);
    let mut fixture = ReadRpc::default();
    fixture.account(pool_id, pool_account(&pool));
    fixture.account(pool.amm_config, account(RAYDIUM_CPMM, config_data));
    fixture.account(pool.token0_mint, account(pool.token0_program, mint(6)));
    fixture.account(pool.token1_mint, account(pool.token1_program, mint(9)));
    fixture.account(
        pool.token0_vault,
        account(pool.token0_program, vault(pool.token0_mint, 1_000_041)),
    );
    fixture.account(
        pool.token1_vault,
        account(pool.token1_program, vault(pool.token1_mint, 2_000_071)),
    );
    fixture.epoch(17);
    (fixture, pool_id, pool, config)
}

fn assert_params(params: &RaydiumCpmmParams, id: Pubkey, pool: &PoolState, config: &AmmConfig) {
    assert_eq!(params.pool_state, id);
    assert_eq!(params.amm_config, pool.amm_config);
    assert_eq!(params.base_mint, pool.token0_mint);
    assert_eq!(params.quote_mint, pool.token1_mint);
    assert_eq!(params.base_vault, pool.token0_vault);
    assert_eq!(params.quote_vault, pool.token1_vault);
    assert_eq!(params.base_token_program, pool.token0_program);
    assert_eq!(params.quote_token_program, pool.token1_program);
    assert_eq!(params.observation_state, pool.observation_key);
    assert_eq!(
        (params.base_reserve, params.quote_reserve),
        (1_000_000, 2_000_000)
    );
    assert_eq!(params.trade_fee_rate, config.trade_fee_rate);
    assert_eq!(params.protocol_fee_rate, config.protocol_fee_rate);
    assert_eq!(params.fund_fee_rate, config.fund_fee_rate);
    assert_eq!(params.creator_fee_rate, config.creator_fee_rate);
    assert_eq!(params.creator_fee_on, pool.creator_fee_on);
    assert_eq!(params.enable_creator_fee, pool.enable_creator_fee);
    assert_eq!(params.base_transfer_fee.basis_points, 0);
    assert_eq!(params.base_transfer_fee.maximum_fee, 0);
    assert_eq!(params.quote_transfer_fee.basis_points, 0);
    assert_eq!(params.quote_transfer_fee.maximum_fee, 0);
}

#[tokio::test]
async fn cpmm_and_stonkfun_swap_real_loaders_preserve_reserves_fees_and_orientation() {
    for kind in [Pool::RaydiumCpmm, Pool::StonkFunSwap] {
        let (fixture, id, pool, config) = fixture();
        let rpc = fixture.client();
        warmup_on_rpc(&rpc, kind, &pool.token0_mint, &pool.token1_mint, Some(&id))
            .await
            .unwrap();
        for (mint, quote) in [
            (pool.token0_mint, pool.token1_mint),
            (pool.token1_mint, pool.token0_mint),
        ] {
            let params = dex_params(&rpc, kind, &mint, &quote, Some(&id), None, None)
                .await
                .unwrap();
            match (kind, params) {
                (Pool::RaydiumCpmm, DexParamEnum::RaydiumCpmm(params))
                | (Pool::StonkFunSwap, DexParamEnum::StonkFunSwap(params)) => {
                    assert_params(&params, id, &pool, &config);
                }
                _ => panic!("wrong SDK parameter variant"),
            }
        }
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 18);
        let vault_reads: Vec<_> = fixture
            .calls()
            .into_iter()
            .filter(|(method, _)| *method == RpcRequest::GetMultipleAccounts)
            .collect();
        assert_eq!(vault_reads.len(), 3);
        for (_, params) in vault_reads {
            assert_eq!(
                params[0],
                serde_json::json!([pool.token0_vault.to_string(), pool.token1_vault.to_string()])
            );
        }
    }
}

#[tokio::test]
async fn cpmm_warmup_quote_only_differs_from_build_and_stonkfun_swap() {
    for kind in [Pool::RaydiumCpmm, Pool::StonkFunSwap] {
        let (fixture, id, pool, _) = fixture();
        let rpc = fixture.client();
        let alien = address(99);
        let warmup = warmup_on_rpc(&rpc, kind, &alien, &pool.token1_mint, Some(&id)).await;
        assert_eq!(warmup.is_ok(), kind == Pool::RaydiumCpmm);
        let error = dex_params(&rpc, kind, &alien, &pool.token1_mint, Some(&id), None, None)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("does not contain requested mint"));
        fixture.assert_read_only();
    }
}

#[tokio::test]
async fn cpmm_fee_underflow_and_mint_owner_mismatch_fail_before_epoch() {
    for underflow in [true, false] {
        let (mut fixture, id, mut pool, _) = fixture();
        if underflow {
            pool.protocol_fees_token0 = 1_000_042;
            fixture.account(id, pool_account(&pool));
        } else {
            fixture.account(pool.token0_mint, account(address(98), mint(6)));
        }
        let error = dex_params(
            &fixture.client(),
            Pool::RaydiumCpmm,
            &pool.token0_mint,
            &pool.token1_mint,
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
            if underflow {
                "Raydium CPMM token0 fees exceed vault balance"
            } else {
                "Raydium CPMM mint owner does not match PoolState token program"
            }
        );
        fixture.assert_read_only();
        assert!(!fixture
            .calls()
            .iter()
            .any(|(method, _)| *method == RpcRequest::GetEpochInfo));
    }
}

fn extension_mint(active_hook: bool) -> Vec<u8> {
    use spl_token_2022_interface::{
        extension::{
            transfer_fee::{TransferFee, TransferFeeConfig},
            transfer_hook::TransferHook,
            BaseStateWithExtensionsMut, ExtensionType, StateWithExtensionsMut,
        },
        state::Mint,
    };
    let extensions = [
        ExtensionType::TransferFeeConfig,
        ExtensionType::TransferHook,
    ];
    let mut data = vec![0; ExtensionType::try_calculate_account_len::<Mint>(&extensions).unwrap()];
    let mut state = StateWithExtensionsMut::<Mint>::unpack_uninitialized(&mut data).unwrap();
    let fee = state.init_extension::<TransferFeeConfig>(false).unwrap();
    fee.older_transfer_fee = TransferFee {
        epoch: 10u64.into(),
        maximum_fee: 5u64.into(),
        transfer_fee_basis_points: 100u16.into(),
    };
    fee.newer_transfer_fee = TransferFee {
        epoch: 20u64.into(),
        maximum_fee: 9u64.into(),
        transfer_fee_basis_points: 300u16.into(),
    };
    let hook = state.init_extension::<TransferHook>(false).unwrap();
    if active_hook {
        hook.program_id = Some(address(97)).try_into().unwrap();
    }
    state.base = Mint {
        decimals: 6,
        supply: 1_000_000_000,
        is_initialized: true,
        ..Default::default()
    };
    state.pack_base();
    state.init_account_type().unwrap();
    data
}

#[tokio::test]
async fn cpmm_token2022_fee_uses_rpc_epoch_and_inactive_hook_does_not_reject() {
    for (epoch, basis_points, maximum_fee) in [(17, 100, 5), (20, 300, 9)] {
        let (mut fixture, id, mut pool, _) = fixture();
        pool.token0_program = spl_token_2022_interface::ID;
        fixture.account(id, pool_account(&pool));
        fixture.account(
            pool.token0_mint,
            account(pool.token0_program, extension_mint(false)),
        );
        fixture.account(
            pool.token0_vault,
            account(pool.token0_program, vault(pool.token0_mint, 1_000_041)),
        );
        fixture.epoch(epoch);
        let params = dex_params(
            &fixture.client(),
            Pool::RaydiumCpmm,
            &pool.token0_mint,
            &pool.token1_mint,
            Some(&id),
            None,
            None,
        )
        .await
        .unwrap();
        let DexParamEnum::RaydiumCpmm(params) = params else {
            panic!("wrong CPMM variant")
        };
        assert_eq!(params.base_token_program, spl_token_2022_interface::ID);
        assert_eq!(params.base_transfer_fee.basis_points, basis_points);
        assert_eq!(params.base_transfer_fee.maximum_fee, maximum_fee);
        assert_eq!(params.base_transfer_fee.calculate(1), 1);
        assert_eq!(params.base_transfer_fee.calculate(100_000), maximum_fee);
        assert_eq!(params.quote_transfer_fee.basis_points, 0);
        assert_eq!(
            (params.base_reserve, params.quote_reserve),
            (1_000_000, 2_000_000)
        );
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 6);
    }
}

#[tokio::test]
async fn cpmm_active_transfer_hook_is_rejected_for_both_warmup_and_build() {
    let (mut fixture, id, mut pool, _) = fixture();
    pool.token0_program = spl_token_2022_interface::ID;
    fixture.account(id, pool_account(&pool));
    fixture.account(
        pool.token0_mint,
        account(pool.token0_program, extension_mint(true)),
    );
    fixture.account(
        pool.token0_vault,
        account(pool.token0_program, vault(pool.token0_mint, 1_000_041)),
    );
    let rpc = fixture.client();
    assert_eq!(
        warmup_on_rpc(
            &rpc,
            Pool::RaydiumCpmm,
            &pool.token0_mint,
            &pool.token1_mint,
            Some(&id)
        )
        .await
        .unwrap_err()
        .to_string(),
        "Active transfer hook requires unsupported extra accounts"
    );
    assert_eq!(
        dex_params(
            &rpc,
            Pool::RaydiumCpmm,
            &pool.token0_mint,
            &pool.token1_mint,
            Some(&id),
            None,
            None
        )
        .await
        .err()
        .unwrap()
        .to_string(),
        "Active transfer hook requires unsupported extra accounts"
    );
    fixture.assert_read_only();
    assert_eq!(fixture.calls().len(), 12);
}

#[tokio::test]
async fn cpmm_pool_and_fee_config_headers_fail_closed() {
    for config in [false, true] {
        for damage in 0..3 {
            let (mut fixture, id, pool, fees) = fixture();
            let mut invalid = if config {
                let mut data = AMM_CONFIG_DISCRIMINATOR.to_vec();
                data.extend(bincode::serialize(&fees).unwrap());
                account(RAYDIUM_CPMM, data)
            } else {
                pool_account(&pool)
            };
            match damage {
                0 => invalid.owner = Pubkey::default(),
                1 => invalid.data[0] ^= 1,
                _ => {
                    invalid.data.pop();
                }
            }
            fixture.account(if config { pool.amm_config } else { id }, invalid);
            let error = dex_params(
                &fixture.client(),
                Pool::RaydiumCpmm,
                &pool.token0_mint,
                &pool.token1_mint,
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
                match (config, damage) {
                    (true, 0) => "Account is not owned by Raydium CPMM program".into(),
                    (false, 0) => "Account is not owned by Raydium Cpmm program".into(),
                    (true, 1) => "Account discriminator is not Raydium CPMM AmmConfig".into(),
                    (false, 1) => "Account discriminator is not Raydium CPMM PoolState".into(),
                    (true, _) => format!(
                        "Raydium CPMM config account is too short: expected at least {}, got {}",
                        8 + AMM_CONFIG_SIZE,
                        7 + AMM_CONFIG_SIZE
                    ),
                    (false, _) => format!(
                        "Raydium CPMM pool account is too short: expected at least {}, got {}",
                        8 + POOL_STATE_SIZE,
                        7 + POOL_STATE_SIZE
                    ),
                }
            );
            fixture.assert_read_only();
            assert_eq!(fixture.calls().len(), if config { 2 } else { 1 });
        }
    }
}
