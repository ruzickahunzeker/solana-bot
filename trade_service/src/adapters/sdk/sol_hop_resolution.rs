use super::{pool_resolution::DiscoveryLane, protocol_params::checks::ensure_pool_contains_quote};
use crate::{
    adapters::rpc::{
        read_http::ReadHttpTransport, read_runtime::SharedReadRuntime,
        sol_hop_reads::SolHopReadSession,
    },
    domain::trade::{Address, WSOL_MINT},
    sanitize::redact_urls,
};
use anyhow::{bail, Result};
use sol_trade_sdk::trading::core::params::RaydiumCpmmParams;
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_sdk::pubkey::Pubkey;
use std::{
    collections::HashMap,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub(crate) async fn resolve<T: ReadHttpTransport + 'static>(
    runtime: Arc<SharedReadRuntime<T>>,
    lanes: &[DiscoveryLane],
    cache: &Mutex<HashMap<Pubkey, (Pubkey, tokio::time::Instant)>>,
    quote_mint: Pubkey,
) -> Result<Pubkey> {
    let mut cache = cache.lock().await;
    if let Some((pool_id, expires_at)) = cache.get(&quote_mint) {
        if *expires_at > tokio::time::Instant::now() {
            return Ok(*pool_id);
        }
    }
    let wsol = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
    if quote_mint == wsol {
        bail!("quote mint is already WSOL")
    }
    let mut last_error = None;
    let mut complete_empty_scans = 0;
    for lane in lanes {
        let started = Instant::now();
        let deadline = started + Duration::from_secs(5);
        let session = SolHopReadSession::new(
            runtime.clone(),
            lane.url().to_owned(),
            lane.commitment(),
            Address::from_bytes(quote_mint.to_bytes()),
            deadline,
        );
        let found = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let result = on_rpc(session.rpc(), &quote_mint, &wsol).await;
            session.finish(result)
        })
        .await;
        match found {
            Ok(Ok(Some(pool_id))) => {
                cache.insert(
                    quote_mint,
                    (
                        pool_id,
                        tokio::time::Instant::now() + Duration::from_secs(30),
                    ),
                );
                return Ok(pool_id);
            }
            Ok(Ok(None)) => {
                complete_empty_scans += 1;
                tracing::info!(
                    elapsed_ms = started.elapsed().as_millis(),
                    "SOL hop RPC scan found no CPMM pool"
                );
                if complete_empty_scans >= 2 {
                    bail!("no WSOL/Quote Raydium CPMM pool found on two RPC lanes")
                }
            }
            Ok(Err(error)) => {
                last_error = Some(redact_urls(&error.to_string()));
                tracing::warn!(
                    elapsed_ms = started.elapsed().as_millis(),
                    "SOL hop RPC lane failed"
                );
            }
            Err(_) => {
                last_error = Some("SOL hop discovery timed out".to_string());
                tracing::warn!(
                    elapsed_ms = started.elapsed().as_millis(),
                    "SOL hop RPC lane timed out"
                );
            }
        }
    }
    bail!(
        "no verified WSOL/Quote Raydium CPMM pool: {}",
        last_error.unwrap_or_else(|| "no Default RPC lane configured".to_string())
    )
}

pub(super) async fn on_rpc(
    rpc: &RpcClient,
    quote_mint: &Pubkey,
    wsol: &Pubkey,
) -> Result<Option<Pubkey>> {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::{accounts, fetch_pool_state};
    use sol_trade_sdk::instruction::utils::raydium_cpmm_types::POOL_STATE_DISCRIMINATOR;
    for (quote_offset, wsol_offset) in [(168usize, 200usize), (200, 168)] {
        let config = RpcProgramAccountsConfig {
            filters: Some(vec![
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(0, &POOL_STATE_DISCRIMINATOR)),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                    quote_offset,
                    quote_mint.as_ref(),
                )),
                RpcFilterType::Memcmp(Memcmp::new_base58_encoded(wsol_offset, wsol.as_ref())),
            ]),
            account_config: RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                data_slice: None,
                commitment: None,
                min_context_slot: None,
            },
            with_context: None,
            sort_results: None,
        };
        #[allow(deprecated)]
        let pools = rpc
            .get_program_ui_accounts_with_config(&accounts::RAYDIUM_CPMM, config)
            .await?;
        for (pool_id, _) in pools {
            let state = fetch_pool_state(rpc, &pool_id).await?;
            if [state.token0_mint, state.token1_mint].contains(wsol)
                && [state.token0_mint, state.token1_mint].contains(quote_mint)
            {
                let params = RaydiumCpmmParams::from_pool_address_by_rpc(rpc, &pool_id).await?;
                let mints = [params.base_mint, params.quote_mint];
                ensure_pool_contains_quote("stonkfun Via-SOL hop", quote_mint, &mints)?;
                ensure_pool_contains_quote("stonkfun Via-SOL hop", wsol, &mints)?;
                let (base_balance, quote_balance) = tokio::try_join!(
                    rpc.get_token_account_balance(&params.base_vault),
                    rpc.get_token_account_balance(&params.quote_vault)
                )?;
                if base_balance.amount.parse::<u64>()? == 0
                    || quote_balance.amount.parse::<u64>()? == 0
                {
                    continue;
                }
                return Ok(Some(pool_id));
            }
        }
    }
    Ok(None)
}
