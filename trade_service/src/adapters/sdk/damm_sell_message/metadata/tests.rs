use super::*;

#[test]
fn sdk_candidate_metadata_round_trips_formal_and_simulation_message_matrix_without_signatures() {
    let mut count = 0;
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        for simulate in [false, true] {
            for percent in [1, 50, 100] {
                for quote in [WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
                    for reversed in [false, true] {
                        for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                            let f = super::super::tests::case(
                                provider, simulate, percent, quote, reversed, program,
                            );
                            let destination = (!simulate && provider != SwqosType::Default)
                                .then(|| candidate::tip_accounts(provider)[0]);
                            let value = DammSellCandidate::build(f.profile().unwrap(), destination)
                                .unwrap();
                            let message = value.message_bytes();
                            let json = value.metadata().encode(&message).unwrap();
                            let decoded = SdkPreparedRecord::decode(&json, &message).unwrap();
                            validate_candidate(&decoded, &message).unwrap();
                            assert_eq!(&decoded, value.metadata());
                            assert!(value
                                .transaction()
                                .signatures
                                .iter()
                                .all(|s| *s == solana_sdk::signature::Signature::default()));
                            assert_eq!(decoded.sell().unwrap().details().balance_slot, 123);
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 96);
}

#[test]
fn sdk_candidate_metadata_reconstructs_accounts_amounts_fees_and_fixed_tip_recipient_not_just_hash()
{
    let value = candidate_fixture(SwqosType::Temporal, false, 100);
    let message = value.message_bytes();
    for field in [
        "wallet",
        "mint",
        "quote",
        "pool",
        "vault",
        "mint_a",
        "program",
        "referral",
        "sysvar",
        "input",
        "input_program",
        "amount",
        "minimum",
        "percent",
        "tip",
        "tip_destination",
        "relay",
        "hash",
        "simulate",
    ] {
        let mut d = value.metadata().sell().unwrap().details().clone();
        match field {
            "wallet" => d.wallet = [30; 32],
            "mint" => d.mint = [30; 32],
            "quote" => d.quote_mint = [30; 32],
            "pool" => {
                d.pool.pool = [30; 32];
                d.pool_hint = Some([30; 32]);
            }
            "vault" => d.pool.token_a_vault = [30; 32],
            "mint_a" => d.pool.token_a_mint = [30; 32],
            "program" => d.pool.token_a_program = [30; 32],
            "referral" => d.pool.referral = Some([30; 32]),
            "sysvar" => d.pool.rate_limiter_sysvar = true,
            "input" => d.input_account = [30; 32],
            "input_program" => d.input_program = [30; 32],
            "amount" => {
                d.balance_units += 1;
                d.input_units += 1;
            }
            "minimum" => d.minimum_output_units += 1,
            "percent" => {
                d.sell_percent = 50;
                d.input_units /= 2;
            }
            "tip" => d.tip_sol_bits = 0.002_f64.to_bits(),
            "tip_destination" => {
                d.tip_destination = Some(candidate::tip_accounts(SwqosType::Temporal)[1].to_bytes())
            }
            "relay" => d.relay = "unrecognized-provider".into(),
            "hash" => d.recent_blockhash = [30; 32],
            "simulate" => d.simulate = true,
            _ => unreachable!(),
        }
        let modified = SdkCandidateMetadata::damm_sell(d, &message).unwrap().into();
        assert!(validate_candidate(&modified, &message).is_err(), "{field}");
    }
}

#[test]
fn sdk_candidate_metadata_rejects_unknown_missing_versions_nested_fields_and_large_payloads() {
    let c = candidate_fixture(SwqosType::Default, false, 100);
    let message = c.message_bytes();
    let json = c.metadata().encode(&message).unwrap();
    for field in [
        "version",
        "profile",
        "unknown",
        "nested_unknown",
        "missing_required",
    ] {
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        match field {
            "version" => value["version"] = serde_json::json!(2),
            "profile" => value["profile"] = serde_json::json!("new-unsupported-profile"),
            "unknown" => value["future_authorization"] = serde_json::json!(true),
            "nested_unknown" => value["details"]["pool"]["future_fee"] = serde_json::json!(1),
            "missing_required" => {
                value["details"]
                    .as_object_mut()
                    .unwrap()
                    .remove("balance_slot");
            }
            _ => unreachable!(),
        }
        assert!(
            SdkCandidateMetadata::decode(&serde_json::to_string(&value).unwrap(), &message)
                .is_err(),
            "{field}"
        );
    }
    assert!(SdkCandidateMetadata::decode(&" ".repeat(65_537), &message).is_err());
    let mut changed = message;
    changed[0] ^= 1;
    assert!(SdkCandidateMetadata::decode(&json, &changed).is_err());
}

#[test]
fn sdk_candidate_metadata_rejects_zero_output_overflow_bad_balance_and_nonfinite_tip() {
    let c = candidate_fixture(SwqosType::Default, false, 100);
    for field in [
        "zero_minimum",
        "zero_input",
        "slot",
        "overflow",
        "percentage",
        "nan",
        "infinity",
        "negative",
        "pool_hint",
    ] {
        let mut d = c.metadata().sell().unwrap().details().clone();
        match field {
            "zero_minimum" => d.minimum_output_units = 0,
            "zero_input" => d.input_units = 0,
            "slot" => d.balance_slot = 0,
            "overflow" => d.balance_units = u64::MAX,
            "percentage" => d.sell_percent = 101,
            "nan" => d.tip_sol_bits = f64::NAN.to_bits(),
            "infinity" => d.tip_sol_bits = f64::INFINITY.to_bits(),
            "negative" => d.tip_sol_bits = (-1_f64).to_bits(),
            "pool_hint" => d.pool_hint = Some([30; 32]),
            _ => unreachable!(),
        }
        assert!(
            SdkCandidateMetadata::damm_sell(d, &c.message_bytes()).is_err(),
            "{field}"
        );
    }
}

#[test]
fn sdk_candidate_metadata_preserves_raw_units_above_f64_precision_and_does_not_refresh_balance() {
    let mut f = super::super::tests::fixture(SwqosType::Default, false, 1);
    let units = 9_007_199_254_740_993;
    let wallet = Address::from_bytes(f.context.wallet.to_bytes());
    let mint = Address::from_bytes(f.intent.mint.to_bytes());
    let input = f.holding.checked_input(wallet, mint).unwrap();
    f.holding = Arc::new(
        PreparationBalanceSnapshot::from_account(
            wallet,
            mint,
            units,
            input,
            123,
            std::time::Instant::now(),
        )
        .unwrap(),
    );
    f.balance = units;
    f.quote = crate::adapters::sdk::damm_probe::DammOutputEvidence::fixture_for_profile(
        f.context.wallet,
        &f.intent,
        &f.request,
        f.balance,
        f.percent,
        &f.holding,
    );
    let value = DammSellCandidate::build(f.profile().unwrap(), None).unwrap();
    let json = value.metadata().encode(&value.message_bytes()).unwrap();
    assert!(json.contains(&units.to_string()));
    assert_eq!(
        SdkCandidateMetadata::decode(&json, &value.message_bytes())
            .unwrap()
            .details()
            .balance_units,
        units
    );
    assert!(PreparationBalanceSnapshot::from_account(
        wallet,
        mint,
        units,
        input,
        123,
        std::time::Instant::now() - std::time::Duration::from_secs(4)
    )
    .is_err());
}

#[test]
fn sdk_candidate_relay_names_round_trip_every_existing_provider() {
    for provider in [
        SwqosType::Default,
        SwqosType::Jito,
        SwqosType::NextBlock,
        SwqosType::ZeroSlot,
        SwqosType::Temporal,
        SwqosType::Bloxroute,
        SwqosType::Node1,
        SwqosType::FlashBlock,
        SwqosType::BlockRazor,
        SwqosType::Astralane,
        SwqosType::Stellium,
        SwqosType::Lightspeed,
        SwqosType::Soyas,
        SwqosType::Speedlanding,
        SwqosType::Helius,
        SwqosType::Solami,
        SwqosType::LunarLander,
        SwqosType::Glaive,
    ] {
        assert_eq!(relay(relay_name(provider)).unwrap(), provider);
    }
    assert!(relay("auto").is_err());
}
