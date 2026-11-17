use super::{super::*, read_rpc::*};
use sol_trade_sdk::instruction::utils::{
    raydium_amm_v4::accounts::RAYDIUM_AMM_V4,
    raydium_amm_v4_types::{
        amm_info_decode, market_state_decode, AmmInfo, Fees, MarketState, OutPutData,
        AMM_INFO_SIZE, MARKET_STATE_SIZE,
    },
};

fn address(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

fn pool_account(pool: &AmmInfo) -> solana_sdk::account::Account {
    let bytes = bincode::serialize(pool).unwrap();
    assert_eq!(bytes.len(), AMM_INFO_SIZE);
    assert_eq!(amm_info_decode(&bytes).unwrap(), *pool);
    account(RAYDIUM_AMM_V4, bytes)
}

pub(super) fn fixture() -> (ReadRpc, Pubkey, AmmInfo, MarketState, Pubkey) {
    let id = address(71);
    let pool = AmmInfo {
        status: 6,
        coin_mint: address(72),
        pc_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
        token_coin: address(73),
        token_pc: address(74),
        open_orders: address(75),
        target_orders: address(76),
        serum_dex: address(77),
        market: address(78),
        coin_decimals: 6,
        pc_decimals: 9,
        out_put: OutPutData {
            need_take_pnl_coin: 31,
            need_take_pnl_pc: 37,
            ..Default::default()
        },
        fees: Fees {
            swap_fee_numerator: 39,
            swap_fee_denominator: 10_001,
            ..Default::default()
        },
        ..Default::default()
    };
    let (nonce, signer) = (0u64..256)
        .find_map(|nonce| {
            Pubkey::create_program_address(
                &[pool.market.as_ref(), &nonce.to_le_bytes()],
                &pool.serum_dex,
            )
            .ok()
            .map(|signer| (nonce, signer))
        })
        .unwrap();
    let market = MarketState {
        own_address: pool.market,
        vault_signer_nonce: nonce,
        coin_mint: pool.coin_mint,
        pc_mint: pool.pc_mint,
        serum_bids: address(79),
        serum_asks: address(80),
        serum_event_queue: address(81),
        serum_coin_vault_account: address(82),
        serum_pc_vault_account: address(83),
        ..Default::default()
    };
    let bytes = bincode::serialize(&market).unwrap();
    assert_eq!(bytes.len(), MARKET_STATE_SIZE);
    assert_eq!(market_state_decode(&bytes).unwrap(), market);
    let mut fixture = ReadRpc::default();
    fixture.account(id, pool_account(&pool));
    fixture.account(pool.market, account(pool.serum_dex, bytes));
    fixture.account(
        pool.token_coin,
        account(
            spl_token_interface::ID,
            super::cpmm::vault(pool.coin_mint, 1_000_031),
        ),
    );
    fixture.account(
        pool.token_pc,
        account(
            spl_token_interface::ID,
            super::cpmm::vault(pool.pc_mint, 2_000_037),
        ),
    );
    (fixture, id, pool, market, signer)
}

#[tokio::test]
async fn amm_real_loader_preserves_market_accounts_pnl_reserves_and_fees() {
    let (fixture, id, pool, market, signer) = fixture();
    let rpc = fixture.client();
    warmup_on_rpc(
        &rpc,
        Pool::RaydiumAmmV4,
        &pool.coin_mint,
        &pool.pc_mint,
        Some(&id),
    )
    .await
    .unwrap();
    for (mint, quote) in [
        (pool.coin_mint, pool.pc_mint),
        (pool.pc_mint, pool.coin_mint),
    ] {
        let params = dex_params(
            &rpc,
            Pool::RaydiumAmmV4,
            &mint,
            &quote,
            Some(&id),
            None,
            None,
        )
        .await
        .unwrap();
        let DexParamEnum::RaydiumAmmV4(params) = params else {
            panic!("wrong AMM variant")
        };
        assert_eq!(params.amm, id);
        assert_eq!(params.coin_mint, pool.coin_mint);
        assert_eq!(params.pc_mint, pool.pc_mint);
        assert_eq!(params.token_coin, pool.token_coin);
        assert_eq!(params.token_pc, pool.token_pc);
        assert_eq!(params.amm_open_orders, pool.open_orders);
        assert_eq!(params.amm_target_orders, pool.target_orders);
        assert_eq!(params.serum_program, pool.serum_dex);
        assert_eq!(params.serum_market, pool.market);
        assert_eq!(params.serum_bids, market.serum_bids);
        assert_eq!(params.serum_asks, market.serum_asks);
        assert_eq!(params.serum_event_queue, market.serum_event_queue);
        assert_eq!(
            params.serum_coin_vault_account,
            market.serum_coin_vault_account
        );
        assert_eq!(params.serum_pc_vault_account, market.serum_pc_vault_account);
        assert_eq!(params.serum_vault_signer, signer);
        assert_eq!(
            (params.coin_reserve, params.pc_reserve),
            (1_000_000, 2_000_000)
        );
        assert_eq!(
            (params.swap_fee_numerator, params.swap_fee_denominator),
            (39, 10_001)
        );
    }
    fixture.assert_read_only();
    assert_eq!(fixture.calls().len(), 9);
}

#[tokio::test]
async fn amm_warmup_quote_only_does_not_expand_build_membership() {
    let (fixture, id, pool, _, _) = fixture();
    let rpc = fixture.client();
    let alien = address(99);
    warmup_on_rpc(&rpc, Pool::RaydiumAmmV4, &alien, &pool.pc_mint, Some(&id))
        .await
        .unwrap();
    assert!(dex_params(
        &rpc,
        Pool::RaydiumAmmV4,
        &alien,
        &pool.pc_mint,
        Some(&id),
        None,
        None
    )
    .await
    .err()
    .unwrap()
    .to_string()
    .contains("does not contain requested mint"));
    fixture.assert_read_only();
    assert_eq!(fixture.calls().len(), 6);
}

#[tokio::test]
async fn amm_pnl_underflow_is_rejected_for_either_vault() {
    for coin in [true, false] {
        let (mut fixture, id, mut pool, _, _) = fixture();
        if coin {
            pool.out_put.need_take_pnl_coin = 1_000_032
        } else {
            pool.out_put.need_take_pnl_pc = 2_000_038
        };
        fixture.account(id, pool_account(&pool));
        let error = dex_params(
            &fixture.client(),
            Pool::RaydiumAmmV4,
            &pool.coin_mint,
            &pool.pc_mint,
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
            if coin {
                "AMM coin PnL exceeds vault balance"
            } else {
                "AMM pc PnL exceeds vault balance"
            }
        );
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 3);
    }
}

#[tokio::test]
async fn amm_short_pool_and_market_layouts_stop_before_vault_reads() {
    for market in [false, true] {
        let (mut fixture, id, pool, state, _) = fixture();
        let (address, owner, mut data) = if market {
            (
                pool.market,
                pool.serum_dex,
                bincode::serialize(&state).unwrap(),
            )
        } else {
            (id, RAYDIUM_AMM_V4, bincode::serialize(&pool).unwrap())
        };
        data.pop();
        fixture.account(address, account(owner, data));
        let error = dex_params(
            &fixture.client(),
            Pool::RaydiumAmmV4,
            &pool.coin_mint,
            &pool.pc_mint,
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
            if market {
                "Failed to decode market state"
            } else {
                "Failed to decode amm info"
            }
        );
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), if market { 2 } else { 1 });
    }
}
