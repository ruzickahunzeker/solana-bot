use super::{super::*, read_rpc::*};
use sol_trade_sdk::instruction::utils::{
    pumpswap::{self, accounts, PumpSwapFeeBasisPoints},
    pumpswap_types::{pool_decode, Pool as SwapPool, POOL_DISCRIMINATOR, POOL_SIZE},
};
use solana_client::rpc_request::RpcRequest;

fn address(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

fn pool_account(pool: &SwapPool) -> solana_sdk::account::Account {
    // Exclude the two Borsh-skipped projected fields from the serde tuple.
    let bytes = bincode::serialize(&(
        pool.pool_bump,
        pool.index,
        pool.creator,
        pool.base_mint,
        pool.quote_mint,
        pool.lp_mint,
        pool.pool_base_token_account,
        pool.pool_quote_token_account,
        pool.lp_supply,
        pool.coin_creator,
        pool.is_mayhem_mode,
        pool.is_cashback_coin,
        pool.virtual_quote_reserves,
        pool.creator_fee_bps,
        pool.can_edit_creator_fee,
        pool.is_holder_reward,
    ))
    .unwrap();
    assert_eq!(bytes.len(), POOL_SIZE);
    assert_eq!(pool_decode(&bytes).unwrap(), *pool);
    let mut data = POOL_DISCRIMINATOR.to_vec();
    data.extend(bytes);
    account(accounts::AMM_PROGRAM, data)
}

fn fixture(mayhem: bool, virtual_quote_reserves: i128) -> (ReadRpc, Pubkey, SwapPool) {
    let id = address(111);
    let base_mint = address(112);
    let pool = SwapPool {
        creator: pumpswap::get_pump_pool_authority_pda(&base_mint),
        base_mint,
        quote_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
        lp_mint: address(113),
        pool_base_token_account: address(114),
        pool_quote_token_account: address(115),
        lp_supply: 10_000,
        coin_creator: address(119),
        is_mayhem_mode: mayhem,
        is_cashback_coin: true,
        virtual_quote_reserves,
        ..Default::default()
    };
    let mut fixture = ReadRpc::default();
    fixture.account(id, pool_account(&pool));
    fixture.account(
        pool.base_mint,
        account(spl_token_2022_interface::ID, super::cpmm::mint(6)),
    );
    fixture.account(
        pool.pool_base_token_account,
        account(
            spl_token_2022_interface::ID,
            super::cpmm::vault(pool.base_mint, 1_000_000),
        ),
    );
    fixture.account(
        pool.pool_quote_token_account,
        account(
            spl_token_interface::ID,
            super::cpmm::vault(pool.quote_mint, 2_000_000),
        ),
    );
    // Fixed-width current GlobalConfig layout. Repeat one recipient in each set
    // so the SDK's random valid-recipient choice remains deterministic.
    let mut global = vec![149, 8, 156, 202, 160, 252, 176, 217];
    global.extend(
        bincode::serialize(&(
            address(120),
            9u64,
            11u64,
            0u8,
            [address(116); 8],
            13u64,
            address(120),
            Pubkey::default(),
            address(118),
            true,
            [address(118); 7],
            true,
            [address(117); 8],
            0u64,
            address(120),
            false,
        ))
        .unwrap(),
    );
    global.push(0); // creator_fee_configurable
    let decoded = pumpswap::decode_global_config(&global).unwrap();
    assert_eq!(
        (
            decoded.lp_fee_basis_points,
            decoded.protocol_fee_basis_points,
            decoded.coin_creator_fee_basis_points
        ),
        (9, 11, 13)
    );
    assert_eq!(decoded.protocol_fee_recipients, [address(116); 8]);
    assert_eq!(decoded.reserved_fee_recipient, address(118));
    assert_eq!(decoded.reserved_fee_recipients, [address(118); 7]);
    assert_eq!(decoded.buyback_fee_recipients, [address(117); 8]);
    assert!(!decoded.creator_fee_configurable);
    fixture.account(
        accounts::GLOBAL_ACCOUNT,
        account(accounts::AMM_PROGRAM, global),
    );
    let mut fees = vec![143, 52, 146, 187, 219, 123, 76, 155];
    fees.extend(
        bincode::serialize(&(
            0u8,
            address(120),
            2u64,
            3u64,
            5u64,
            1u32,
            0u128,
            31u64,
            37u64,
            41u64,
            0u32,
            0u64,
            0u64,
            0u64,
        ))
        .unwrap(),
    );
    let decoded = pumpswap::decode_fee_config(&fees).unwrap();
    assert_eq!(decoded.flat_fees, PumpSwapFeeBasisPoints::new(2, 3, 5));
    assert_eq!(decoded.fee_tiers.len(), 1);
    assert_eq!(decoded.fee_tiers[0].market_cap_lamports_threshold, 0);
    assert_eq!(
        decoded.fee_tiers[0].fees,
        PumpSwapFeeBasisPoints::new(31, 37, 41)
    );
    fixture.account(accounts::FEE_CONFIG, account(accounts::FEE_PROGRAM, fees));
    (fixture, id, pool)
}

#[tokio::test]
async fn pump_swap_real_loader_uses_coherent_snapshot_virtual_reserves_and_current_fees() {
    // All scenarios use identical configs because vendor config caches are process-global.
    for (mayhem, virtual_quote) in [(false, 1000), (true, -1000)] {
        let (fixture, id, pool) = fixture(mayhem, virtual_quote);
        let rpc = fixture.client();
        warmup_on_rpc(
            &rpc,
            Pool::PumpAmm,
            &pool.base_mint,
            &pool.quote_mint,
            Some(&id),
        )
        .await
        .unwrap();
        for (mint, quote) in [
            (pool.base_mint, pool.quote_mint),
            (pool.quote_mint, pool.base_mint),
        ] {
            let params = dex_params(&rpc, Pool::PumpAmm, &mint, &quote, Some(&id), None, None)
                .await
                .unwrap();
            let DexParamEnum::PumpSwap(params) = params else {
                panic!("wrong PumpSwap variant")
            };
            assert_eq!(params.pool, id);
            assert_eq!(params.base_mint, pool.base_mint);
            assert_eq!(params.quote_mint, pool.quote_mint);
            assert_eq!(params.pool_base_token_account, pool.pool_base_token_account);
            assert_eq!(
                params.pool_quote_token_account,
                pool.pool_quote_token_account
            );
            assert_eq!(
                (
                    params.pool_base_token_reserves,
                    params.pool_quote_token_reserves
                ),
                (1_000_000, 2_000_000)
            );
            assert_eq!(params.virtual_quote_reserves, virtual_quote);
            assert_eq!(
                params.effective_quote_reserves().unwrap(),
                (2_000_000i128 + virtual_quote) as u64
            );
            assert_eq!(params.base_mint_supply, Some(1_000_000_000));
            assert_eq!(params.base_token_program, spl_token_2022_interface::ID);
            assert_eq!(params.quote_token_program, spl_token_interface::ID);
            assert_eq!(params.pool_creator, pool.creator);
            assert_eq!(params.coin_creator, pool.coin_creator);
            assert_eq!(params.is_mayhem_mode, mayhem);
            assert!(params.is_cashback_coin);
            assert_eq!(params.cashback_fee_basis_points, 0);
            assert_eq!(
                params.fee_basis_points,
                PumpSwapFeeBasisPoints::new(31, 37, 41)
            );
            assert_eq!(
                params.protocol_fee_recipient_override,
                Some(address(if mayhem { 118 } else { 116 }))
            );
            assert_eq!(
                params.protocol_extra_fee_recipient_override,
                Some(address(117))
            );
            let authority = Pubkey::find_program_address(
                &[b"creator_vault", pool.coin_creator.as_ref()],
                &accounts::AMM_PROGRAM,
            )
            .0;
            assert_eq!(params.coin_creator_vault_authority, authority);
            assert_eq!(
                params.coin_creator_vault_ata,
                spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
                    &authority,
                    &pool.quote_mint,
                    &spl_token_interface::ID
                )
            );
            params.validate_accounts().unwrap();
        }
        fixture.assert_read_only();
        let calls = fixture.calls();
        let snapshots: Vec<_> = calls
            .iter()
            .filter(|(method, _)| *method == RpcRequest::GetMultipleAccounts)
            .collect();
        assert_eq!(snapshots.len(), 3);
        for (_, params) in snapshots {
            assert_eq!(
                params[0],
                serde_json::json!([
                    id.to_string(),
                    pool.pool_base_token_account.to_string(),
                    pool.pool_quote_token_account.to_string(),
                    pool.base_mint.to_string()
                ])
            );
        }
        // Six Pool/snapshot reads plus zero to six config reads (cached independently).
        assert!((6..=12).contains(&calls.len()));
    }
}

#[tokio::test]
async fn pump_swap_warmup_quote_only_does_not_expand_build_membership() {
    let (fixture, id, pool) = fixture(false, 1000);
    let rpc = fixture.client();
    let alien = address(99);
    warmup_on_rpc(&rpc, Pool::PumpAmm, &alien, &pool.quote_mint, Some(&id))
        .await
        .unwrap();
    assert!(dex_params(
        &rpc,
        Pool::PumpAmm,
        &alien,
        &pool.quote_mint,
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
}

#[tokio::test]
async fn pump_swap_invalid_vault_or_mint_snapshot_fails_before_fee_loading() {
    for damage in 0..4 {
        let (mut fixture, id, pool) = fixture(false, 1000);
        let (address, owner, mut data, message) = match damage {
            0 => (
                pool.pool_base_token_account,
                spl_token_2022_interface::ID,
                super::cpmm::vault(address(98), 1_000_000),
                "Pool vault mint does not match Pool account",
            ),
            1 => (
                pool.pool_base_token_account,
                Pubkey::default(),
                super::cpmm::vault(pool.base_mint, 1_000_000),
                "Pool vault is not owned by a supported token program",
            ),
            2 => (
                pool.base_mint,
                spl_token_interface::ID,
                super::cpmm::mint(6),
                "Base mint and base vault use different token programs",
            ),
            _ => (
                pool.pool_quote_token_account,
                spl_token_interface::ID,
                super::cpmm::vault(pool.quote_mint, 2_000_000),
                "Pool vault is not initialized",
            ),
        };
        if damage == 3 {
            data[108] = 0;
        }
        fixture.account(address, account(owner, data));
        let error = dex_params(
            &fixture.client(),
            Pool::PumpAmm,
            &pool.base_mint,
            &pool.quote_mint,
            Some(&id),
            None,
            None,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert_eq!(error, message);
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 2);
    }
}
