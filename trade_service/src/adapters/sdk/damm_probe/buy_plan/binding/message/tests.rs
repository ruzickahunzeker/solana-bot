use super::*;
use crate::adapters::{
    rpc::damm_sell_simulation::tests::MemorySimulation,
    sdk::{damm_instructions::tip_accounts, ordinary_params::ordinary_buy_params},
};
use solana_sdk::{
    message::{Message, VersionedMessage},
    signature::Signature,
};
use std::sync::atomic::{AtomicBool, Ordering};

mod shared_tests;

pub(super) async fn profile(
    provider: SwqosType,
    simulate: bool,
    native: bool,
    exists: bool,
) -> Arc<DammBuyMessage> {
    let (quote, command, request, context) =
        super::super::tests::fixture(native, false, exists, simulate, provider).await;
    DammBuyMessage::new(Arc::new(quote), context, &command, &request).unwrap()
}

pub(super) async fn evidenced(
    wallet: Pubkey,
    native: bool,
    extended: bool,
    exists: bool,
    reversed: bool,
    remaining: Option<u64>,
    simulate: bool,
    provider: SwqosType,
) -> Result<Arc<DammBuyMessage>> {
    let (plan, evidence) = super::super::super::evidence::tests::message_fixture(
        wallet, native, extended, exists, reversed, remaining,
    )
    .await;
    let command = super::super::tests::intent(&plan, simulate);
    let context = DammBuyContext {
        wallet,
        provider,
        version: TradeTransactionVersion::V0,
        seed_optimized: false,
    };
    let minimum = evidence.minimum(&plan, command.slippage_bps)?;
    let request = ordinary_buy_params(
        &command,
        DexParamEnum::MeteoraDammV2(plan.accounts.pool.clone()),
        if native {
            TradeTokenType::WSOL
        } else {
            TradeTokenType::USDC
        },
        plan.accounts.amount,
        exists,
        provider,
        Some(minimum),
    )?;
    let quote = DammBuyQuote::new(context, &command, plan, evidence)?;
    DammBuyMessage::new(Arc::new(quote), context, &command, &request)
}

pub(super) fn full(profile: &Arc<DammBuyMessage>) -> Vec<Instruction> {
    let q = &profile.quote;
    let destination =
        (!q.intent.simulate && q.context.provider != SwqosType::Default && q.intent.tip_sol > 0.0)
            .then(|| tip_accounts(q.context.provider)[0]);
    profile.prepare(destination).unwrap().instructions
}

#[tokio::test]
async fn damm_buy_message_matches_64_actual_sdk_protocols_and_independent_full_messages_without_signing(
) {
    use sol_trade_sdk::{
        instruction::meteora_damm_v2::MeteoraDammV2InstructionBuilder,
        swqos::TradeType,
        trading::{common::extend_compute_budget_instructions, core::traits::InstructionBuilder},
    };
    let base = crate::adapters::sdk::damm_probe::sdk_oracle_tests::oracle_cases()
        .await
        .remove(0)
        .request;
    use solana_sdk::signature::Signer;
    let mut count = 0;
    for native in [false, true] {
        for extended in [false, true] {
            for exists in [false, true] {
                for reversed in [false, true] {
                    for simulate in [false, true] {
                        for provider in [SwqosType::Default, SwqosType::Temporal] {
                            let p = evidenced(
                                base.payer.pubkey(),
                                native,
                                extended,
                                exists,
                                reversed,
                                None,
                                simulate,
                                provider,
                            )
                            .await
                            .unwrap();
                            let q = &p.quote;
                            let mut request = base.clone();
                            request.trade_type = TradeType::Buy;
                            request.input_mint = q.intent.quote_mint;
                            request.output_mint = q.intent.mint;
                            request.input_amount = q.intent.amount_lamports;
                            request.fixed_output_amount = Some(q.minimum);
                            request.protocol_params = p.request.extension_params.clone();
                            request.create_input_mint_ata = true;
                            request.close_input_mint_ata = false;
                            request.create_output_mint_ata = p.request.create_mint_ata;
                            request.close_output_mint_ata = false;
                            let raw = MeteoraDammV2InstructionBuilder
                                .build_buy_instructions(&request)
                                .await
                                .unwrap();
                            assert_eq!(p.protocol, raw);
                            let mut expected = vec![];
                            let destination = (!simulate && provider != SwqosType::Default)
                                .then(|| tip_accounts(provider)[0]);
                            if let Some(destination) = destination {
                                expected.push(solana_system_interface::instruction::transfer(
                                    &q.context.wallet,
                                    &destination,
                                    100_000,
                                ));
                            }
                            extend_compute_budget_instructions(&mut expected, 1_000, 300_000);
                            expected.extend(raw.clone());
                            let candidate = p.prepare(destination).unwrap();
                            assert_eq!(candidate.instructions, expected);
                            assert_eq!(
                                candidate.transaction.message,
                                VersionedMessage::Legacy(Message::new_with_blockhash(
                                    &expected,
                                    Some(&q.context.wallet),
                                    &q.intent.recent_blockhash
                                ))
                            );
                            assert_eq!(candidate.transaction.signatures, [Signature::default()]);
                            let (wire, signature) = sol_trade_sdk::swqos::serialization::serialize_transaction_bincode_sync(
                    &candidate.transaction).unwrap();
                            assert_eq!(
                                wire.as_ref(),
                                bincode::serialize(&candidate.transaction).unwrap()
                            );
                            assert!(wire.len() <= 1232);
                            assert_eq!(signature, Signature::default());
                            if !simulate {
                                let invocation = guard::Invocation::new(p.clone());
                                let manager = invocation.middleware();
                                assert_eq!(
                                    manager
                                        .apply_middlewares_process_protocol_instructions(
                                            raw.clone(),
                                            "MeteoraDammV2",
                                            true
                                        )
                                        .unwrap(),
                                    raw
                                );
                                assert_eq!(
                                    manager
                                        .apply_middlewares_process_full_instructions(
                                            expected.clone(),
                                            "MeteoraDammV2",
                                            true
                                        )
                                        .unwrap(),
                                    expected
                                );
                            }
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(count, 64);
}

#[tokio::test]
async fn damm_buy_message_uses_same_bank_remaining_native_balance_for_exact_final_fee_boundaries() {
    for native in [false, true] {
        for simulate in [false, true] {
            for provider in [SwqosType::Default, SwqosType::Temporal] {
                let required = if !simulate && provider != SwqosType::Default {
                    100_300
                } else {
                    300
                };
                for remaining in [required - 1, required, required + 1] {
                    let result = evidenced(
                        Pubkey::new_from_array([5; 32]),
                        native,
                        false,
                        false,
                        false,
                        Some(remaining),
                        simulate,
                        provider,
                    )
                    .await;
                    assert_eq!(result.is_ok(), remaining >= required,
                    "native={native} simulate={simulate} provider={provider:?} remaining={remaining}");
                    if let Err(error) = &result {
                        let failure = error.downcast_ref::<crate::domain::preparation_balance::PreparationBalanceFailure>().unwrap();
                        assert!(!failure.allows_lane_fallback());
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn damm_buy_message_freezes_target_creation_even_when_quote_would_allow_another_request() {
    let p = profile(SwqosType::Temporal, false, true, true).await;
    let q = &p.quote;
    p.validate(q.context, &q.intent, &p.request).unwrap();
    p.validate_request(q.context, &p.request).unwrap();
    assert!(!p.request.create_mint_ata);
    let mut other = p.request.clone();
    other.create_mint_ata = true;
    q.validate(q.context, &q.intent, &other).unwrap();
    assert!(p.validate(q.context, &q.intent, &other).is_err());
    assert!(p.validate_request(q.context, &other).is_err());
    let mut intent = q.intent.clone();
    intent.amount_lamports = Some(101);
    assert!(p.validate(q.context, &intent, &p.request).is_err());
    p.validate(q.context, &q.intent, &p.request).unwrap();
}

#[tokio::test]
async fn damm_buy_message_rejects_saturated_tip_budget_without_overflow_or_constant_fallback() {
    let (quote, mut intent, mut request, context) =
        super::super::tests::fixture(true, false, true, false, SwqosType::Temporal).await;
    intent.tip_sol = f64::MAX;
    request.gas_fee_strategy = crate::adapters::sdk::ordinary_params::default_fee_strategy(
        context.provider,
        intent.tip_sol,
    );
    let quote = DammBuyQuote::new(context, &intent, quote.plan, quote.evidence).unwrap();
    let error = DammBuyMessage::new(Arc::new(quote), context, &intent, &request)
        .err()
        .unwrap();
    assert!(error.to_string().contains("budget overflow"));
}

#[tokio::test]
async fn damm_buy_message_rejects_compiled_permission_index_hash_and_wire_version_mutations() {
    let p = profile(SwqosType::Temporal, false, true, false).await;
    let candidate = p
        .prepare(Some(tip_accounts(SwqosType::Temporal)[0]))
        .unwrap();
    for change in 0..9 {
        let mut actual = candidate.transaction.message.clone();
        let VersionedMessage::Legacy(message) = &mut actual else {
            unreachable!()
        };
        match change {
            0 => message.header.num_required_signatures = 2,
            1 => message.header.num_readonly_signed_accounts = 1,
            2 => message.header.num_readonly_unsigned_accounts = 0,
            3 => message.account_keys[0] = Pubkey::new_from_array([99; 32]),
            4 => message.recent_blockhash = solana_sdk::hash::Hash::new_from_array([99; 32]),
            5 => message.instructions[0].program_id_index = 255,
            6 => message.instructions[0].accounts[0] = 255,
            7 => message.instructions[0].data[4] ^= 1,
            _ => message.instructions.swap(0, 1),
        }
        assert!(
            crate::adapters::sdk::damm_instructions::validate_compiled_message(
                p.quote.context.wallet,
                &candidate.instructions,
                p.quote.intent.recent_blockhash,
                &actual
            )
            .is_err()
        );
    }
    let VersionedMessage::Legacy(message) = &candidate.transaction.message else {
        unreachable!()
    };
    let versioned = VersionedMessage::V0(solana_sdk::message::v0::Message {
        header: message.header,
        account_keys: message.account_keys.clone(),
        recent_blockhash: message.recent_blockhash,
        instructions: message.instructions.clone(),
        address_table_lookups: vec![],
    });
    assert!(
        crate::adapters::sdk::damm_instructions::validate_compiled_message(
            p.quote.context.wallet,
            &candidate.instructions,
            p.quote.intent.recent_blockhash,
            &versioned
        )
        .is_err()
    );
}

#[tokio::test]
async fn damm_buy_message_unsigned_simulation_uses_exact_fixed_packet_without_invoking_sdk() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    for native in [false, true] {
        for exists in [false, true] {
            let p = profile(SwqosType::Temporal, true, native, exists).await;
            let candidate = p.prepare(None).unwrap();
            let source = MemorySimulation::new();
            let called = AtomicBool::new(false);
            let report = p
                .execute(&source.client(), |_| async {
                    called.store(true, Ordering::SeqCst);
                    panic!("simulation must not enter the SDK signer");
                })
                .await
                .unwrap_or_else(|e| panic!("{}", e.message));
            assert!(
                report.success && report.signatures.is_empty() && !called.load(Ordering::SeqCst)
            );
            let calls = source.calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0][1]["sigVerify"], false);
            assert_eq!(calls[0][1]["replaceRecentBlockhash"], false);
            assert_eq!(calls[0][1]["minContextSlot"], 43);
            let bytes = STANDARD.decode(calls[0][0].as_str().unwrap()).unwrap();
            assert_eq!(bytes, bincode::serialize(&candidate.transaction).unwrap());
        }
    }
}

struct UnsignedBuyLane {
    profile: Arc<DammBuyMessage>,
    rpc: solana_client::nonblocking::rpc_client::RpcClient,
}

impl crate::adapters::sdk::ordinary_lanes::OrdinarySdkLane for UnsignedBuyLane {
    type Prepared = Arc<DammBuyMessage>;
    fn provider(&self) -> SwqosType {
        self.profile.quote.context.provider
    }
    async fn prepare(&self, trade: &ExecuteTrade) -> Result<Self::Prepared> {
        self.profile
            .validate(self.profile.quote.context, trade, &self.profile.request)?;
        Ok(self.profile.clone())
    }
    fn validate_prepared(&self, prepared: &Self::Prepared, trade: &ExecuteTrade) -> Result<()> {
        prepared.validate(self.profile.quote.context, trade, &self.profile.request)
    }
    async fn invoke(
        &self,
        prepared: Self::Prepared,
    ) -> Result<SdkLaneReport, SdkInvocationFailure> {
        prepared
            .execute(&self.rpc, |_| async {
                panic!("unsigned ordinary simulation cannot invoke SDK")
            })
            .await
    }
    async fn observe(
        &self,
        _: &str,
        _: &ExecuteTrade,
    ) -> Result<crate::adapters::sdk::ordinary_lanes::SdkLaneObservation> {
        panic!("unsigned ordinary simulation cannot query a signature")
    }
}

#[tokio::test]
async fn damm_buy_message_unsigned_report_reaches_original_ordinary_result_with_no_transaction_signature(
) {
    let p = profile(SwqosType::Temporal, true, true, false).await;
    let source = MemorySimulation::new();
    let trade = p.quote.intent.clone();
    let lane = UnsignedBuyLane {
        profile: p,
        rpc: source.client(),
    };
    let result = crate::adapters::sdk::ordinary_lanes::execute([lane], &trade)
        .await
        .unwrap();
    assert!(result.success && result.signature.is_none() && result.error.is_none());
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_buy_message_unsigned_rpc_failure_or_rejection_never_calls_sdk_or_retries() {
    for fail in [false, true] {
        let p = profile(SwqosType::Temporal, true, true, false).await;
        let mut source = MemorySimulation::new();
        source.fail = fail;
        if !fail {
            source.response["value"]["err"] =
                serde_json::json!({"InstructionError":[2,{"Custom":6001}]});
        }
        let result = p
            .execute(&source.client(), |_| async {
                panic!("no fallback SDK execution after unsigned failure");
            })
            .await;
        if fail {
            let e = result.err().unwrap();
            assert!(!e.message.contains("simulation-secret") && e.signatures.is_empty());
        } else {
            let report = result.unwrap_or_else(|e| panic!("{}", e.message));
            assert!(!report.success && report.signatures.is_empty());
        }
        assert_eq!(source.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn damm_buy_message_simulation_cancellation_drops_original_rpc_without_fallback() {
    let p = profile(SwqosType::Temporal, true, true, false).await;
    let mut source = MemorySimulation::new();
    source.block = true;
    let rpc = source.client();
    let task = tokio::spawn(async move { p.execute(&rpc, |_| async { panic!("no SDK") }).await });
    source.entered.notified().await;
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_buy_message_rechecks_original_quote_after_unsigned_await_without_renewal() {
    let p = profile(SwqosType::Temporal, true, true, false).await;
    let mut source = MemorySimulation::new();
    source.delay = Some(std::time::Duration::from_millis(3100));
    let error = p
        .execute(&source.client(), |_| async { panic!("no SDK") })
        .await
        .err()
        .unwrap();
    assert!(error.message.contains("expired") && error.signatures.is_empty());
    assert_eq!(source.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn damm_buy_message_live_callback_uses_original_instructions_without_any_simulation_or_signer(
) {
    let p = profile(SwqosType::Temporal, false, true, false).await;
    let source = MemorySimulation::new();
    let expected = full(&p);
    let protocol = p.protocol.clone();
    let result = p
        .execute(&source.client(), |manager| async move {
            manager
                .apply_middlewares_process_protocol_instructions(protocol, "MeteoraDammV2", true)
                .unwrap();
            manager
                .apply_middlewares_process_full_instructions(expected, "MeteoraDammV2", true)
                .unwrap();
            Ok(SdkLaneReport {
                success: false,
                signatures: vec![],
                error: Some("public-fixture-no-signing".into()),
            })
        })
        .await
        .unwrap_or_else(|e| panic!("{}", e.message));
    assert!(!result.success && result.signatures.is_empty());
    assert!(source.calls.lock().unwrap().is_empty());
}
