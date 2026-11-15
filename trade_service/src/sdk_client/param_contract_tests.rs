use super::*;
use crate::adapters::sdk::protocol_params::{
    checks::{ensure_pool_contains_mint, ensure_pool_contains_quote},
    describe_bonk_param_error,
};
use crate::models::{USD1_MINT, USDC_MINT};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn quote_token_selection_v1_baseline() {
    let mut cases = vec![];
    for pool in [
        Pool::Pump,
        Pool::PumpAmm,
        Pool::Bonk,
        Pool::StonkFun,
        Pool::StonkFunSwap,
        Pool::MeteoraDammV2,
        Pool::MeteoraDbc,
        Pool::RaydiumAmmV4,
        Pool::RaydiumCpmm,
    ] {
        for quote in [
            WSOL_MINT,
            crate::models::SOL_MINT,
            USDC_MINT,
            USD1_MINT,
            "11111111111111111111111111111111",
        ] {
            let mint = Pubkey::from_str(quote).unwrap();
            for side in ["buy", "sell"] {
                let result = if side == "buy" {
                    trade_token_type_for_buy(pool, &mint)
                } else {
                    trade_token_type_for_sell(pool, &mint)
                };
                let result = match result {
                    Ok(token) => json!({"type":format!("{token:?}")}),
                    Err(error) => json!({"error":error.to_string()}),
                };
                cases.push(json!({"pool":pool,"quote":quote,"side":side,"result":result}));
            }
        }
    }
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&cases).unwrap())),
        "bea3c71308bb919260ed7e6df7852a10f0657a0096195804df1961b7ef068a96"
    );
}

#[test]
fn pool_member_checks_keep_direction_and_exact_error_text() {
    let base = Pubkey::new_from_array([0; 32]);
    let quote = Pubkey::from_str(WSOL_MINT).unwrap();
    let absent = Pubkey::from_str(USDC_MINT).unwrap();
    for mints in [[base, quote], [quote, base]] {
        assert!(ensure_pool_contains_mint("fixture", &base, &mints).is_ok());
        assert!(ensure_pool_contains_quote("fixture", &quote, &mints).is_ok());
        let names = mints
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            ensure_pool_contains_mint("fixture", &absent, &mints)
                .unwrap_err()
                .to_string(),
            format!("fixture pool does not contain requested mint {absent}; pool mints: {names}")
        );
        assert_eq!(
            ensure_pool_contains_quote("fixture", &absent, &mints)
                .unwrap_err()
                .to_string(),
            format!(
                "fixture pool does not contain requested quote_mint {absent}; pool mints: {names}"
            )
        );
    }
    assert_eq!(
        ensure_pool_contains_quote("fixture", &absent, &[])
            .unwrap_err()
            .to_string(),
        format!("fixture pool does not contain requested quote_mint {absent}; pool mints: ")
    );
}

#[test]
fn bonk_missing_pool_description_keeps_quote_name_and_original_error() {
    let mint = Pubkey::new_from_array([0; 32]);
    for (quote, label) in [(WSOL_MINT, "WSOL"), (USD1_MINT, "USD1")] {
        let quote = Pubkey::from_str(quote).unwrap();
        assert_eq!(describe_bonk_param_error(&mint,&quote,anyhow!("AccountNotFound: synthetic")).to_string(),format!("bonk {label} pool not found for mint {mint}; this mint may not have an active Bonk {label} launchpad pool, may use another quote route, or may already be migrated. SDK error: AccountNotFound: synthetic"));
        assert_eq!(
            describe_bonk_param_error(&mint, &quote, anyhow!("synthetic timeout")).to_string(),
            "synthetic timeout"
        );
    }
}
