use super::{super::*, read_rpc::ReadRpc};
use crate::{
    adapters::{
        rpc::{
            read_http::{HttpReply, ReadHttpTransport, ReadRequest},
            read_runtime::SharedReadRuntime,
        },
        sdk::trade_command::ExecuteTrade,
    },
    domain::{route::PoolRouteHint, trade::Address},
};
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use solana_sdk::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Instant;

struct Transport {
    fixture: ReadRpc,
    calls: Mutex<Vec<(Value, Instant)>>,
    fail_fee: bool,
}
impl ReadHttpTransport for Transport {
    async fn fetch(&self, _: &Url, request: &ReadRequest) -> anyhow::Result<HttpReply> {
        let body = request.body();
        self.calls
            .lock()
            .unwrap()
            .push((body.clone(), Instant::now()));
        if self.fail_fee
            && body["method"] == "getAccountInfo"
            && body["params"][0]
                == sol_trade_sdk::instruction::utils::pumpswap::accounts::FEE_CONFIG.to_string()
        {
            return Ok(HttpReply {
                status: StatusCode::SERVICE_UNAVAILABLE,
                retry_after: None,
                body: vec![],
            });
        }
        let method = match body["method"].as_str().unwrap() {
            "getAccountInfo" => RpcRequest::GetAccountInfo,
            "getMultipleAccounts" => RpcRequest::GetMultipleAccounts,
            "getEpochInfo" => RpcRequest::GetEpochInfo,
            "getProgramAccounts" => RpcRequest::GetProgramAccounts,
            _ => panic!("protocol loader used a capability absent from its fixture"),
        };
        let result = self
            .fixture
            .respond(method, body["params"].clone())
            .unwrap();
        Ok(HttpReply {
            status: StatusCode::OK,
            retry_after: None,
            body: serde_json::to_vec(&json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                .unwrap(),
        })
    }
}

#[tokio::test]
async fn parameter_loader_shared_pump_swap_mint_discovery_retains_all_six_filtered_scans() {
    use sol_trade_sdk::instruction::utils::pumpswap;
    let (mut fixture, id, pool) = super::pump_swap::fixture(false, 1000);
    fixture.absent(pumpswap::get_pool_v2_pda(&pool.base_mint).unwrap());
    fixture.absent(pumpswap::get_canonical_pool_pda(&pool.base_mint));
    fixture.program_scan(pumpswap::accounts::AMM_PROGRAM, &[id]);
    let trade = command(Pool::PumpAmm, pool.base_mint, pool.quote_mint, None);
    let old = dex_params(
        &fixture.client(),
        trade.pool,
        &trade.mint,
        &trade.quote_mint,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let url = "https://parameter-fixture.invalid".to_owned();
    let runtime = Arc::new(SharedReadRuntime::with_test_transport(
        &[url.clone()],
        Transport {
            fixture: fixture.clone(),
            calls: Mutex::new(vec![]),
            fail_fee: false,
        },
    ));
    let current = load_for_trade(
        runtime.clone(),
        url,
        solana_commitment_config::CommitmentConfig::default(),
        &trade,
    )
    .await
    .unwrap();
    assert_eq!(witness(&old), witness(&current));
    let calls = runtime.test_transport().calls.lock().unwrap();
    let scans = calls
        .iter()
        .filter(|(body, _)| body["method"] == "getProgramAccounts")
        .collect::<Vec<_>>();
    assert_eq!(scans.len(), 6);
    for (body, _) in scans {
        assert_eq!(body["params"][1]["filters"].as_array().unwrap().len(), 3);
    }
    fixture.assert_read_only();
}
fn command(pool: Pool, mint: Pubkey, quote: Pubkey, id: Option<Pubkey>) -> ExecuteTrade {
    ExecuteTrade {
        pool,
        mint,
        quote_mint: quote,
        pool_id: id,
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(1000),
        sell_percent: None,
        slippage_bps: 1500,
        tip_sol: 0.0001,
        recent_blockhash: Hash::new_from_array([9; 32]),
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 0,
        simulate: false,
        use_mev: Some(false),
        mev_provider: None,
    }
}
fn cases() -> Vec<(ReadRpc, ExecuteTrade)> {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let (read, mint, _, _) = super::pump::fixture(false, false);
    let mut cases = vec![(read, command(Pool::Pump, mint, wsol, None))];
    let (read, id, pool) = super::pump_swap::fixture(false, 1000);
    cases.push((
        read,
        command(Pool::PumpAmm, pool.base_mint, pool.quote_mint, Some(id)),
    ));
    for kind in [Pool::Bonk, Pool::StonkFun] {
        let mut read = ReadRpc::default();
        let (id, state) =
            super::stonkfun::curve(&mut read, accounts::STONKFUN_STANDARD_PLATFORM_CONFIG, wsol);
        let id = if kind == Pool::Bonk {
            let derived =
                sol_trade_sdk::instruction::utils::bonk::get_pool_pda(&state.base_mint, &wsol)
                    .unwrap();
            read.account(
                derived,
                super::read_rpc::account(accounts::BONK, super::pool_bytes(&state)),
            );
            None
        } else {
            Some(id)
        };
        cases.push((read, command(kind, state.base_mint, wsol, id)));
    }
    for kind in [Pool::RaydiumCpmm, Pool::StonkFunSwap] {
        let (read, id, pool, _) = super::cpmm::fixture();
        cases.push((
            read,
            command(kind, pool.token0_mint, pool.token1_mint, Some(id)),
        ));
    }
    let (read, id, pool) = super::damm_v2::fixture();
    cases.push((
        read,
        command(
            Pool::MeteoraDammV2,
            pool.token_a_mint,
            pool.token_b_mint,
            Some(id),
        ),
    ));
    let (read, id, mint, quote) = super::dbc::shared_fixture();
    cases.push((read, command(Pool::MeteoraDbc, mint, quote, Some(id))));
    let (read, id, pool, _, _) = super::amm_v4::fixture();
    cases.push((
        read,
        command(Pool::RaydiumAmmV4, pool.coin_mint, pool.pc_mint, Some(id)),
    ));
    cases
}
fn witness(params: &DexParamEnum) -> Value {
    match params {
        DexParamEnum::PumpFun(p) => {
            json!({"kind":"pump","curve":p.bonding_curve.account.to_string(),"reserves":[p.bonding_curve.virtual_token_reserves,p.bonding_curve.virtual_sol_reserves,p.bonding_curve.real_token_reserves,p.bonding_curve.real_sol_reserves],"creator":p.bonding_curve.creator.to_string(),"program":p.token_program.to_string(),"ata":p.associated_bonding_curve.to_string(),"vault":p.creator_vault.to_string(),"fee":p.fee_recipient.to_string(),"sharing":p.fee_sharing_creator_vault_if_active.map(|key|key.to_string())})
        }
        DexParamEnum::PumpSwap(p) => {
            json!({"kind":"pump-amm","pool":p.pool.to_string(),"mints":[p.base_mint.to_string(),p.quote_mint.to_string()],"programs":[p.base_token_program.to_string(),p.quote_token_program.to_string()],"vaults":[p.pool_base_token_account.to_string(),p.pool_quote_token_account.to_string()],"reserves":[p.pool_base_token_reserves,p.pool_quote_token_reserves],"virtual":p.virtual_quote_reserves.to_string(),"fees":[p.fee_basis_points.lp_fee_basis_points,p.fee_basis_points.protocol_fee_basis_points,p.fee_basis_points.coin_creator_fee_basis_points],"recipient":p.protocol_fee_recipient_override.map(|key|key.to_string()),"extra":p.protocol_extra_fee_recipient_override.map(|key|key.to_string()),"creatorVault":p.coin_creator_vault_ata.to_string()})
        }
        DexParamEnum::Bonk(p) | DexParamEnum::StonkFun(p) => {
            json!({"kind":"launchlab","pool":p.pool_state.to_string(),"quote":p.quote_mint.to_string(),"vaults":[p.base_vault.to_string(),p.quote_vault.to_string()],"programs":[p.mint_token_program.to_string(),p.quote_token_program.to_string()],"fees":[p.trade_fee_rate,p.platform_fee_rate,p.creator_fee_rate],"reserves":[p.virtual_base.to_string(),p.virtual_quote.to_string(),p.real_base.to_string(),p.real_quote.to_string()],"platform":p.platform_config.to_string(),"global":p.global_config.to_string()})
        }
        DexParamEnum::RaydiumCpmm(p) | DexParamEnum::StonkFunSwap(p) => {
            json!({"kind":"cpmm","pool":p.pool_state.to_string(),"mints":[p.base_mint.to_string(),p.quote_mint.to_string()],"vaults":[p.base_vault.to_string(),p.quote_vault.to_string()],"programs":[p.base_token_program.to_string(),p.quote_token_program.to_string()],"reserves":[p.base_reserve,p.quote_reserve],"fees":[p.trade_fee_rate,p.protocol_fee_rate,p.fund_fee_rate,p.creator_fee_rate],"creator":[p.creator_fee_on as u64,u64::from(p.enable_creator_fee)],"transferFee":[p.base_transfer_fee.basis_points as u64,p.base_transfer_fee.maximum_fee,p.quote_transfer_fee.basis_points as u64,p.quote_transfer_fee.maximum_fee]})
        }
        DexParamEnum::MeteoraDammV2(p) => {
            json!({"kind":"damm","pool":p.pool.to_string(),"mints":[p.token_a_mint.to_string(),p.token_b_mint.to_string()],"vaults":[p.token_a_vault.to_string(),p.token_b_vault.to_string()],"programs":[p.token_a_program.to_string(),p.token_b_program.to_string()],"mode":p.swap_mode,"sysvar":p.include_rate_limiter_sysvar,"referral":p.referral_token_account.map(|key|key.to_string())})
        }
        DexParamEnum::MeteoraDbc(p) => {
            json!({"kind":"dbc","pool":p.pool.to_string(),"config":p.config.to_string(),"mints":[p.base_mint.to_string(),p.quote_mint.to_string()],"vaults":[p.base_vault.to_string(),p.quote_vault.to_string()],"programs":[p.base_token_program.to_string(),p.quote_token_program.to_string()],"reserves":[p.virtual_pool.pool_state.base_reserve,p.virtual_pool.pool_state.quote_reserve],"price":p.virtual_pool.pool_state.sqrt_price.to_string(),"sysvar":p.include_instructions_sysvar})
        }
        DexParamEnum::RaydiumAmmV4(p) => {
            json!({"kind":"amm-v4","pool":p.amm.to_string(),"market":p.serum_market.to_string(),"mints":[p.coin_mint.to_string(),p.pc_mint.to_string()],"vaults":[p.token_coin.to_string(),p.token_pc.to_string()],"reserves":[p.coin_reserve,p.pc_reserve],"fees":[p.swap_fee_numerator,p.swap_fee_denominator],"orders":[p.amm_open_orders.to_string(),p.amm_target_orders.to_string(),p.serum_bids.to_string(),p.serum_asks.to_string(),p.serum_event_queue.to_string()]})
        }
        DexParamEnum::StonkFunViaSol(p) => {
            use sol_trade_sdk::trading::core::params::{StonkFunMemeLeg, StonkFunSolHop};
            let target = match &p.meme_leg {
                StonkFunMemeLeg::Curve(value) => witness(&DexParamEnum::StonkFun(value.clone())),
                StonkFunMemeLeg::Graduated(value) => {
                    witness(&DexParamEnum::StonkFunSwap(value.clone()))
                }
            };
            let hop = match &p.sol_hop {
                StonkFunSolHop::RaydiumCpmm(value) => {
                    witness(&DexParamEnum::RaydiumCpmm(value.clone()))
                }
                _ => panic!("unreviewed hop"),
            };
            json!({"kind":"via-sol","target":target,"hop":hop})
        }
        _ => panic!("unexpected existing protocol variant"),
    }
}

#[tokio::test]
async fn parameter_loader_shared_preserves_all_nine_real_sdk_variants_and_warmup() {
    let url = "https://parameter-fixture.invalid".to_owned();
    let mut seen = std::collections::HashSet::new();
    for (fixture, trade) in cases() {
        let old = dex_params(
            &fixture.client(),
            trade.pool,
            &trade.mint,
            &trade.quote_mint,
            trade.pool_id.as_ref(),
            None,
            None,
        )
        .await
        .unwrap();
        let old_witness = witness(&old);
        let runtime = Arc::new(SharedReadRuntime::with_test_transport(
            &[url.clone()],
            Transport {
                fixture: fixture.clone(),
                calls: Mutex::new(vec![]),
                fail_fee: false,
            },
        ));
        let current = load_for_trade(
            runtime.clone(),
            url.clone(),
            solana_commitment_config::CommitmentConfig::default(),
            &trade,
        )
        .await
        .unwrap();
        assert_eq!(
            std::mem::discriminant(&old),
            std::mem::discriminant(&current)
        );
        assert_eq!(witness(&current), old_witness);
        let hint = PoolRouteHint {
            pool: trade.pool,
            mint: Address::from_bytes(trade.mint.to_bytes()),
            quote_mint: Address::from_bytes(trade.quote_mint.to_bytes()),
            pool_id: trade.pool_id.map(|id| Address::from_bytes(id.to_bytes())),
        };
        warmup_scoped(
            runtime.clone(),
            url.clone(),
            solana_commitment_config::CommitmentConfig::default(),
            &hint,
        )
        .await
        .unwrap();
        let calls = runtime.test_transport().calls.lock().unwrap();
        assert!(!calls.is_empty());
        for (body, _) in calls.iter() {
            seen.insert(body["method"].as_str().unwrap().to_owned());
        }
        for pair in calls.windows(2) {
            assert!(pair[1].1.duration_since(pair[0].1) >= std::time::Duration::from_millis(240));
        }
        fixture.assert_read_only();
    }
    assert_eq!(
        seen,
        std::collections::HashSet::from([
            "getAccountInfo".into(),
            "getMultipleAccounts".into(),
            "getEpochInfo".into()
        ])
    );
}

#[tokio::test]
async fn parameter_loader_shared_keeps_local_missing_pool_errors_before_any_rpc_admission() {
    let url = "https://parameter-fixture.invalid".to_owned();
    let runtime = Arc::new(SharedReadRuntime::with_test_transport(
        &[url],
        Transport {
            fixture: ReadRpc::default(),
            calls: Mutex::new(vec![]),
            fail_fee: false,
        },
    ));
    for (pool, name) in [
        (Pool::StonkFun, "stonkfun"),
        (Pool::StonkFunSwap, "stonkfun-swap"),
        (Pool::MeteoraDammV2, "meteora-damm-v2"),
        (Pool::MeteoraDbc, "meteora-dbc"),
        (Pool::RaydiumAmmV4, "raydium-amm-v4"),
        (Pool::RaydiumCpmm, "raydium-cpmm"),
    ] {
        let trade = command(
            pool,
            Pubkey::new_from_array([1; 32]),
            Pubkey::new_from_array([2; 32]),
            None,
        );
        let error = load_for_trade(
            runtime.clone(),
            "https://not-registered.invalid".into(),
            solana_commitment_config::CommitmentConfig::default(),
            &trade,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.to_string(), format!("pool_id is required for {name}"));
        let hint = PoolRouteHint {
            pool,
            mint: Address::from_bytes(trade.mint.to_bytes()),
            quote_mint: Address::from_bytes(trade.quote_mint.to_bytes()),
            pool_id: None,
        };
        assert_eq!(
            warmup_scoped(
                runtime.clone(),
                "https://not-registered.invalid".into(),
                solana_commitment_config::CommitmentConfig::default(),
                &hint
            )
            .await
            .err()
            .unwrap()
            .to_string(),
            format!("pool_id is required for {name}")
        );
    }
    assert!(runtime.test_transport().calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn parameter_loader_shared_stonkfun_explicit_hop_uses_the_same_scope_and_decoder() {
    let (mut fixture, hop_id, hop, _) = super::cpmm::fixture();
    let (id, state) = super::stonkfun::curve(
        &mut fixture,
        accounts::STONKFUN_STANDARD_PLATFORM_CONFIG,
        hop.token0_mint,
    );
    let mut trade = command(Pool::StonkFun, state.base_mint, state.quote_mint, Some(id));
    trade.sol_hop_pool = Some(Pool::RaydiumCpmm);
    trade.sol_hop_pool_id = Some(hop_id);
    let old = dex_params(
        &fixture.client(),
        trade.pool,
        &trade.mint,
        &trade.quote_mint,
        Some(&id),
        trade.sol_hop_pool,
        Some(&hop_id),
    )
    .await
    .unwrap();
    let url = "https://parameter-fixture.invalid".to_owned();
    let runtime = Arc::new(SharedReadRuntime::with_test_transport(
        &[url.clone()],
        Transport {
            fixture: fixture.clone(),
            calls: Mutex::new(vec![]),
            fail_fee: false,
        },
    ));
    let current = load_for_trade(
        runtime.clone(),
        url,
        solana_commitment_config::CommitmentConfig::default(),
        &trade,
    )
    .await
    .unwrap();
    assert_eq!(witness(&current), witness(&old));
    assert_eq!(runtime.test_transport().calls.lock().unwrap().len(), 13);
    fixture.assert_read_only();
}

const OPTIONAL_FLAG: &str = "SOL_BOT_PARAMETER_OPTIONAL_CASE";
const OPTIONAL_DONE: &str = "PARAMETER_OPTIONAL_DONE";
#[test]
fn parameter_loader_shared_sdk_optional_config_cannot_erase_transport_failure() {
    let child = crate::adapters::test_support::IsolatedTestProcess::spawn(
        "adapters::sdk::protocol_params::tests::shared::parameter_optional_child",
        OPTIONAL_FLAG,
        "fault",
    );
    child.await_line(OPTIONAL_DONE);
    child.finish();
}
#[tokio::test]
async fn parameter_optional_child() {
    if std::env::var(OPTIONAL_FLAG).is_err() {
        return;
    }
    use crate::{
        adapters::rpc::parameter_reads::{ParameterReadScope, ParameterReadSession},
        domain::parameter_read::ParameterReadFailure,
    };
    use sol_trade_sdk::instruction::utils::pumpswap;
    let (fixture, id, pool) = super::pump_swap::fixture(false, 1000);
    let url = "https://parameter-fixture.invalid".to_owned();
    let runtime = Arc::new(SharedReadRuntime::with_test_transport(
        &[url.clone()],
        Transport {
            fixture: fixture.clone(),
            calls: Mutex::new(vec![]),
            fail_fee: true,
        },
    ));
    let s = ParameterReadSession::new(
        runtime.clone(),
        url.clone(),
        solana_commitment_config::CommitmentConfig::default(),
        ParameterReadScope::new(
            Pool::PumpAmm,
            Address::from_bytes(pool.base_mint.to_bytes()),
            Some(Address::from_bytes(id.to_bytes())),
        ),
    );
    assert!(pumpswap::fetch_fee_config(s.rpc()).await.is_none());
    let error = s.finish(Ok("SDK erased optional error")).err().unwrap();
    assert!(error
        .downcast_ref::<ParameterReadFailure>()
        .unwrap()
        .allows_lane_fallback());
    let trade = command(Pool::PumpAmm, pool.base_mint, pool.quote_mint, Some(id));
    let error = load_for_trade(
        runtime,
        url,
        solana_commitment_config::CommitmentConfig::default(),
        &trade,
    )
    .await
    .err()
    .unwrap();
    assert!(error.downcast_ref::<ParameterReadFailure>().is_some());
    fixture.assert_read_only();
    println!("{OPTIONAL_DONE}");
}
