use anyhow::{bail, Context, Result};
use solana_account_decoder::UiAccountData;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_request::TokenAccountsFilter};
use solana_sdk::pubkey::Pubkey;
use std::{sync::Arc, time::Duration};

use crate::{
    application::ports::wallet_reads::WalletReadPort,
    domain::{trade::Address, wallet::QuoteBalance},
};

pub(crate) struct RpcWalletReader {
    wallet: Address,
    rpcs: Vec<Arc<RpcClient>>,
}

impl RpcWalletReader {
    pub(crate) fn new(wallet: Address, rpcs: Vec<Arc<RpcClient>>) -> Self {
        Self { wallet, rpcs }
    }
}

impl WalletReadPort for RpcWalletReader {
    async fn quote_balance(&self, mint: Address) -> Result<QuoteBalance> {
        let rpc = self
            .rpcs
            .first()
            .context("no Default RPC lane configured")?;
        let wallet = Pubkey::new_from_array(self.wallet.to_bytes());
        let mint = Pubkey::new_from_array(mint.to_bytes());
        let supply = rpc.get_token_supply(&mint).await?;
        let balance_units = token_balance(rpc, &wallet, &mint).await?;
        let balance_lamports = rpc.get_balance(&wallet).await?;
        Ok(QuoteBalance {
            decimals: supply.decimals,
            balance_units,
            balance_lamports,
        })
    }

    async fn wallet_balances(&self, wallets: &[Address]) -> Result<Vec<u64>> {
        let wallets = wallets
            .iter()
            .map(|v| Pubkey::new_from_array(v.to_bytes()))
            .collect::<Vec<_>>();
        let mut last_error = None;
        for rpc in &self.rpcs {
            match tokio::time::timeout(Duration::from_secs(4), rpc.get_multiple_accounts(&wallets))
                .await
            {
                Ok(Ok(accounts)) => {
                    return Ok(accounts
                        .into_iter()
                        .map(|account| account.map(|v| v.lamports).unwrap_or(0))
                        .collect())
                }
                Ok(Err(error)) => last_error = Some(error.to_string()),
                Err(_) => last_error = Some("RPC balance query timed out".to_string()),
            }
        }
        bail!(
            "wallet balances unavailable: {}",
            last_error.unwrap_or_else(|| "no Default RPC lane configured".to_string())
        )
    }
}

/// Existing SDK token-balance semantics, now requiring only RPC plus public addresses.
pub(crate) async fn token_balance(rpc: &RpcClient, wallet: &Pubkey, mint: &Pubkey) -> Result<u64> {
    let accounts = match rpc
        .get_token_accounts_by_owner(wallet, TokenAccountsFilter::Mint(*mint))
        .await
    {
        Ok(accounts) => accounts,
        Err(error) => {
            tracing::warn!(
                mint = %mint,
                error = %crate::sanitize::redact_urls(&error.to_string()),
                "token account scan failed, trying ATA balance fallback"
            );
            return token_balance_from_atas(rpc, wallet, mint).await;
        }
    };
    if accounts.is_empty() {
        return Ok(0);
    }
    let mut total = 0_u64;
    let mut parsed_count = 0_u64;
    for account in accounts {
        if let UiAccountData::Json(parsed) = account.account.data {
            if parsed.program != "spl-token" && parsed.program != "spl-token-2022" {
                continue;
            }
            parsed_count += 1;
            let amount = parsed
                .parsed
                .get("info")
                .and_then(|info| info.get("tokenAmount"))
                .and_then(|token_amount| token_amount.get("amount"))
                .and_then(|amount| amount.as_str())
                .context("missing token amount in RPC response")?
                .parse::<u64>()
                .context("parse token amount")?;
            total = total.saturating_add(amount);
        }
    }
    if parsed_count == 0 {
        token_balance_from_atas(rpc, wallet, mint).await
    } else {
        Ok(total)
    }
}

async fn token_balance_from_atas(rpc: &RpcClient, wallet: &Pubkey, mint: &Pubkey) -> Result<u64> {
    let mut last_error = None;
    for token_program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
        let ata = spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
            wallet,
            mint,
            &token_program,
        );
        match rpc.get_token_account_balance(&ata).await {
            Ok(balance) => {
                let amount = balance
                    .amount
                    .parse::<u64>()
                    .context("parse ATA token amount")?;
                if amount > 0 {
                    return Ok(amount);
                }
            }
            Err(error) => last_error = Some(error),
        }
    }
    if let Some(error) = last_error {
        return Err(error.into());
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use solana_client::rpc_request::RpcRequest;
    use std::collections::HashMap;

    fn address(byte: u8) -> Address {
        Address::from_bytes([byte; 32])
    }

    fn rpc(mocks: impl IntoIterator<Item = (RpcRequest, Value)>) -> Arc<RpcClient> {
        Arc::new(RpcClient::new_mock_with_mocks(
            "fails".into(),
            mocks.into_iter().collect::<HashMap<_, _>>(),
        ))
    }

    fn response(value: Value) -> Value {
        json!({"context":{"slot":42},"value":value})
    }

    fn account(lamports: u64) -> Value {
        json!({"lamports":lamports,"owner":"11111111111111111111111111111111","data":["","base64"],"executable":false,"rentEpoch":0,"space":0})
    }

    fn token_account(program: &str, amount: Value) -> Value {
        json!({"pubkey":"11111111111111111111111111111111","account":{
            "lamports":1,"owner":spl_token_interface::ID.to_string(),"executable":false,"rentEpoch":0,"space":165,
            "data":{"program":program,"parsed":{"info":{"tokenAmount":{"amount":amount}}},"space":165}
        }})
    }

    #[tokio::test]
    async fn quote_read_requires_only_public_addresses_and_rpc_mocks() {
        let reader = RpcWalletReader::new(
            address(1),
            vec![rpc([
                (
                    RpcRequest::GetTokenSupply,
                    response(
                        json!({"amount":"10000000000000000000","decimals":6,"uiAmount":null,"uiAmountString":"10000000000000"}),
                    ),
                ),
                (
                    RpcRequest::GetTokenAccountsByOwner,
                    response(json!([
                        token_account("spl-token", json!("9007199254740993")),
                        token_account("spl-token-2022", json!("7")),
                        token_account("unknown-program", json!("100")),
                    ])),
                ),
                (RpcRequest::GetBalance, response(json!(123456789))),
            ])],
        );
        assert_eq!(
            reader.quote_balance(address(2)).await.unwrap(),
            QuoteBalance {
                decimals: 6,
                balance_units: 9007199254741000,
                balance_lamports: 123456789
            }
        );
    }

    #[tokio::test]
    async fn bulk_balance_preserves_order_and_missing_account_zero() {
        let reader = RpcWalletReader::new(
            address(1),
            vec![rpc([(
                RpcRequest::GetMultipleAccounts,
                response(json!([account(u64::MAX), null, account(9)])),
            )])],
        );
        assert_eq!(
            reader
                .wallet_balances(&[address(1), address(2), address(3)])
                .await
                .unwrap(),
            vec![u64::MAX, 0, 9]
        );
    }

    #[tokio::test]
    async fn bulk_balance_tries_backup_after_rpc_failure() {
        let reader = RpcWalletReader::new(
            address(1),
            vec![
                rpc([]),
                rpc([(
                    RpcRequest::GetMultipleAccounts,
                    response(json!([account(17)])),
                )]),
            ],
        );
        assert_eq!(
            reader.wallet_balances(&[address(1)]).await.unwrap(),
            vec![17]
        );
    }

    #[tokio::test]
    async fn empty_lanes_keep_original_errors() {
        let reader = RpcWalletReader::new(address(1), vec![]);
        assert_eq!(
            reader
                .quote_balance(address(2))
                .await
                .unwrap_err()
                .to_string(),
            "no Default RPC lane configured"
        );
        assert_eq!(
            reader.wallet_balances(&[]).await.unwrap_err().to_string(),
            "wallet balances unavailable: no Default RPC lane configured"
        );
    }

    #[tokio::test]
    async fn token_scan_preserves_empty_zero_and_saturating_integer_sum() {
        let wallet = Pubkey::new_from_array([1; 32]);
        let mint = Pubkey::new_from_array([2; 32]);
        let empty = rpc([(RpcRequest::GetTokenAccountsByOwner, response(json!([])))]);
        assert_eq!(token_balance(&empty, &wallet, &mint).await.unwrap(), 0);
        let overflowing = rpc([(
            RpcRequest::GetTokenAccountsByOwner,
            response(json!([
                token_account("spl-token", json!(u64::MAX.to_string())),
                token_account("spl-token-2022", json!("1")),
            ])),
        )]);
        assert_eq!(
            token_balance(&overflowing, &wallet, &mint).await.unwrap(),
            u64::MAX
        );
    }

    #[tokio::test]
    async fn malformed_token_amount_retains_original_error_instead_of_zero_balance() {
        let wallet = Pubkey::new_from_array([1; 32]);
        let mint = Pubkey::new_from_array([2; 32]);
        for (amount, message) in [
            (json!(null), "missing token amount in RPC response"),
            (json!(1), "missing token amount in RPC response"),
            (json!("-1"), "parse token amount"),
            (json!("18446744073709551616"), "parse token amount"),
        ] {
            let client = rpc([(
                RpcRequest::GetTokenAccountsByOwner,
                response(json!([token_account("spl-token", amount)])),
            )]);
            assert_eq!(
                token_balance(&client, &wallet, &mint)
                    .await
                    .unwrap_err()
                    .to_string(),
                message
            );
        }
    }

    #[tokio::test]
    async fn scan_failure_and_unparsed_accounts_retain_ata_fallback() {
        let wallet = Pubkey::new_from_array([1; 32]);
        let mint = Pubkey::new_from_array([2; 32]);
        for scan in [
            None,
            Some(response(json!([token_account(
                "unknown-program",
                json!("10")
            )]))),
            Some(response(
                json!([{"pubkey":"11111111111111111111111111111111","account":account(1)}]),
            )),
        ] {
            let mut mocks = vec![(
                RpcRequest::GetTokenAccountBalance,
                response(
                    json!({"amount":"9007199254740993","decimals":6,"uiAmount":null,"uiAmountString":"9007199254.740993"}),
                ),
            )];
            if let Some(scan) = scan {
                mocks.push((RpcRequest::GetTokenAccountsByOwner, scan));
            }
            assert_eq!(
                token_balance(&rpc(mocks), &wallet, &mint).await.unwrap(),
                9007199254740993
            );
        }
    }

    #[test]
    fn standard_ata_derivation_matches_previous_sdk_fast_helper() {
        for wallet_byte in [0, 1, 42, 255] {
            for mint_byte in [0, 2, 99, 255] {
                let wallet = Pubkey::new_from_array([wallet_byte; 32]);
                let mint = Pubkey::new_from_array([mint_byte; 32]);
                for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                    assert_eq!(spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&wallet,&mint,&program),sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(&wallet,&mint,&program));
                }
            }
        }
    }
}
