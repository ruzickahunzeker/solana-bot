use super::super::tests::profile;
use super::*;
use crate::domain::sdk_candidate::SdkCandidateMetadata;
use serde_json::json;

async fn candidate() -> DammBuyCandidate {
    let p = profile(SwqosType::Temporal, false, true, false).await;
    p.prepare(Some(
        crate::adapters::sdk::damm_instructions::tip_accounts(SwqosType::Temporal)[0],
    ))
    .unwrap()
}

#[tokio::test]
async fn damm_buy_record_round_trips_real_candidates_for_both_assets_modes_and_ata_states() {
    let mut count = 0;
    for native in [false, true] {
        for simulate in [false, true] {
            for exists in [false, true] {
                for provider in [SwqosType::Default, SwqosType::Temporal] {
                    let p = profile(provider, simulate, native, exists).await;
                    let destination = (!simulate && provider != SwqosType::Default).then(|| {
                        crate::adapters::sdk::damm_instructions::tip_accounts(provider)[0]
                    });
                    let c = p.prepare(destination).unwrap();
                    let message = c.transaction.message.serialize();
                    let encoded = c.record.encode(&message).unwrap();
                    let decoded = DammBuyCandidateRecord::decode(&encoded, &message).unwrap();
                    assert_eq!(&decoded, c.record.buy().unwrap());
                    let common = crate::domain::sdk_prepared_record::SdkPreparedRecord::decode(
                        &encoded, &message,
                    )
                    .unwrap();
                    assert!(common.sell().is_err());
                    assert_eq!(common.encode(&message).unwrap(), encoded);
                    validate_record(&decoded, &message).unwrap();
                    c.validate_full(&c.instructions).unwrap();
                    assert!(SdkCandidateMetadata::decode(&encoded, &message).is_err());
                    assert_eq!(
                        c.transaction.signatures,
                        [solana_sdk::signature::Signature::default()]
                    );
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 16);
}

#[tokio::test]
async fn damm_buy_record_captures_only_actual_same_bank_funding_and_raw_asset_units() {
    for native in [false, true] {
        let p = profile(SwqosType::Default, false, native, false).await;
        let c = p.prepare(None).unwrap();
        let d = c.record.buy().unwrap().details();
        assert_eq!(d.input_units, 100);
        assert_eq!(
            d.funding,
            if native {
                BuyFunding::NativeSol
            } else {
                BuyFunding::ExistingQuote
            }
        );
        assert_eq!(d.input_decimals, if native { 9 } else { 6 });
        assert_eq!(d.output_decimals, 6);
        assert_eq!(d.native.before_lamports, 100_000_000);
        assert_eq!(d.native.deposit_lamports, if native { 100 } else { 0 });
        assert_eq!(d.native.base_fee_lamports, 5_000);
        assert_eq!(d.native.rent_lamports, 2_039_280);
        assert_eq!(
            d.native.remaining_lamports,
            d.native.before_lamports
                - d.native.deposit_lamports
                - d.native.rent_lamports
                - d.native.base_fee_lamports
        );
        assert_eq!(d.priority_fee_lamports, 300);
        assert_eq!(d.tip_lamports, 0);
        assert_eq!((d.account_slot, d.simulation_slot), (42, 43));
        assert_eq!(
            (d.quoted_net_output_units, d.minimum_output_units),
            (120, 102)
        );
        let value = serde_json::to_value(c.record.buy().unwrap()).unwrap();
        assert!(value["details"].get("sell_percent").is_none());
        assert!(value["details"].get("balance_units").is_none());
    }
}

#[tokio::test]
async fn damm_buy_record_usdc_to_wsol_binds_canonical_output_program_and_decimals_without_wrapping_input(
) {
    let (plan, evidence) =
        super::super::super::super::evidence::tests::native_output_record_fixture(false).await;
    let intent = super::super::super::tests::intent(&plan, false);
    let context = DammBuyContext {
        wallet: plan.accounts.wallet,
        provider: SwqosType::Default,
        version: TradeTransactionVersion::V0,
        seed_optimized: false,
    };
    let minimum = evidence.minimum(&plan, intent.slippage_bps).unwrap();
    let request = crate::adapters::sdk::ordinary_params::ordinary_buy_params(
        &intent,
        DexParamEnum::MeteoraDammV2(plan.accounts.pool.clone()),
        TradeTokenType::USDC,
        100,
        false,
        context.provider,
        Some(minimum),
    )
    .unwrap();
    let quote = DammBuyQuote::new(context, &intent, plan, evidence).unwrap();
    let p = DammBuyMessage::new(Arc::new(quote), context, &intent, &request).unwrap();
    let c = p.prepare(None).unwrap();
    let message = c.transaction.message.serialize();
    validate_record(c.record.buy().unwrap(), &message).unwrap();
    let d = c.record.buy().unwrap().details();
    assert_eq!(d.output_decimals, 9);
    assert_eq!(d.mint, WSOL_TOKEN_ACCOUNT.to_bytes());
    assert_eq!(d.native.deposit_lamports, 0);
    assert_eq!(d.funding, BuyFunding::ExistingQuote);
    for change in 0..2 {
        let mut d = d.clone();
        if change == 0 {
            d.output_decimals = 6;
        } else {
            d.output_program = spl_token_2022_interface::ID.to_bytes();
        }
        let record = DammBuyCandidateRecord::new(d, &message).unwrap();
        assert!(validate_record(&record, &message).is_err());
    }
}

#[tokio::test]
async fn damm_buy_record_reconstructs_all_transaction_accounts_amounts_fees_and_flags_not_only_digest(
) {
    let c = candidate().await;
    let message = c.transaction.message.serialize();
    for change in 0..23 {
        let mut d = c.record.buy().unwrap().details().clone();
        match change {
            0 => d.wallet = [99; 32],
            1 => d.mint = [99; 32],
            2 => d.quote_mint = [99; 32],
            3 => {
                d.pool.pool = [99; 32];
                d.pool_hint = Some([99; 32]);
            }
            4 => d.pool.token_a_vault = [99; 32],
            5 => d.pool.token_b_vault = [99; 32],
            6 => d.pool.token_a_mint = [99; 32],
            7 => d.pool.token_a_program = [99; 32],
            8 => d.pool.referral = Some([99; 32]),
            9 => d.pool.rate_limiter_sysvar = !d.pool.rate_limiter_sysvar,
            10 => d.input_account = [99; 32],
            11 => d.output_account = [99; 32],
            12 => d.input_program = [99; 32],
            13 => d.output_program = [99; 32],
            14 => d.input_decimals = 6,
            15 => {
                d.input_units += 1;
                d.native.deposit_lamports += 1;
                d.native.before_lamports += 1;
            }
            16 => {
                d.minimum_output_units = 103;
                d.quoted_net_output_units = 122;
            }
            17 => {
                d.create_output_account = false;
                d.output_initialized = false;
            }
            18 => {
                d.tip_destination = Some(
                    crate::adapters::sdk::damm_instructions::tip_accounts(SwqosType::Temporal)[1]
                        .to_bytes(),
                )
            }
            19 => {
                d.tip_sol_bits = 0.0002f64.to_bits();
                d.tip_lamports = 200_000;
            }
            20 => d.priority_fee_lamports = 301,
            21 => d.relay = "jito".into(),
            _ => d.recent_blockhash = [99; 32],
        }
        let rejected = match DammBuyCandidateRecord::new(d, &message) {
            Ok(record) => validate_record(&record, &message).is_err(),
            Err(_) => true,
        };
        assert!(rejected, "change {change}");
    }
}

#[tokio::test]
async fn damm_buy_record_rejects_inconsistent_native_math_slots_minimum_and_creation_evidence() {
    let c = candidate().await;
    let message = c.transaction.message.serialize();
    for change in 0..15 {
        let mut d = c.record.buy().unwrap().details().clone();
        match change {
            0 => d.native.before_lamports -= 1,
            1 => d.native.remaining_lamports += 1,
            2 => d.native.rent_lamports += 1,
            3 => d.native.base_fee_lamports += 1,
            4 => d.native.deposit_lamports = 0,
            5 => d.native.rent_lamports = u64::MAX,
            6 => d.priority_fee_lamports = u64::MAX,
            7 => d.tip_lamports = u64::MAX,
            8 => d.account_slot = 0,
            9 => d.simulation_slot = d.account_slot - 1,
            10 => d.quoted_net_output_units = 0,
            11 => d.minimum_output_units = 1,
            12 => d.slippage_bps = 10_000,
            13 => d.tip_sol_bits = f64::NAN.to_bits(),
            _ => d.create_output_account = false,
        }
        assert!(
            DammBuyCandidateRecord::new(d, &message).is_err(),
            "change {change}"
        );
    }
}

#[tokio::test]
async fn damm_buy_record_distinguishes_nominal_reconstruction_from_original_live_quote_evidence() {
    let mut c = candidate().await;
    let message = c.transaction.message.serialize();
    let mut d = c.record.buy().unwrap().details().clone();
    d.native.before_lamports += 1;
    d.native.remaining_lamports += 1;
    d.quoted_net_output_units += 1;
    // These claims do not alter the signed message. Only the original runtime quote can attest them.
    let claimed = DammBuyCandidateRecord::new(d, &message).unwrap();
    validate_record(&claimed, &message).unwrap();
    c.record = SdkPreparedRecord::DammBuy(claimed);
    let error = c.validate_full(&c.instructions).unwrap_err();
    assert!(error.to_string().contains("original live evidence"));
}

#[tokio::test]
async fn damm_buy_record_readback_does_not_renew_expired_quote_or_recreate_live_candidate() {
    let mut p = profile(SwqosType::Default, false, true, false).await;
    let c = p.prepare(None).unwrap();
    let message = c.transaction.message.serialize();
    let encoded = c.record.encode(&message).unwrap();
    drop(c);
    let profile = Arc::get_mut(&mut p).unwrap();
    Arc::get_mut(&mut profile.quote)
        .unwrap()
        .evidence
        .expire_for_tests();
    let record = DammBuyCandidateRecord::decode(&encoded, &message).unwrap();
    validate_record(&record, &message).unwrap();
    assert!(p.prepare(None).is_err());
}

#[tokio::test]
async fn damm_buy_record_cannot_be_interpreted_as_sell_or_accept_unsupported_schema_or_extra_fields(
) {
    let c = candidate().await;
    let message = c.transaction.message.serialize();
    let encoded = c.record.encode(&message).unwrap();
    for change in 0..7 {
        let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        match change {
            0 => value["version"] = json!(2),
            1 => value["profile"] = json!("damm-v2-sell-v1"),
            2 => value["extra"] = json!(true),
            3 => value["details"]["sell_percent"] = json!(100),
            4 => value["details"]["pool"]["extra"] = json!(true),
            5 => value["details"]["input_units"] = json!(100.5),
            _ => {
                value["details"]
                    .as_object_mut()
                    .unwrap()
                    .remove("input_units");
            }
        }
        assert!(DammBuyCandidateRecord::decode(&value.to_string(), &message).is_err());
    }
    let sell =
        crate::adapters::sdk::damm_sell_message::candidate_fixture(SwqosType::Default, false, 50);
    let sell_bytes = sell.message_bytes();
    let sell_json = sell.metadata().encode(&sell_bytes).unwrap();
    assert!(DammBuyCandidateRecord::decode(&sell_json, &sell_bytes).is_err());
    assert!(SdkCandidateMetadata::decode(&encoded, &message).is_err());
    assert_eq!(
        SdkCandidateMetadata::decode(&sell_json, &sell_bytes)
            .unwrap()
            .encode(&sell_bytes)
            .unwrap(),
        sell_json
    );
}

#[tokio::test]
async fn damm_buy_record_digest_size_and_integer_minimum_are_bounded_without_float_conversion() {
    let c = candidate().await;
    let message = c.transaction.message.serialize();
    let encoded = c.record.encode(&message).unwrap();
    assert!(DammBuyCandidateRecord::decode(&" ".repeat(65_537), &message).is_err());
    assert!(DammBuyCandidateRecord::new(c.record.buy().unwrap().details().clone(), &[]).is_err());
    assert!(
        DammBuyCandidateRecord::new(c.record.buy().unwrap().details().clone(), &vec![1; 1233])
            .is_err()
    );
    assert!(DammBuyCandidateRecord::decode(&encoded, b"different-message").is_err());
    for output in [1, 1_000_000, 9_007_199_254_740_993, u64::MAX] {
        let mut d = c.record.buy().unwrap().details().clone();
        d.slippage_bps = 0;
        d.quoted_net_output_units = output;
        d.minimum_output_units = output;
        let record = DammBuyCandidateRecord::new(d, &message).unwrap();
        assert_eq!(
            DammBuyCandidateRecord::decode(&record.encode(&message).unwrap(), &message).unwrap(),
            record
        );
    }
}

#[tokio::test]
async fn damm_buy_record_preserves_mode_mev_and_confirmation_context_but_never_turns_it_into_permission(
) {
    let c = candidate().await;
    let message = c.transaction.message.serialize();
    let mut d = c.record.buy().unwrap().details().clone();
    d.use_mev = Some(true);
    d.requested_mev_provider = Some(crate::domain::trade::MevProvider::Temporal);
    d.wait_tx_confirmed = true;
    d.mev_submit_verify_ms = 500;
    let record = DammBuyCandidateRecord::new(d, &message).unwrap();
    let decoded =
        DammBuyCandidateRecord::decode(&record.encode(&message).unwrap(), &message).unwrap();
    assert_eq!(decoded, record);
    validate_record(&decoded, &message).unwrap();
    let mut c = c;
    c.record = SdkPreparedRecord::DammBuy(decoded);
    assert!(c.validate_full(&c.instructions).is_err());
}

#[test]
fn damm_buy_record_relay_names_keep_the_original_sell_names_for_every_pinned_sdk_provider() {
    for (provider, name) in [
        (SwqosType::Default, "default"),
        (SwqosType::Jito, "jito"),
        (SwqosType::NextBlock, "next-block"),
        (SwqosType::ZeroSlot, "zero-slot"),
        (SwqosType::Temporal, "temporal"),
        (SwqosType::Bloxroute, "bloxroute"),
        (SwqosType::Node1, "node1"),
        (SwqosType::FlashBlock, "flash-block"),
        (SwqosType::BlockRazor, "block-razor"),
        (SwqosType::Astralane, "astralane"),
        (SwqosType::Stellium, "stellium"),
        (SwqosType::Lightspeed, "lightspeed"),
        (SwqosType::Soyas, "soyas"),
        (SwqosType::Speedlanding, "speedlanding"),
        (SwqosType::Helius, "helius"),
        (SwqosType::Solami, "solami"),
        (SwqosType::LunarLander, "lunar-lander"),
        (SwqosType::Glaive, "glaive"),
    ] {
        assert_eq!(relay_name(provider), name);
        assert_eq!(relay(name).unwrap(), provider);
    }
    assert!(relay("unknown-provider").is_err());
}
