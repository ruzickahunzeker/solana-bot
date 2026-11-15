use super::*;
use crate::{
    application::{ordinary_claims::test_support::fixture, trade_command::test_support::command},
    config::TradePersistenceMode,
    domain::trade::{Address, ExecutionMethod, ExecutionMode, MevProvider, Pool, TradeAmount},
};

#[test]
fn command_binding_checks_every_field_and_preserves_float_bits_and_literal_keys() {
    let original = command();
    let changes: &[(&str, fn(&mut TradeCommand))] = &[
        ("wallet", |c| c.wallet_id.push('x')),
        ("key", |c| {
            c.idempotency_key = c.idempotency_key.trim().into()
        }),
        ("pool", |c| c.route.pool = Pool::MeteoraDammV2),
        ("mint", |c| c.route.mint = Address::from_bytes([9; 32])),
        ("quote", |c| {
            c.route.quote_mint = Address::from_bytes([9; 32])
        }),
        ("pool_id", |c| {
            c.route.pool_id = Some(Address::from_bytes([9; 32]))
        }),
        ("hop", |c| c.route.sol_hop_pool = Some(Pool::RaydiumCpmm)),
        ("hop_id", |c| {
            c.route.sol_hop_pool_id = Some(Address::from_bytes([9; 32]))
        }),
        ("amount", |c| c.amount = TradeAmount::BuySol { lamports: 1 }),
        ("method", |c| c.execution.method = ExecutionMethod::Sdk),
        ("slippage", |c| c.execution.slippage_bps += 1),
        ("tip", |c| {
            c.execution.tip_sol = f64::from_bits(c.execution.tip_sol.to_bits() + 1)
        }),
        ("use_mev", |c| c.execution.use_mev = None),
        ("provider", |c| {
            c.execution.mev_provider = Some(MevProvider::Temporal)
        }),
        ("mode", |c| c.execution.mode = ExecutionMode::Simulate),
        ("aggregate", |c| c.aggregator_sol = true),
        ("priority_fee", |c| {
            c.pumpapi_priority_fee_sol = f64::from_bits(c.pumpapi_priority_fee_sol.to_bits() + 1)
        }),
    ];
    assert!(same_command(&original, &original.clone()));
    for (name, change) in changes {
        let mut altered = original.clone();
        change(&mut altered);
        assert!(!same_command(&original, &altered), "{name}");
        assert!(!same_command(&altered, &original), "{name}");
    }
    let mut zero = original.clone();
    zero.execution.tip_sol = 0.0;
    let mut negative_zero = zero.clone();
    negative_zero.execution.tip_sol = -0.0;
    assert!(!same_command(&zero, &negative_zero));
}

#[tokio::test]
async fn owner_binding_rejects_each_changed_identity_or_command_without_leaking_in_debug() {
    let f = fixture(TradePersistenceMode::Durable).await;
    let changes: &[(&str, fn(&mut ExecutionOwner))] = &[
        ("request_id", |o| o.request_id.push('x')),
        ("request_hash", |o| o.request_hash.push('x')),
        ("mode", |o| o.mode = TradeJournalMode::Shadow),
        ("operation", |o| o.operation = ClaimOperation::SdkTrade),
        ("wallet_id", |o| o.wallet_id.push('x')),
        ("wallet_pubkey", |o| o.wallet_pubkey.push('x')),
        ("key", |o| {
            o.idempotency_key = o.idempotency_key.trim().into()
        }),
        ("command", |o| o.command.execution.slippage_bps += 1),
    ];
    let baseline = ExecutionOwner::fixture(&f.execution);
    baseline.ensure_binding(&f.execution).unwrap();
    assert_eq!(format!("{baseline:?}"), "ExecutionOwner(<redacted>)");
    for (name, change) in changes {
        let mut owner = ExecutionOwner::new(&f.execution, baseline.nonce.clone()).unwrap();
        assert_eq!(owner, baseline);
        change(&mut owner);
        assert_ne!(owner, baseline, "{name}");
        assert!(owner.ensure_binding(&f.execution).is_err(), "{name}");
    }
}

#[tokio::test]
async fn owner_constructor_rejects_invalid_nonces_and_equality_distinguishes_valid_owners() {
    let f = fixture(TradePersistenceMode::Memory).await;
    for nonce in [
        "".into(),
        "a".repeat(31),
        "a".repeat(33),
        "z".repeat(32),
        " ".repeat(32),
    ] {
        assert!(ExecutionOwner::new(&f.execution, nonce).is_err());
    }
    let a = ExecutionOwner::new(&f.execution, "a".repeat(32)).unwrap();
    let b = ExecutionOwner::new(&f.execution, "b".repeat(32)).unwrap();
    assert_ne!(a, b);
    a.ensure_binding(&f.execution).unwrap();
    b.ensure_binding(&f.execution).unwrap();
}

#[tokio::test]
async fn owner_constructor_refuses_command_wallet_or_literal_key_drift_from_original_reservation() {
    let f = fixture(TradePersistenceMode::Durable).await;
    for field in ["wallet", "key", "invalid_amount", "simulate"] {
        let mut command = f.execution.command().clone();
        match field {
            "wallet" => command.wallet_id.push('x'),
            "key" => command.idempotency_key = command.idempotency_key.trim().into(),
            "invalid_amount" => command.amount = TradeAmount::SellPercent { percent: 100 },
            "simulate" => command.execution.mode = ExecutionMode::Simulate,
            _ => unreachable!(),
        }
        let altered = f.execution.with_command_for_test(command);
        assert!(
            ExecutionOwner::new(&altered, "a".repeat(32)).is_err(),
            "{field}"
        );
    }
}
