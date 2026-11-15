use super::*;

fn post_token(case: &mut Case, position: usize, amount: u64, lamports: u64) {
    let key = keys(&case.plan)[position];
    let i = index(&case.plan, key).unwrap();
    change_token(&mut case.result["accounts"][position], |a| {
        a.amount = amount
    });
    case.result["accounts"][position]["lamports"] = json!(lamports);
    case.result["postBalances"][i] = json!(lamports);
    let row = case.result["postTokenBalances"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["accountIndex"] == i)
        .unwrap();
    row["uiTokenAmount"]["amount"] = json!(amount.to_string());
}

fn refund_rent(case: &mut Case, value: u64) {
    let wallet = index(&case.plan, case.plan.accounts.wallet).unwrap();
    let after = case.result["accounts"][8]["lamports"].as_u64().unwrap() + value;
    case.result["accounts"][8]["lamports"] = json!(after);
    case.result["postBalances"][wallet] = json!(after);
}

fn prefund(case: &mut Case, position: usize, lamports: u64) {
    assert!(case.source.rpc.values[position].is_null());
    let key = keys(&case.plan)[position];
    let i = index(&case.plan, key).unwrap();
    let mint = if position == 6 {
        case.plan.accounts.input_mint
    } else {
        case.plan.accounts.output_mint
    };
    case.source.rpc.values[position] = ui(solana_system_interface::program::id(), vec![]);
    case.source.rpc.values[position]["lamports"] = json!(lamports);
    case.result["preBalances"][i] = json!(lamports);
    let native = mint == WSOL_TOKEN_ACCOUNT;
    let inherited = if native {
        lamports.saturating_sub(RESERVE)
    } else {
        0
    };
    let output = if position == 7 { case.net_output } else { 0 };
    post_token(
        case,
        position,
        inherited + output,
        lamports.max(RESERVE) + if native { output } else { 0 },
    );
    refund_rent(case, lamports.min(RESERVE));
}

#[tokio::test]
async fn damm_buy_quote_unsynchronized_input_is_accounted_only_from_the_simulated_bank() {
    for reversed in [false, true] {
        let mut case = Case::new(reversed, true, false, true, true, Encoding::Parsed);
        // The earlier account bank has a different surplus, never used to derive output.
        case.source.rpc.values[6]["lamports"] = json!(RESERVE + 1000 + 999);
        let input = index(&case.plan, case.plan.accounts.input_ata).unwrap();
        case.result["preBalances"][input] = json!(RESERVE + 1500 + 300);
        post_token(&mut case, 6, 1800, RESERVE + 1800);
        let quote = case.run().await.unwrap();
        assert_eq!(quote.output_units, 120);
        assert_eq!(quote.minimum(&case.plan, 1500).unwrap(), 102);
        assert_eq!(
            case.source.rpc.methods(),
            [
                RpcRequest::GetMultipleAccounts,
                RpcRequest::SimulateTransaction
            ]
        );
    }
}

#[tokio::test]
async fn damm_buy_quote_sync_cannot_hide_under_backing_missing_credit_or_a_stale_post_amount() {
    for mutation in 0..5 {
        let mut case = Case::new(false, true, false, true, true, Encoding::Parsed);
        let input = index(&case.plan, case.plan.accounts.input_ata).unwrap();
        case.result["preBalances"][input] = json!(RESERVE + 1800);
        post_token(&mut case, 6, 1800, RESERVE + 1800);
        match mutation {
            0 => case.result["preBalances"][input] = json!(RESERVE + 1499),
            1 => case.result["preTokenBalances"][0]["uiTokenAmount"]["amount"] = json!("1801"),
            2 => post_token(&mut case, 6, 1500, RESERVE + 1800),
            3 => post_token(&mut case, 6, 1801, RESERVE + 1801),
            _ => {
                let row = case.result["preTokenBalances"][0].clone();
                case.result["preTokenBalances"]
                    .as_array_mut()
                    .unwrap()
                    .push(row);
            }
        }
        assert!(case.run().await.is_err(), "mutation={mutation}");
    }
}

#[tokio::test]
async fn damm_buy_quote_prefunded_native_atas_charge_only_the_actual_rent_shortfall() {
    let mut count = 0;
    for reversed in [false, true] {
        for input in [1, RESERVE / 2, RESERVE, RESERVE + 1000] {
            for output in [1, RESERVE / 2, RESERVE, RESERVE + 2000] {
                let mut case = Case::new(reversed, true, false, false, false, Encoding::Parsed);
                prefund(&mut case, 6, input);
                prefund(&mut case, 7, output);
                let quote = case.run().await.unwrap();
                assert_eq!(quote.output_units, 120);
                assert_eq!(quote.probe_fee_lamports, FEE);
                count += 1;
            }
        }
    }
    assert_eq!(count, 32);
}

#[tokio::test]
async fn damm_buy_quote_usdc_and_token2022_outputs_support_prefunded_system_atas() {
    for reversed in [false, true] {
        for extended in [false, true] {
            for lamports in [1, RESERVE / 2, RESERVE, RESERVE + 2000] {
                let mut case = Case::new(reversed, false, extended, true, false, Encoding::Partial);
                prefund(&mut case, 7, lamports);
                assert_eq!(
                    case.run().await.unwrap().output_units,
                    if extended { 118 } else { 120 }
                );
            }
        }
    }
}

#[tokio::test]
async fn damm_buy_quote_prefunding_cannot_be_used_as_usdc_input_balance() {
    let mut case = Case::new(false, false, false, true, true, Encoding::Parsed);
    case.source.rpc.values[6] = ui(solana_system_interface::program::id(), vec![]);
    case.source.rpc.values[6]["lamports"] = json!(10_000_000);
    assert!(case.run().await.is_err());
    assert_eq!(case.source.rpc.methods(), [RpcRequest::GetMultipleAccounts]);
}

#[tokio::test]
async fn damm_buy_quote_prefunded_account_owner_data_and_simulated_rent_must_still_match() {
    for mutation in 0..6 {
        let mut case = Case::default();
        prefund(&mut case, 6, RESERVE / 2);
        match mutation {
            0 => case.source.rpc.values[6]["owner"] = json!(key(99).to_string()),
            1 => case.source.rpc.values[6] = ui(solana_system_interface::program::id(), vec![0]),
            2 => case.source.rpc.values[6]["executable"] = json!(true),
            3 => case.source.rpc.values[6]["space"] = json!(1),
            4 => {
                let input = index(&case.plan, case.plan.accounts.input_ata).unwrap();
                case.result["preBalances"][input] = json!(RESERVE / 2 + 1);
            }
            _ => refund_rent(&mut case, 1),
        }
        assert!(case.run().await.is_err(), "mutation={mutation}");
    }
}

#[tokio::test]
async fn damm_buy_quote_prefunded_credits_are_not_derived_from_an_earlier_snapshot() {
    let mut case = Case::default();
    prefund(&mut case, 6, RESERVE + 1000);
    case.source.rpc.values[6]["lamports"] = json!(RESERVE + 9000);
    assert_eq!(case.run().await.unwrap().output_units, 120);
}

#[tokio::test]
async fn damm_buy_quote_usdc_to_wsol_accepts_existing_and_prefunded_native_output_without_wrapping_input(
) {
    let mut count = 0;
    for reversed in [false, true] {
        for exists in [false, true] {
            for prefunding in [0, 1, RESERVE / 2, RESERVE, RESERVE + 1000] {
                if exists && prefunding > 0 {
                    continue;
                }
                let mut case = Case::with_output(
                    reversed,
                    false,
                    false,
                    true,
                    (exists, WSOL_TOKEN_ACCOUNT),
                    Encoding::Parsed,
                );
                if !exists && prefunding > 0 {
                    prefund(&mut case, 7, prefunding);
                }
                let quote = case.run().await.unwrap();
                assert_eq!(quote.output_units, 120);
                assert_eq!(quote.minimum(&case.plan, 1500).unwrap(), 102);
                assert_eq!(case.plan.funding, Funding::ExistingUsdc);
                assert_eq!(case.plan.transaction.message.instructions.len(), 3);
                count += 1;
            }
        }
    }
    assert_eq!(count, 12);
}

#[tokio::test]
async fn damm_buy_quote_native_output_preserves_unsynchronized_lamports_without_inventing_tokens() {
    let mut case = Case::with_output(
        false,
        false,
        false,
        true,
        (true, WSOL_TOKEN_ACCOUNT),
        Encoding::Parsed,
    );
    case.source.rpc.values[7]["lamports"] = json!(RESERVE + 2000 + 999);
    let output = index(&case.plan, case.plan.accounts.output_ata).unwrap();
    case.result["preBalances"][output] = json!(RESERVE + 2500 + 300);
    post_token(&mut case, 7, 2620, RESERVE + 2620 + 300);
    assert_eq!(case.run().await.unwrap().output_units, 120);
    // No sync_native on output: those 300 lamports cannot become quoted swap output.
    post_token(&mut case, 7, 2920, RESERVE + 2620 + 300);
    assert!(case.run().await.is_err());
}
