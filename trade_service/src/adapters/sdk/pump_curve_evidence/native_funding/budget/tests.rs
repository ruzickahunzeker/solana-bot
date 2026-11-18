use super::*;
use crate::adapters::sdk::pump_curve_evidence::native_funding::tests::{decode, fixture};

fn units(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

fn ceilings(rent: Option<u64>) -> CostCeilings {
    CostCeilings {
        output_ata_rent: rent.map(units),
        network_fee: units(20_000),
        relay_tip: 10_000,
        other_instruction_debits: 500,
    }
}

#[test]
fn native_buy_budget_funds_missing_ata_and_distinguishes_existing_ata() {
    let (plan, mut values) = fixture(Side::Buy, spl_token_interface::ID);
    let buy = NativeIntentAmount::BuyLamports(units(1_000_000));
    let existing = decode(&plan, &values).unwrap();
    assert_eq!(
        existing.check_budget(buy, ceilings(None)).unwrap(),
        NativeBudget {
            input_units: units(1_000_000),
            native_debit_ceiling: 1_030_500,
            native_remaining_at_ceiling: 18_969_500,
        }
    );
    assert!(existing
        .check_budget(buy, ceilings(Some(2_000_000)))
        .is_err());
    values[5] = None;
    let missing = decode(&plan, &values).unwrap();
    assert!(missing.check_budget(buy, ceilings(None)).is_err());
    assert_eq!(
        missing
            .check_budget(buy, ceilings(Some(2_000_000)))
            .unwrap(),
        NativeBudget {
            input_units: units(1_000_000),
            native_debit_ceiling: 3_030_500,
            native_remaining_at_ceiling: 16_969_500,
        }
    );
    assert!(missing
        .check_budget(NativeIntentAmount::SellPercent(50), ceilings(None))
        .is_err());
}

#[test]
fn native_sell_budget_keeps_ordinary_floor_and_never_credits_proceeds_or_rent() {
    let (plan, mut values) = fixture(Side::Sell, spl_token_interface::ID);
    let snapshot = decode(&plan, &values).unwrap();
    for (percent, expected) in [(5, 1), (50, 10), (100, 21)] {
        assert_eq!(
            snapshot
                .check_budget(NativeIntentAmount::SellPercent(percent), ceilings(None))
                .unwrap(),
            NativeBudget {
                input_units: units(expected),
                native_debit_ceiling: 30_500,
                native_remaining_at_ceiling: 19_969_500,
            }
        );
    }
    for percent in [0, 1, 101] {
        assert!(snapshot
            .check_budget(NativeIntentAmount::SellPercent(percent), ceilings(None))
            .is_err());
    }
    assert!(snapshot
        .check_budget(NativeIntentAmount::BuyLamports(units(1)), ceilings(None))
        .is_err());
    assert!(snapshot
        .check_budget(NativeIntentAmount::SellPercent(100), ceilings(Some(1)))
        .is_err());
    values[6].as_mut().unwrap().lamports = 30_499;
    let short = decode(&plan, &values).unwrap();
    assert!(short
        .check_budget(NativeIntentAmount::SellPercent(100), ceilings(None))
        .unwrap_err()
        .to_string()
        .contains("cannot fund"));
}

#[test]
fn native_budget_checks_exact_balance_and_integer_overflow() {
    let (buy_plan, mut values) = fixture(Side::Buy, spl_token_interface::ID);
    values[6].as_mut().unwrap().lamports = 1_030_500;
    let snapshot = decode(&buy_plan, &values).unwrap();
    assert_eq!(
        snapshot
            .check_budget(
                NativeIntentAmount::BuyLamports(units(1_000_000)),
                ceilings(None)
            )
            .unwrap()
            .native_remaining_at_ceiling,
        0
    );
    assert!(snapshot
        .check_budget(
            NativeIntentAmount::BuyLamports(units(1_000_001)),
            ceilings(None)
        )
        .is_err());
    assert!(snapshot
        .check_budget(
            NativeIntentAmount::BuyLamports(units(u64::MAX)),
            ceilings(None)
        )
        .unwrap_err()
        .to_string()
        .contains("overflows"));

    let (sell_plan, values) = fixture(Side::Sell, spl_token_interface::ID);
    let mut large = decode(&sell_plan, &values).unwrap();
    large.user_base_units = Some(u64::MAX);
    assert!(large
        .check_budget(NativeIntentAmount::SellPercent(100), ceilings(None))
        .unwrap_err()
        .to_string()
        .contains("sell amount overflows"));
}
