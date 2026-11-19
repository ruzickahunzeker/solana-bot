use super::*;
use crate::adapters::sdk::pump_curve_evidence::{
    instruction_amounts::{tests::request, PumpWireAmounts},
    native_funding::{
        destinations::tests::account,
        instruction_tests::{build as sdk_build, funding_with_global},
    },
    SharingPrefix,
};
use crate::domain::preparation_balance::MAX_BALANCE_AGE;
use sol_trade_sdk::{
    constants::{SOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT},
    instruction::utils::pumpfun::global_constants,
    swqos::TradeType,
};
use solana_sdk::{message::Message, signature::Signature};
use std::time::{Duration, Instant};

fn nz(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn bank(side: TradeType, cashback: bool) -> NativeFundingSnapshot {
    let params = request(SOL_TOKEN_ACCOUNT, side, None, cashback);
    let mut bank = funding_with_global(&params, Some(account(global_constants::FEE_RECIPIENT)));
    bank.curve.curve.cashback = cashback;
    bank
}

fn selection(bank: &NativeFundingSnapshot) -> Pubkey {
    bank.destinations().unwrap().buyback[3]
}

#[tokio::test]
async fn pump_bank_swap_matches_actual_locked_sdk_native_wire_and_all_account_privileges() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for cashback in [false, true] {
            let mut params = request(
                SOL_TOKEN_ACCOUNT,
                side,
                matches!(side, TradeType::Sell).then_some(77),
                cashback,
            );
            params.input_amount = Some(1_000);
            let mut sdk = sdk_build(&params).await;
            let amounts = PumpWireAmounts::decode(&sdk.data).unwrap();
            let minimum = match amounts.amounts {
                TradeAmounts::BuyExactInput { spend, minimum } => {
                    assert_eq!(spend, nz(1_000));
                    minimum
                }
                TradeAmounts::SellExactInput { input, minimum } => {
                    assert_eq!(input, nz(1_000));
                    minimum
                }
                _ => panic!("fixed native input required"),
            };
            let snapshot = bank(side, cashback);
            let chosen = selection(&snapshot);
            // Only select one of the same-bank buyback set, replacing the SDK's random choice.
            sdk.accounts.last_mut().unwrap().pubkey = chosen;
            let plan = snapshot
                .prepare_native_swap(nz(1_000), minimum, chosen)
                .unwrap();
            assert_eq!(plan.instruction().unwrap(), &sdk);
            let hash = Hash::new_from_array([9; 32]);
            let wallet = plan.funding.keys[6];
            assert_eq!(
                Message::new_with_blockhash(
                    &[plan.instruction().unwrap().clone()],
                    Some(&wallet),
                    &hash
                ),
                Message::new_with_blockhash(&[sdk], Some(&wallet), &hash)
            );
        }
    }
}

#[test]
fn pump_bank_swap_owns_integer_input_minimum_and_rejects_every_changed_account_or_wire() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for value in [1, (1u64 << 53) + 1, u64::MAX] {
            let mut snapshot = bank(side, true);
            snapshot.native_lamports = u64::MAX;
            snapshot.user_base_units = Some(u64::MAX);
            let chosen = selection(&snapshot);
            let plan = snapshot
                .prepare_native_swap(nz(value), nz(value), chosen)
                .unwrap();
            let decoded = PumpWireAmounts::decode(&plan.instruction().unwrap().data).unwrap();
            assert_eq!(
                decoded.amounts,
                if matches!(side, TradeType::Buy) {
                    TradeAmounts::BuyExactInput {
                        spend: nz(value),
                        minimum: nz(value),
                    }
                } else {
                    TradeAmounts::SellExactInput {
                        input: nz(value),
                        minimum: nz(value),
                    }
                }
            );
            let original = plan.instruction().unwrap().clone();
            let mut caller_copy = original.clone();
            caller_copy.accounts[6].pubkey = Pubkey::new_unique();
            caller_copy.data[8] ^= 1;
            assert_eq!(plan.instruction().unwrap(), &original);
        }
        let count = if matches!(side, TradeType::Buy) {
            18
        } else {
            17
        };
        for index in 0..count {
            for privilege in [false, true] {
                let snapshot = bank(side, true);
                let chosen = selection(&snapshot);
                let mut plan = snapshot
                    .prepare_native_swap(nz(1_000), nz(77), chosen)
                    .unwrap();
                if privilege {
                    plan.instruction.accounts[index].is_writable ^= true;
                } else {
                    plan.instruction.accounts[index].pubkey = Pubkey::new_unique();
                }
                assert!(plan.instruction().is_err(), "{side:?}/{index}/{privilege}");
            }
        }
        for offset in [0, 8, 16] {
            let snapshot = bank(side, true);
            let chosen = selection(&snapshot);
            let mut plan = snapshot
                .prepare_native_swap(nz(1_000), nz(77), chosen)
                .unwrap();
            plan.instruction.data[offset] ^= 1;
            assert!(plan.instruction().is_err());
        }
    }
}

#[test]
fn pump_bank_swap_rejects_unsupported_bank_profiles_expiry_and_unfunded_input() {
    for side in [TradeType::Buy, TradeType::Sell] {
        for damage in 0..12 {
            let mut snapshot = bank(side, true);
            let mut chosen = selection(&snapshot);
            match damage {
                0 => snapshot.curve.quote = USDC_TOKEN_ACCOUNT,
                1 => snapshot.curve.base_mint_layout.program = spl_token_2022_interface::ID,
                2 => snapshot.curve.curve.complete = true,
                3 => snapshot.user_base_units = None,
                4 => {
                    snapshot.native_lamports = 0;
                    snapshot.user_base_units = Some(0);
                }
                5 => snapshot.curve.curve.mayhem = true,
                6 => {
                    snapshot.curve.sharing = SharingPrefix::Present {
                        bump: 1,
                        version: 1,
                        status: 1,
                    }
                }
                7 => snapshot.curve.curve.creator = Pubkey::default(),
                8 => snapshot.curve.global_data.truncate(80),
                9 => chosen = Pubkey::default(),
                10 => snapshot.started = Instant::now() - MAX_BALANCE_AGE - Duration::from_secs(1),
                _ => snapshot.started = Instant::now() + Duration::from_secs(1),
            }
            assert!(
                snapshot
                    .prepare_native_swap(nz(1_000), nz(77), chosen)
                    .is_err(),
                "{side:?}/{damage}"
            );
        }
    }
}

#[test]
fn pump_bank_swap_connects_only_to_original_unsigned_discovery_floor_and_original_age() {
    for side in [TradeType::Buy, TradeType::Sell] {
        let snapshot = bank(side, true);
        let started = snapshot.started;
        let chosen = selection(&snapshot);
        let plan = snapshot
            .prepare_native_swap(nz(1_000), nz(777), chosen)
            .unwrap();
        let original = plan.instruction().unwrap().clone();
        let wallet = plan.funding.keys[6];
        assert_eq!(plan.funding.started, started);
        let quote = plan
            .into_quote_probe(Hash::new_from_array([9; 32]))
            .unwrap();
        let packet = quote.packet().unwrap();
        assert_eq!(packet.signatures, [Signature::default()]);
        assert_eq!(packet.message.header.num_required_signatures, 1);
        assert_eq!(packet.message.account_keys[0], wallet);
        assert_eq!(packet.message.instructions.len(), 1);
        let mut discovery = original;
        discovery.data[16..24].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(
            packet.message,
            Message::new_with_blockhash(
                &[discovery],
                Some(&wallet),
                &Hash::new_from_array([9; 32])
            )
        );
        let snapshot = bank(side, true);
        let chosen = selection(&snapshot);
        assert!(snapshot
            .prepare_native_swap(nz(1_000), nz(777), chosen)
            .unwrap()
            .into_quote_probe(Hash::default())
            .is_err());
        let snapshot = bank(side, true);
        let chosen = selection(&snapshot);
        let mut plan = snapshot
            .prepare_native_swap(nz(1_000), nz(777), chosen)
            .unwrap();
        plan.funding.started = Instant::now() - MAX_BALANCE_AGE - Duration::from_secs(1);
        assert!(plan
            .into_quote_probe(Hash::new_from_array([9; 32]))
            .is_err());
    }
}
