use super::{super::*, read_rpc::*};
use sol_trade_sdk::instruction::utils::bonk::{self, accounts};

#[tokio::test]
async fn bonk_real_loader_derives_the_existing_wsol_and_usd1_paths() {
    for quote in [
        Pubkey::from_str(WSOL_MINT).unwrap(),
        sol_trade_sdk::constants::USD1_TOKEN_ACCOUNT,
    ] {
        let mut fixture = ReadRpc::default();
        let (_, state) = super::stonkfun::curve(
            &mut fixture,
            accounts::STONKFUN_STANDARD_PLATFORM_CONFIG,
            quote,
        );
        let id = bonk::get_pool_pda(&state.base_mint, &quote).unwrap();
        fixture.account(id, account(accounts::BONK, super::pool_bytes(&state)));
        let rpc = fixture.client();
        warmup_on_rpc(&rpc, Pool::Bonk, &state.base_mint, &quote, None)
            .await
            .unwrap();
        let params = dex_params(&rpc, Pool::Bonk, &state.base_mint, &quote, None, None, None)
            .await
            .unwrap();
        let DexParamEnum::Bonk(params) = params else {
            panic!("wrong Bonk variant")
        };
        assert_eq!(params.pool_state, id);
        assert_eq!(params.quote_mint, quote);
        assert_eq!(params.base_vault, state.base_vault);
        assert_eq!(params.quote_vault, state.quote_vault);
        assert_eq!(params.global_config, state.global_config);
        assert_eq!(params.platform_config, state.platform_config);
        assert_eq!(params.mint_token_program, spl_token_interface::ID);
        assert_eq!(params.quote_token_program, spl_token_interface::ID);
        assert_eq!(
            (
                params.virtual_base,
                params.virtual_quote,
                params.real_base,
                params.real_quote
            ),
            (1_000_000, 2_000_000, 3_000, 4_000)
        );
        assert_eq!(
            (
                params.trade_fee_rate,
                params.platform_fee_rate,
                params.creator_fee_rate
            ),
            (3100, 700, 200)
        );
        fixture.assert_read_only();
        let calls = fixture.calls();
        assert_eq!(calls.len(), 12);
        assert_eq!(calls[0].1[0], id.to_string());
        assert_eq!(calls[6].1[0], id.to_string());
    }
}

#[tokio::test]
async fn bonk_missing_derived_pool_keeps_warmup_raw_and_build_descriptive_errors() {
    for (quote, label) in [
        (Pubkey::from_str(WSOL_MINT).unwrap(), "WSOL"),
        (sol_trade_sdk::constants::USD1_TOKEN_ACCOUNT, "USD1"),
    ] {
        let mint = Pubkey::new_from_array([91; 32]);
        let id = bonk::get_pool_pda(&mint, &quote).unwrap();
        let mut fixture = ReadRpc::default();
        fixture.absent(id);
        let rpc = fixture.client();
        let raw = warmup_on_rpc(&rpc, Pool::Bonk, &mint, &quote, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(raw.contains("AccountNotFound"));
        assert!(raw.contains(&id.to_string()));
        let error = dex_params(&rpc, Pool::Bonk, &mint, &quote, None, None, None)
            .await
            .err()
            .unwrap()
            .to_string();
        assert_eq!(error, format!("bonk {label} pool not found for mint {mint}; this mint may not have an active Bonk {label} launchpad pool, may use another quote route, or may already be migrated. SDK error: {raw}"));
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 2);
    }
}
