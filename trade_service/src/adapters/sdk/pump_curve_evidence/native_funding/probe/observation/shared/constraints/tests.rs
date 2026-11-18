use super::*;
use crate::adapters::rpc::pump_native_probe::PumpNativeProbePacket;
use crate::adapters::sdk::pump_curve_evidence::{
    instruction_amounts::{PumpWireAmounts, TradeAmounts},
    native_funding::probe::observation::{post_bank::tests::response_with_bank, tests::observed},
};
use serde_json::json;
use sol_trade_sdk::swqos::TradeType;
use std::time::{Duration, Instant};

async fn fixture(side: TradeType) -> NativeBankObservation {
    let (probe, response) = observed(side).await;
    let response = response_with_bank(&probe, response);
    let bank = probe.inspect_post_bank(&response).unwrap();
    let observation = probe.inspect_observation(response).unwrap();
    NativeBankObservation { observation, bank }
}

#[tokio::test]
async fn native_swap_constraints_bind_net_slippage_without_changing_exact_input_or_roles() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for slippage in [0, 1, 1_500, 9_999] {
            let source = fixture(side).await;
            let net = source.observation.output.get();
            let expected = u128::from(net) * u128::from(10_000 - slippage) / 10_000;
            if expected == 0 {
                assert!(source.swap_constraints(slippage).is_err());
                continue;
            }
            let constraints = source.swap_constraints(slippage).unwrap();
            assert_eq!(constraints.minimum.get() as u128, expected);
            let tx = constraints.packet().unwrap();
            let original = constraints.source.observation.probe.packet().unwrap();
            assert_eq!(tx.message.account_keys, original.message.account_keys);
            assert_eq!(tx.message.header, original.message.header);
            assert_eq!(
                tx.message.recent_blockhash,
                original.message.recent_blockhash
            );
            assert_eq!(tx.message.instructions.len(), 1);
            assert_eq!(
                tx.message.instructions[0].accounts,
                original.message.instructions[0].accounts
            );
            let wire = PumpWireAmounts::decode(&tx.message.instructions[0].data).unwrap();
            let input = constraints.source.observation.probe.input;
            assert_eq!(
                wire.amounts,
                if matches!(side, TradeType::Buy) {
                    TradeAmounts::BuyExactInput {
                        spend: input,
                        minimum: constraints.minimum,
                    }
                } else {
                    TradeAmounts::SellExactInput {
                        input,
                        minimum: constraints.minimum,
                    }
                }
            );
            assert_eq!(tx.signatures, [Signature::default()]);
        }
    }
}

#[tokio::test]
async fn native_swap_constraints_prefund_only_same_response_spend_without_sell_output_credit() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let source = fixture(side).await;
        assert_ne!(source.observation.probe.funding.native_lamports, 50_000_000);
        let constraints = source.swap_constraints(1_500).unwrap();
        assert_eq!(constraints.native.available_before, 50_000_000);
        let debit = if matches!(side, TradeType::Buy) {
            1_005_000
        } else {
            5_000
        };
        assert_eq!(constraints.native.maximum_debit, debit);
        assert_eq!(
            constraints.native.remaining_without_output,
            50_000_000 - debit
        );
        if matches!(side, TradeType::Sell) {
            assert_ne!(
                constraints.native.remaining_without_output,
                constraints.source.bank.snapshot.native_lamports
            );
        }
        let keys = constraints.source.observation.probe.evidence_accounts();
        // The floor-one discovery profile must not be widened to accept this formal-minimum template.
        assert!(PumpNativeProbePacket::new(
            constraints.packet().unwrap(),
            &keys,
            175,
            Instant::now() + Duration::from_secs(1)
        )
        .is_err());
    }
    let (probe, mut response) = observed(TradeType::Sell).await;
    let curve = probe
        .transaction
        .message
        .account_keys
        .iter()
        .position(|key| *key == probe.funding.keys[0])
        .unwrap();
    response["value"]["postBalances"][0] = json!(49_997_000u64);
    response["value"]["postBalances"][curve] = json!(59_992_000u64);
    let response = response_with_bank(&probe, response);
    let bank = probe.inspect_post_bank(&response).unwrap();
    let observation = probe.inspect_observation(response).unwrap();
    assert_eq!(observation.output.get(), 2_000);
    let constraints = NativeBankObservation { observation, bank }
        .swap_constraints(1_500)
        .unwrap();
    assert_eq!(constraints.native.available_before, 50_000_000);
    assert_eq!(constraints.native.remaining_without_output, 49_995_000);
}

#[test]
fn native_swap_constraints_reject_invalid_slippage_zero_rounding_and_integer_overflow_fallbacks() {
    let one = NonZeroU64::new(1).unwrap();
    assert_eq!(minimum_output(one, 0).unwrap(), one);
    assert!(minimum_output(one, 1).is_err());
    for bps in [10_000, u64::MAX] {
        assert!(minimum_output(one, bps).is_err());
    }
    for bps in [0, 1, 1_500, 9_999] {
        let output = NonZeroU64::new(u64::MAX).unwrap();
        assert_eq!(
            u128::from(minimum_output(output, bps).unwrap().get()),
            u128::from(u64::MAX) * u128::from(10_000 - bps) / 10_000
        );
    }
}

#[tokio::test]
async fn native_swap_constraints_reject_bank_drift_expiry_and_modified_unsigned_templates() {
    for damage in 0..8 {
        let mut source = fixture(TradeType::Buy).await;
        match damage {
            0 => source.bank.snapshot.curve.slot += 1,
            1 => source.bank.snapshot.started = Instant::now(),
            2 => {
                source.observation.probe.funding.started = Instant::now()
                    - crate::domain::preparation_balance::MAX_BALANCE_AGE
                    - Duration::from_secs(1)
            }
            3 => source.bank.snapshot.side = Side::Sell,
            4 => source.bank.snapshot.curve.quote = sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT,
            5 => source.bank.snapshot.keys.swap(0, 1),
            6 => source.bank.snapshot.native_lamports = u64::MAX,
            _ => source.observation.network_fee = NonZeroU64::new(u64::MAX).unwrap(),
        }
        assert!(source.swap_constraints(1_500).is_err());
    }
    let mut source = fixture(TradeType::Sell).await;
    source.bank.snapshot.native_lamports = 0;
    assert!(source.swap_constraints(1_500).is_err());
    for damage in 0..8 {
        let mut constraints = fixture(TradeType::Sell)
            .await
            .swap_constraints(1_500)
            .unwrap();
        match damage {
            0 => constraints.minimum = NonZeroU64::new(1).unwrap(),
            1 => constraints.transaction.signatures[0] = Signature::from([1; 64]),
            2 => constraints.transaction.message.instructions[0].data[8..16]
                .copy_from_slice(&1u64.to_le_bytes()),
            3 => constraints
                .transaction
                .message
                .instructions
                .push(constraints.transaction.message.instructions[0].clone()),
            4 => {
                constraints.source.observation.probe.funding.started = Instant::now()
                    - crate::domain::preparation_balance::MAX_BALANCE_AGE
                    - Duration::from_secs(1)
            }
            5 => constraints.native.maximum_debit += 1,
            6 => constraints.source.bank.snapshot.native_lamports += 1,
            _ => constraints.source.bank.snapshot.curve.slot += 1,
        }
        assert!(constraints.packet().is_err());
    }
}
