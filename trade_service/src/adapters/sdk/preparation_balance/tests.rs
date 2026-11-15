use super::*;
use crate::domain::trade::{USD1_MINT, USDC_MINT};
use solana_sdk::hash::Hash;
use std::time::{Duration, Instant};

fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}
fn addr(byte: u8) -> Address {
    Address::from_bytes([byte; 32])
}
fn trade(pool: Pool, quote: Pubkey) -> ExecuteTrade {
    ExecuteTrade {
        pool,
        mint: key(7),
        quote_mint: quote,
        pool_id: Some(key(8)),
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(1000),
        sell_percent: Some(100),
        slippage_bps: 1500,
        tip_sol: 0.0001,
        recent_blockhash: Hash::new_from_array([9; 32]),
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 17,
        simulate: false,
        use_mev: None,
        mev_provider: None,
    }
}
fn pools() -> [Pool; 9] {
    [
        Pool::Pump,
        Pool::PumpAmm,
        Pool::Bonk,
        Pool::StonkFun,
        Pool::StonkFunSwap,
        Pool::MeteoraDammV2,
        Pool::MeteoraDbc,
        Pool::RaydiumAmmV4,
        Pool::RaydiumCpmm,
    ]
}

#[test]
fn preparation_balance_gate_matches_existing_protocol_quote_hop_and_mode_conditions() {
    for pool in pools() {
        for quote in [
            WSOL_MINT.parse().unwrap(),
            USDC_MINT.parse().unwrap(),
            USD1_MINT.parse().unwrap(),
            key(90),
        ] {
            for hop in [false, true] {
                for simulate in [false, true] {
                    for side in [Side::Buy, Side::Sell] {
                        let mut t = trade(pool, quote);
                        t.simulate = simulate;
                        if hop {
                            t.sol_hop_pool = Some(Pool::RaydiumCpmm);
                            t.sol_hop_pool_id = Some(key(91));
                        }
                        let expected = if side == Side::Sell {
                            Some(t.mint)
                        } else if matches!(
                            pool,
                            Pool::PumpAmm | Pool::StonkFun | Pool::StonkFunSwap | Pool::RaydiumCpmm
                        ) && quote != WSOL_MINT.parse::<Pubkey>().unwrap()
                            && !hop
                        {
                            Some(quote)
                        } else {
                            None
                        };
                        assert_eq!(balance_mint(&t, side), expected);
                        let snapshot = expected.map(|mint| {
                            PreparationBalanceSnapshot::fixture(
                                addr(1),
                                Address::from_bytes(mint.to_bytes()),
                                1_000_000,
                                Instant::now(),
                            )
                        });
                        validate(snapshot.as_ref(), addr(1), &t, side).unwrap();
                        if expected.is_some() {
                            assert!(validate(None, addr(1), &t, side).is_err());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn preparation_balance_gate_rejects_wrong_identity_missing_or_unexpected_snapshot() {
    let t = trade(Pool::RaydiumCpmm, key(90));
    for (wallet, mint) in [
        (addr(2), Address::from_bytes(key(90).to_bytes())),
        (addr(1), addr(99)),
    ] {
        let snapshot = PreparationBalanceSnapshot::fixture(wallet, mint, 1000, Instant::now());
        assert!(validate(Some(&snapshot), addr(1), &t, Side::Buy).is_err());
    }
    let no_balance = trade(Pool::Pump, WSOL_MINT.parse().unwrap());
    let snapshot = PreparationBalanceSnapshot::fixture(addr(1), addr(7), 1000, Instant::now());
    assert!(validate(Some(&snapshot), addr(1), &no_balance, Side::Buy).is_err());
}

#[test]
fn preparation_balance_gate_rejects_insufficient_zero_and_missing_buy_inputs() {
    let mut t = trade(Pool::RaydiumCpmm, key(90));
    let snapshot = PreparationBalanceSnapshot::fixture(
        addr(1),
        Address::from_bytes(t.quote_mint.to_bytes()),
        999,
        Instant::now(),
    );
    assert!(validate(Some(&snapshot), addr(1), &t, Side::Buy).is_err());
    for amount in [None, Some(0)] {
        t.amount_lamports = amount;
        assert!(validate(Some(&snapshot), addr(1), &t, Side::Buy).is_err());
    }
}

#[test]
fn preparation_balance_gate_stops_sdk_percent_overflow_and_dust_for_all_protocols() {
    for pool in pools() {
        for (units, percent) in [
            (u64::MAX, Some(100)),
            (u64::MAX, Some(2)),
            (1, Some(25)),
            (1000, Some(0)),
            (1000, Some(101)),
            (1000, None),
        ] {
            let mut t = trade(pool, WSOL_MINT.parse().unwrap());
            t.sell_percent = percent;
            let snapshot =
                PreparationBalanceSnapshot::fixture(addr(1), addr(7), units, Instant::now());
            assert!(validate(Some(&snapshot), addr(1), &t, Side::Sell).is_err());
        }
    }
}

#[test]
fn preparation_balance_gate_preserves_valid_integer_boundary_without_sdk_multiplication() {
    for percent in [1, 25, 99, 100] {
        let mut t = trade(Pool::Pump, WSOL_MINT.parse().unwrap());
        t.sell_percent = Some(percent);
        let snapshot = PreparationBalanceSnapshot::fixture(
            addr(1),
            addr(7),
            u64::MAX / percent,
            Instant::now(),
        );
        validate(Some(&snapshot), addr(1), &t, Side::Sell).unwrap();
    }
}

#[test]
fn preparation_balance_gate_rechecks_age_without_refetch_or_renewal() {
    let t = trade(Pool::Pump, WSOL_MINT.parse().unwrap());
    let snapshot = PreparationBalanceSnapshot::fixture(
        addr(1),
        addr(7),
        1000,
        Instant::now() - Duration::from_secs(4),
    );
    assert!(validate(Some(&snapshot), addr(1), &t, Side::Sell).is_err());
}

#[test]
fn preparation_balance_gate_seed_mode_is_explicitly_unaudited() {
    require_canonical_mode(false).unwrap();
    assert!(require_canonical_mode(true).is_err());
}
