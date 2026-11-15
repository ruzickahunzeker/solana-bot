//! Owned project intent and frozen provider JSON only; no SDK, signer or RPC.
use super::*;
use crate::{
    adapters::pumpapi::request::build_request,
    application::trade_command::test_support::command,
    domain::trade::{
        ExecutionMethod, ExecutionMode, MevProvider, Pool, Side, TradeAmount, USD1_MINT, USDC_MINT,
        WSOL_MINT,
    },
};
use sha2::{Digest, Sha256};

fn config() -> PumpApiConfig {
    PumpApiConfig {
        enabled: true,
        url: "https://fixture.invalid/local".into(),
        timeout_ms: 1000,
        default_priority_fee_sol: 0.005,
        lightning_enabled: false,
    }
}
fn address(value: &str) -> Address {
    Address::from_bytes(bs58::decode(value).into_vec().unwrap().try_into().unwrap())
}
fn assert_command(actual: &TradeCommand, expected: &TradeCommand) {
    assert_eq!(actual.wallet_id, expected.wallet_id);
    assert_eq!(actual.idempotency_key, expected.idempotency_key);
    assert_eq!(actual.route.pool, expected.route.pool);
    assert_eq!(actual.route.mint, expected.route.mint);
    assert_eq!(actual.route.quote_mint, expected.route.quote_mint);
    assert_eq!(actual.route.pool_id, expected.route.pool_id);
    assert_eq!(actual.route.sol_hop_pool, expected.route.sol_hop_pool);
    assert_eq!(actual.route.sol_hop_pool_id, expected.route.sol_hop_pool_id);
    assert_eq!(actual.amount, expected.amount);
    assert_eq!(actual.execution.method, expected.execution.method);
    assert_eq!(
        actual.execution.slippage_bps,
        expected.execution.slippage_bps
    );
    assert_eq!(
        actual.execution.tip_sol.to_bits(),
        expected.execution.tip_sol.to_bits()
    );
    assert_eq!(actual.execution.use_mev, expected.execution.use_mev);
    assert_eq!(
        actual.execution.mev_provider,
        expected.execution.mev_provider
    );
    assert_eq!(actual.execution.mode, expected.execution.mode);
    assert_eq!(actual.aggregator_sol, expected.aggregator_sol);
    assert_eq!(
        actual.pumpapi_priority_fee_sol.to_bits(),
        expected.pumpapi_priority_fee_sol.to_bits()
    );
}

#[test]
fn pumpapi_h2b1_snapshot_owns_all_context_without_aliasing_mutable_source() {
    let mut source = command();
    let expected = source.clone();
    let trade = pumpapi_trade(&source, config(), true);
    source.wallet_id.clear();
    source.idempotency_key.clear();
    source.route.pool = Pool::RaydiumCpmm;
    source.route.mint = Address::from_bytes([3; 32]);
    source.route.quote_mint = Address::from_bytes([4; 32]);
    source.route.pool_id = Some(Address::from_bytes([5; 32]));
    source.route.sol_hop_pool = Some(Pool::RaydiumAmmV4);
    source.route.sol_hop_pool_id = Some(Address::from_bytes([6; 32]));
    source.amount = TradeAmount::SellPercent { percent: 100 };
    source.execution.method = ExecutionMethod::Sdk;
    source.execution.mode = ExecutionMode::Simulate;
    source.execution.slippage_bps = 1;
    source.execution.tip_sol = 0.03;
    source.execution.use_mev = Some(true);
    source.execution.mev_provider = Some(MevProvider::Temporal);
    source.aggregator_sol = true;
    source.pumpapi_priority_fee_sol = 0.04;
    assert_command(trade.command(), &expected);
    assert!(!trade.simulate());
    assert!(trade.wait_tx_confirmed());
    assert_eq!(trade.config().url, "https://fixture.invalid/local");
}

#[test]
fn pumpapi_h2b1_real_snapshot_projection_keeps_old_eighty_request_fingerprint() {
    let mut samples = vec![];
    for quote in [
        address(WSOL_MINT),
        address(USDC_MINT),
        address(USD1_MINT),
        Address::from_bytes([4; 32]),
    ] {
        for pool in [None, Some(Address::from_bytes([5; 32]))] {
            for side in [Side::Buy, Side::Sell] {
                for amount in [1u64, 1_000_000, 1_000_000_000, (1u64 << 53) + 1, u64::MAX] {
                    let mut source = command();
                    source.route.quote_mint = quote;
                    source.route.pool_id = pool;
                    source.amount = match side {
                        Side::Buy if quote == address(WSOL_MINT) => {
                            TradeAmount::BuySol { lamports: amount }
                        }
                        Side::Buy => TradeAmount::BuyQuote {
                            mint: quote,
                            units: amount,
                        },
                        Side::Sell => TradeAmount::SellPercent { percent: 100 },
                    };
                    let trade = pumpapi_trade(&source, config(), true);
                    let projected = pumpapi_request(Address::from_bytes([1; 32]), &trade);
                    assert_eq!(projected.side, side);
                    assert_eq!(projected.quote_mint, quote);
                    samples.push(build_request(&projected).unwrap());
                }
            }
        }
    }
    assert_eq!(samples.len(), 80);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&samples).unwrap())
        ),
        "79eae7218b84f98ccf08c048282855ff952b057ff1701c2525dfd74b4606cd0c"
    );
}

#[test]
fn pumpapi_h2b1_protocol_identity_hop_and_execution_options_are_retained_not_transmitted() {
    let original = pumpapi_trade(&command(), config(), true);
    let wallet = Address::from_bytes([1; 32]);
    let original_request = build_request(&pumpapi_request(wallet, &original)).unwrap();
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
        for simulate in [false, true] {
            let mut source = command();
            source.wallet_id = " literal-wallet ".into();
            source.idempotency_key = " literal-key-no-trim ".into();
            source.route.pool = pool;
            source.route.sol_hop_pool = Some(Pool::RaydiumCpmm);
            source.route.sol_hop_pool_id = Some(Address::from_bytes([7; 32]));
            source.execution.tip_sol = 0.02;
            source.execution.use_mev = Some(true);
            source.execution.mev_provider = Some(MevProvider::Temporal);
            source.execution.mode = ExecutionMode::resolve(Some(simulate), false);
            source.aggregator_sol = true;
            let trade = pumpapi_trade(&source, config(), false);
            assert_command(trade.command(), &source);
            assert_eq!(trade.simulate(), simulate);
            assert!(!trade.wait_tx_confirmed());
            assert_eq!(
                build_request(&pumpapi_request(wallet, &trade)).unwrap(),
                original_request
            );
        }
    }
}

#[test]
fn pumpapi_h2b1_exact_amount_kind_and_quote_override_survive_projection() {
    for amount in [
        TradeAmount::BuySol { lamports: 9 },
        TradeAmount::BuyQuote {
            mint: Address::from_bytes([4; 32]),
            units: 11,
        },
        TradeAmount::SellPercent { percent: 100 },
    ] {
        let mut source = command();
        source.amount = amount;
        let trade = pumpapi_trade(&source, config(), false);
        assert_eq!(trade.command().amount, amount);
        let request = pumpapi_request(Address::from_bytes([1; 32]), &trade);
        assert_eq!(request.amount_units, amount.buy_units());
        assert_eq!(request.sell_percent, amount.sell_percent());
        assert_eq!(request.quote_mint, source.quote_mint());
        // The legacy effective Quote does not erase the original route hint.
        assert_eq!(trade.command().route.quote_mint, address(WSOL_MINT));
    }
}

#[test]
fn pumpapi_h2b1_fee_bit_patterns_and_provider_defaults_are_not_conflated() {
    for bits in [
        0u64,
        (-0.0f64).to_bits(),
        0.0002f64.to_bits(),
        f64::NAN.to_bits() | 7,
    ] {
        let mut source = command();
        source.pumpapi_priority_fee_sol = f64::from_bits(bits);
        source.execution.tip_sol = f64::from_bits(bits ^ (1u64 << 63));
        let trade = pumpapi_trade(&source, config(), true);
        let request = pumpapi_request(Address::from_bytes([1; 32]), &trade);
        assert_eq!(request.priority_fee_sol.to_bits(), bits);
        assert_command(trade.command(), &source);
        assert_eq!(trade.config().default_priority_fee_sol, 0.005);
    }
}

#[test]
fn pumpapi_h2b1_editing_a_wire_projection_does_not_edit_the_local_intent() {
    let source = command();
    let trade = pumpapi_trade(&source, config(), false);
    let wallet = Address::from_bytes([1; 32]);
    let before = build_request(&pumpapi_request(wallet, &trade)).unwrap();
    let mut projection = pumpapi_request(wallet, &trade);
    projection.wallet = Address::from_bytes([8; 32]);
    projection.mint = Address::from_bytes([9; 32]);
    projection.quote_mint = Address::from_bytes([10; 32]);
    projection.amount_units = Some(99);
    assert_command(trade.command(), &source);
    assert_ne!(build_request(&projection).unwrap(), before);
    assert_eq!(
        build_request(&pumpapi_request(wallet, &trade)).unwrap(),
        before
    );
}

#[tokio::test]
async fn pumpapi_h2b1_owned_intent_survives_suspend_and_source_drop() {
    let mut source = command();
    source.wallet_id = " snapshot-wallet ".into();
    source.route.pool = Pool::MeteoraDammV2;
    source.route.pool_id = Some(Address::from_bytes([5; 32]));
    let expected = source.clone();
    let trade = pumpapi_trade(&source, config(), true);
    drop(source);
    let moved = tokio::spawn(async move {
        tokio::task::yield_now().await;
        trade
    })
    .await
    .unwrap();
    assert_command(moved.command(), &expected);
    assert!(moved.wait_tx_confirmed());
}
