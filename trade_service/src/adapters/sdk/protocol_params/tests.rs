use super::*;
use base64::Engine;
use sol_trade_sdk::instruction::utils::{
    bonk::accounts,
    bonk_types::{pool_state_decode, PoolState, POOL_STATE_DISCRIMINATOR, POOL_STATE_SIZE},
};
use solana_client::rpc_request::RpcRequest;
use std::collections::HashMap;

mod amm_v4;
mod bonk;
mod cpmm;
mod damm_v2;
mod dbc;
mod discovery;
mod pump;
mod pump_swap;
mod read_rpc;
mod shared;
mod stonkfun;

fn state() -> PoolState {
    PoolState {
        base_mint: Pubkey::default(),
        quote_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
        ..Default::default()
    }
}

fn pool_bytes(state: &PoolState) -> Vec<u8> {
    // This fixed-width synthetic layout is checked against the SDK's Borsh decoder.
    let encoded = bincode::serialize(state).unwrap();
    assert_eq!(encoded.len(), POOL_STATE_SIZE);
    assert_eq!(pool_state_decode(&encoded).unwrap(), *state);
    let mut data = POOL_STATE_DISCRIMINATOR.to_vec();
    data.extend(encoded);
    data
}

fn rpc(data: Vec<u8>, owner: Pubkey) -> RpcClient {
    let size = data.len();
    RpcClient::new_mock_with_mocks(
        "fails".into(),
        HashMap::from([(
            RpcRequest::GetAccountInfo,
            serde_json::json!({
                "context":{"slot":42},"value":{"lamports":2039280,"owner":owner.to_string(),"data":[base64::engine::general_purpose::STANDARD.encode(data),"base64"],"executable":false,"rentEpoch":0,"space":size}
            }),
        )]),
    )
}

#[tokio::test]
async fn missing_pool_ids_keep_each_protocols_local_error_before_rpc() {
    let rpc = RpcClient::new_mock("fails".into());
    let mint = Pubkey::default();
    let quote = Pubkey::from_str(WSOL_MINT).unwrap();
    for (pool, name) in [
        (Pool::StonkFun, "stonkfun"),
        (Pool::StonkFunSwap, "stonkfun-swap"),
        (Pool::MeteoraDammV2, "meteora-damm-v2"),
        (Pool::MeteoraDbc, "meteora-dbc"),
        (Pool::RaydiumAmmV4, "raydium-amm-v4"),
        (Pool::RaydiumCpmm, "raydium-cpmm"),
    ] {
        let expected = format!("pool_id is required for {name}");
        assert_eq!(
            warmup_on_rpc(&rpc, pool, &mint, &quote, None)
                .await
                .unwrap_err()
                .to_string(),
            expected
        );
        assert_eq!(
            dex_params(&rpc, pool, &mint, &quote, None, None, None)
                .await
                .err()
                .unwrap()
                .to_string(),
            expected
        );
    }
}

#[tokio::test]
async fn unsupported_sol_hop_kind_is_rejected_before_rpc() {
    let rpc = RpcClient::new_mock("fails".into());
    let pool_id = Pubkey::new_from_array([9; 32]);
    let quote = Pubkey::default();
    for pool in [None, Some(Pool::PumpAmm), Some(Pool::StonkFun)] {
        assert_eq!(
            load_stonkfun_sol_hop(&rpc, pool, &pool_id, &quote)
                .await
                .err()
                .unwrap()
                .to_string(),
            "stonkfun Via-SOL only supports a Raydium CPMM SOL hop"
        );
    }
}

#[test]
fn synthetic_launchlab_layout_uses_current_sdk_decoder() {
    assert_eq!(pool_bytes(&state()).len(), 8 + POOL_STATE_SIZE);
}

#[tokio::test]
async fn launchlab_owner_discriminator_and_short_layout_fail_closed() {
    let state = state();
    let valid = pool_bytes(&state);
    let mut bad_discriminator = valid.clone();
    bad_discriminator[0] ^= 1;
    for (data, owner, message) in [
        (
            valid.clone(),
            Pubkey::default(),
            "Account is not owned by the LaunchLab program".to_string(),
        ),
        (
            bad_discriminator,
            accounts::BONK,
            "Account discriminator is not LaunchLab PoolState".to_string(),
        ),
        (
            valid[..8].to_vec(),
            accounts::BONK,
            format!(
                "LaunchLab pool account is too short: expected at least {} bytes, got 8",
                8 + POOL_STATE_SIZE
            ),
        ),
    ] {
        let rpc = rpc(data, owner);
        assert_eq!(
            load_stonkfun_params(
                &rpc,
                &Pubkey::new_from_array([9; 32]),
                &state.base_mint,
                &state.quote_mint
            )
            .await
            .err()
            .unwrap()
            .to_string(),
            message
        );
    }
}

#[tokio::test]
async fn stonkfun_mint_and_quote_checks_precede_platform_authorization() {
    let state = state();
    let absent = Pubkey::new_from_array([7; 32]);
    let pool = Pubkey::new_from_array([9; 32]);
    for (mint, quote, message) in [
        (
            absent,
            state.quote_mint,
            format!(
                "stonkfun pool does not contain requested mint {absent}; pool mints: {}",
                state.base_mint
            ),
        ),
        (
            state.base_mint,
            absent,
            format!(
                "stonkfun pool does not contain requested quote_mint {absent}; pool mints: {}",
                state.quote_mint
            ),
        ),
    ] {
        let rpc = rpc(pool_bytes(&state), accounts::BONK);
        assert_eq!(
            load_stonkfun_params(&rpc, &pool, &mint, &quote)
                .await
                .err()
                .unwrap()
                .to_string(),
            message
        );
    }
}

#[test]
fn stonkfun_accepts_only_standard_or_reward_platform() {
    let pool = Pubkey::new_from_array([9; 32]);
    for platform in [
        accounts::STONKFUN_STANDARD_PLATFORM_CONFIG,
        accounts::STONKFUN_REWARD_PLATFORM_CONFIG,
    ] {
        assert!(ensure_stonkfun_platform(&pool, &platform).is_ok());
    }
    let platform = Pubkey::default();
    assert_eq!(
        ensure_stonkfun_platform(&pool, &platform)
            .unwrap_err()
            .to_string(),
        format!("launchlab pool {pool} is not a recognized StonkFun platform: {platform}")
    );
}

#[tokio::test]
async fn unrecognized_launchlab_platform_is_not_a_stonkfun_pool() {
    let state = state();
    let pool = Pubkey::new_from_array([9; 32]);
    let rpc = rpc(pool_bytes(&state), accounts::BONK);
    assert_eq!(
        load_stonkfun_params(&rpc, &pool, &state.base_mint, &state.quote_mint)
            .await
            .err()
            .unwrap()
            .to_string(),
        format!(
            "launchlab pool {pool} is not a recognized StonkFun platform: {}",
            state.platform_config
        )
    );
}
