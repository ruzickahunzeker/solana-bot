use super::*;
fn a(byte: u8) -> Address {
    Address::from_bytes([byte; 32])
}

#[test]
fn preparation_balance_snapshot_checks_wallet_mint_and_integer_units() {
    let value = PreparationBalanceSnapshot::fixture(a(1), a(9), 1000, Instant::now());
    assert_eq!(value.checked_units(a(1), a(9)).unwrap(), 1000);
    assert!(value.checked_units(a(2), a(9)).is_err());
    assert!(value.checked_units(a(1), a(8)).is_err());
}
#[test]
fn preparation_balance_snapshot_expired_or_future_observation_is_rejected() {
    for started in [
        Instant::now() - Duration::from_secs(4),
        Instant::now() + Duration::from_secs(1),
    ] {
        let value = PreparationBalanceSnapshot::fixture(a(1), a(9), 1000, started);
        let error = value.checked_units(a(1), a(9)).unwrap_err();
        assert!(!error
            .downcast_ref::<PreparationBalanceFailure>()
            .unwrap()
            .allows_lane_fallback());
    }
}
#[test]
fn preparation_balance_snapshot_requires_slot_and_metadata_for_positive_units() {
    for (units, program, decimals, slot, valid) in [
        (1, None, Some(6), 42, false),
        (1, Some(a(2)), None, 42, false),
        (0, None, None, 0, false),
        (0, None, None, 42, true),
        (1, Some(a(2)), Some(6), 42, true),
    ] {
        assert_eq!(
            PreparationBalanceSnapshot::from_metadata(
                a(1),
                a(9),
                units,
                program,
                decimals,
                slot,
                Instant::now()
            )
            .is_ok(),
            valid
        );
    }
}
#[test]
fn preparation_balance_only_unavailable_errors_allow_preparation_lane_fallback() {
    for kind in [
        BalanceFailureKind::Invalid,
        BalanceFailureKind::Expired,
        BalanceFailureKind::Unavailable,
    ] {
        let error = anyhow::Error::new(PreparationBalanceFailure::new(kind, "fixture"))
            .context("outer preparation context");
        assert_eq!(
            error
                .downcast_ref::<PreparationBalanceFailure>()
                .unwrap()
                .allows_lane_fallback(),
            kind == BalanceFailureKind::Unavailable
        );
    }
}

#[test]
fn preparation_balance_raw_input_requires_account_identity_and_preserves_precision() {
    let input = PreparationInput {
        account: a(3),
        program: a(2),
        decimals: 9,
    };
    let value = PreparationBalanceSnapshot::from_account(
        a(1),
        a(9),
        9007199254740993,
        input,
        42,
        Instant::now(),
    )
    .unwrap();
    assert_eq!(value.checked_input(a(1), a(9)).unwrap(), input);
    assert_eq!(value.checked_units(a(1), a(9)).unwrap(), 9007199254740993);
    assert!(value.checked_input(a(2), a(9)).is_err());
    assert!(value.checked_input(a(1), a(8)).is_err());
    let metadata = PreparationBalanceSnapshot::fixture(a(1), a(9), 1000, Instant::now());
    assert!(metadata.checked_input(a(1), a(9)).is_err());
}

#[test]
fn preparation_balance_raw_input_access_does_not_bypass_freshness() {
    let mut value = PreparationBalanceSnapshot::fixture(
        a(1),
        a(9),
        1000,
        Instant::now() - Duration::from_secs(4),
    );
    value.input_account = Some(a(3));
    assert!(value.checked_input(a(1), a(9)).is_err());
    assert!(PreparationBalanceSnapshot::from_account(
        a(1),
        a(9),
        1000,
        PreparationInput {
            account: a(3),
            program: a(2),
            decimals: 6
        },
        0,
        Instant::now()
    )
    .is_err());
}
