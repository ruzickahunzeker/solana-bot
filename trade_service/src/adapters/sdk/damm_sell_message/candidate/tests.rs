use super::super::tests::{case, fixture, key};
use super::*;
use solana_sdk::{hash::Hash, message::v0};

fn candidate() -> DammSellCandidate {
    DammSellCandidate::build(
        fixture(SwqosType::Temporal, false, 50).profile().unwrap(),
        Some(tip_accounts(SwqosType::Temporal)[0]),
    )
    .unwrap()
}

#[test]
fn damm_compiled_binding_matches_standard_message_without_sdk_compiler_for_the_profile_matrix() {
    let mut count = 0;
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        for simulate in [false, true] {
            for percent in [1, 50, 100] {
                for quote in [WSOL_TOKEN_ACCOUNT, USDC_TOKEN_ACCOUNT] {
                    for reversed in [false, true] {
                        for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
                            let f = case(provider, simulate, percent, quote, reversed, program);
                            let destination = (!simulate && provider != SwqosType::Default)
                                .then(|| tip_accounts(provider)[0]);
                            let c = DammSellCandidate::build(f.profile().unwrap(), destination)
                                .unwrap();
                            let expected = Message::new_with_blockhash(
                                &c.instructions,
                                Some(&f.context.wallet),
                                &f.intent.recent_blockhash,
                            );
                            assert_eq!(
                                c.transaction.message,
                                VersionedMessage::Legacy(expected.clone())
                            );
                            assert_eq!(c.message_bytes(), expected.serialize());
                            assert_eq!(c.transaction.signatures, [Signature::default()]);
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
fn damm_compiled_binding_rejects_program_data_account_index_permission_order_and_hash_changes() {
    let c = candidate();
    let profile = &c.profile;
    for change in 0..14 {
        let mut actual = c.transaction.message.clone();
        let VersionedMessage::Legacy(message) = &mut actual else {
            unreachable!()
        };
        match change {
            0 => message.header.num_required_signatures = 2,
            1 => message.header.num_readonly_signed_accounts = 1,
            2 => message.header.num_readonly_unsigned_accounts = 0,
            3 => message.account_keys[0] = key(30),
            4 => message.account_keys.swap(1, 2),
            5 => message.account_keys.push(key(30)),
            6 => {
                message.account_keys.pop();
            }
            7 => message.recent_blockhash = Hash::new_from_array([30; 32]),
            8 => message.instructions[0].program_id_index = 255,
            9 => message.instructions[0].accounts[0] = 255,
            10 => message.instructions[0].data[0] ^= 1,
            11 => message.instructions.swap(0, 1),
            12 => message.instructions.push(message.instructions[0].clone()),
            13 => {
                message.instructions.pop();
            }
            _ => unreachable!(),
        }
        assert!(
            validate_compiled_message(
                profile.context.wallet,
                &c.instructions,
                profile.intent.recent_blockhash,
                &actual,
            )
            .is_err(),
            "change {change}"
        );
    }
}

#[test]
fn damm_compiled_binding_cannot_rebind_old_bytes_to_a_new_wallet_hash_or_instruction_set() {
    let c = candidate();
    let wallet = c.profile.context.wallet;
    let hash = c.profile.intent.recent_blockhash;
    assert!(
        validate_compiled_message(key(30), &c.instructions, hash, &c.transaction.message).is_err()
    );
    assert!(validate_compiled_message(
        wallet,
        &c.instructions,
        Hash::new_from_array([30; 32]),
        &c.transaction.message
    )
    .is_err());
    let mut changed = c.instructions.clone();
    changed[0].accounts[1].pubkey = tip_accounts(SwqosType::Temporal)[1];
    assert!(validate_compiled_message(wallet, &changed, hash, &c.transaction.message).is_err());
    changed = c.instructions.clone();
    changed[0].accounts[1].is_writable = false;
    assert!(validate_compiled_message(wallet, &changed, hash, &c.transaction.message).is_err());
}

#[test]
fn damm_compiled_binding_requires_review_before_switching_this_legacy_wire_profile_to_v0() {
    let c = candidate();
    let VersionedMessage::Legacy(original) = &c.transaction.message else {
        unreachable!()
    };
    let mut converted = v0::Message {
        header: original.header,
        account_keys: original.account_keys.clone(),
        recent_blockhash: original.recent_blockhash,
        instructions: original.instructions.clone(),
        address_table_lookups: vec![],
    };
    assert!(validate_compiled_message(
        c.profile.context.wallet,
        &c.instructions,
        original.recent_blockhash,
        &VersionedMessage::V0(converted.clone())
    )
    .is_err());
    converted
        .address_table_lookups
        .push(v0::MessageAddressTableLookup {
            account_key: key(30),
            writable_indexes: vec![0],
            readonly_indexes: vec![],
        });
    assert!(validate_compiled_message(
        c.profile.context.wallet,
        &c.instructions,
        original.recent_blockhash,
        &VersionedMessage::V0(converted)
    )
    .is_err());
}
