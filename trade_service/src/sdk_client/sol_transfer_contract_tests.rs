use super::calculate_sol_transfer_amount;
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn legacy_sol_transfer_amount_matrix_before_extraction() {
    let mut rows = Vec::new();
    for balance in [
        0,
        4999,
        5000,
        5001,
        1_000_000,
        9_007_199_254_740_993,
        u64::MAX,
    ] {
        for fee in [0, 5000, u64::MAX] {
            for rent in [0, 890_880, u64::MAX] {
                for fixed in [
                    None,
                    Some(0),
                    Some(1),
                    Some(94_120),
                    Some(9_007_199_254_740_993),
                    Some(u64::MAX),
                ] {
                    for percent in [None, Some(0), Some(50), Some(100), Some(u64::MAX)] {
                        let result =
                            match calculate_sol_transfer_amount(balance, fee, rent, fixed, percent)
                            {
                                Ok((amount, reserve)) => json!({"amount":amount,"reserve":reserve}),
                                Err(error) => json!({"error":error.to_string()}),
                            };
                        rows.push(json!({"balance":balance,"fee":fee,"rent":rent,"fixed":fixed,"percent":percent,"result":result}));
                    }
                }
            }
        }
    }
    assert_eq!(rows.len(), 1890);
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&rows).unwrap()));
    assert_eq!(
        fingerprint,
        "18bdd66c466fbf01c6b72ca4f1c1189c3126fbccc58aed53ae5fcaff7162660b"
    );
}

#[test]
fn legacy_sol_transfer_fee_rent_and_error_priority_examples() {
    assert_eq!(
        calculate_sol_transfer_amount(1_000_000, 5000, 890_880, None, Some(50)).unwrap(),
        (47_060, 900_880)
    );
    assert_eq!(
        calculate_sol_transfer_amount(1_000_000, 5000, u64::MAX, None, Some(100)).unwrap(),
        (995_000, 0)
    );
    assert_eq!(
        calculate_sol_transfer_amount(4999, 5000, 0, None, Some(100))
            .unwrap_err()
            .to_string(),
        "insufficient SOL balance for network fee"
    );
    assert_eq!(
        calculate_sol_transfer_amount(5000, 5000, 0, None, Some(100))
            .unwrap_err()
            .to_string(),
        "insufficient SOL balance after network fee"
    );
    assert_eq!(calculate_sol_transfer_amount(0, 0, 0, None, Some(75)).unwrap_err().to_string(), "insufficient SOL balance for fee and rent reserve: balance=0, fee=0, reserve=10000 lamports");
    assert_eq!(
        calculate_sol_transfer_amount(1_000_000, 5000, 890_880, Some(94_121), None)
            .unwrap_err()
            .to_string(),
        "insufficient SOL balance: spendable=94120 lamports, requested=94121 lamports"
    );
}
