//! Pin the actual SDK compiler boundary; compilation alone grants no execution authority.
use sol_trade_sdk::{
    common::TradeTransactionVersion, trading::core::transaction_pool::compile_message,
};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{v0, v1, AddressLookupTableAccount, Message, VersionedMessage},
    pubkey::Pubkey,
};

fn key(value: u8) -> Pubkey {
    Pubkey::new_from_array([value; 32])
}

fn instructions() -> Vec<Instruction> {
    vec![Instruction {
        program_id: key(5),
        accounts: vec![
            AccountMeta::new_readonly(key(1), true),
            AccountMeta::new(key(2), false),
            AccountMeta::new_readonly(key(3), false),
        ],
        data: vec![11, 22, 33],
    }]
}

fn compile(instructions: &[Instruction]) -> VersionedMessage {
    compile_message(
        &key(1),
        instructions,
        &[],
        Hash::new_from_array([9; 32]),
        TradeTransactionVersion::V0,
        v1::TransactionConfig::empty(),
    )
    .unwrap()
}

#[test]
fn unsigned_compile_v0_without_alts_preserves_actual_legacy_selection() {
    let instructions = instructions();
    let actual = compile(&instructions);
    let expected = VersionedMessage::Legacy(Message::new_with_blockhash(
        &instructions,
        Some(&key(1)),
        &Hash::new_from_array([9; 32]),
    ));
    assert!(matches!(actual, VersionedMessage::Legacy(_)));
    assert_eq!(actual.serialize(), expected.serialize());
    assert_eq!(actual.header().num_required_signatures, 1);
    assert_eq!(actual.static_account_keys()[0], key(1));
    assert_eq!(actual, compile(&instructions));
}

#[test]
fn unsigned_compile_v0_with_alts_preserves_indices_and_unresolved_static_keys() {
    let instructions = instructions();
    let table = AddressLookupTableAccount {
        key: key(8),
        addresses: vec![key(3), key(2), key(7)],
    };
    let hash = Hash::new_from_array([9; 32]);
    let actual = compile_message(
        &key(1),
        &instructions,
        std::slice::from_ref(&table),
        hash,
        TradeTransactionVersion::V0,
        v1::TransactionConfig::empty(),
    )
    .unwrap();
    let expected = VersionedMessage::V0(
        v0::Message::try_compile(&key(1), &instructions, &[table], hash).unwrap(),
    );
    assert_eq!(actual.serialize(), expected.serialize());
    let VersionedMessage::V0(message) = &actual else {
        panic!("ALT-backed V0 must not silently select legacy");
    };
    assert_eq!(message.address_table_lookups.len(), 1);
    assert_eq!(message.address_table_lookups[0].account_key, key(8));
    assert_eq!(message.address_table_lookups[0].writable_indexes, [1]);
    assert_eq!(message.address_table_lookups[0].readonly_indexes, [0]);
    assert_eq!(message.account_keys, [key(1), key(5)]);
    assert_eq!(message.instructions[0].accounts, [0, 2, 3]);

    let missing = AddressLookupTableAccount {
        key: key(8),
        addresses: vec![key(7)],
    };
    let unresolved = compile_message(
        &key(1),
        &instructions,
        &[missing],
        hash,
        TradeTransactionVersion::V0,
        v1::TransactionConfig::empty(),
    )
    .unwrap();
    assert!(unresolved.static_account_keys().contains(&key(2)));
    assert!(unresolved.static_account_keys().contains(&key(3)));
}

#[test]
fn unsigned_compile_v1_binds_explicit_configuration_without_signing() {
    let instructions = instructions();
    let hash = Hash::new_from_array([9; 32]);
    let config = v1::TransactionConfig::empty()
        .with_compute_unit_limit(300_000)
        .with_priority_fee(300)
        .with_loaded_accounts_data_size_limit(64 * 1024 * 1024);
    let actual = compile_message(
        &key(1),
        &instructions,
        &[],
        hash,
        TradeTransactionVersion::V1,
        config,
    )
    .unwrap();
    let expected = VersionedMessage::V1(
        v1::Message::try_compile_with_config(&key(1), &instructions, hash, config).unwrap(),
    );
    assert!(matches!(actual, VersionedMessage::V1(_)));
    assert_eq!(actual.serialize(), expected.serialize());
    let changed = compile_message(
        &key(1),
        &instructions,
        &[],
        hash,
        TradeTransactionVersion::V1,
        config.with_priority_fee(301),
    )
    .unwrap();
    assert_ne!(actual.serialize(), changed.serialize());
}

#[test]
fn unsigned_compile_v1_rejects_alts_instead_of_switching_version() {
    let error = compile_message(
        &key(1),
        &instructions(),
        &[AddressLookupTableAccount {
            key: key(8),
            addresses: vec![key(2)],
        }],
        Hash::new_from_array([9; 32]),
        TradeTransactionVersion::V1,
        v1::TransactionConfig::empty(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "V1 transactions do not support address lookup tables"
    );
}

#[test]
fn unsigned_compile_does_not_authorize_additional_signers_or_local_meta_flags() {
    let mut instructions = instructions();
    let baseline = compile(&instructions);
    let reserved = std::collections::HashSet::<Pubkey>::new();
    let index = baseline
        .static_account_keys()
        .iter()
        .position(|address| *address == key(3))
        .unwrap();
    assert!(!baseline.is_signer(index));
    assert!(!baseline.is_maybe_writable_with_reserved_addresses(index, Some(&reserved)));

    instructions.push(Instruction {
        program_id: key(6),
        accounts: vec![AccountMeta::new(key(3), true)],
        data: vec![],
    });
    let promoted = compile(&instructions);
    let index = promoted
        .static_account_keys()
        .iter()
        .position(|address| *address == key(3))
        .unwrap();
    assert_eq!(promoted.header().num_required_signatures, 2);
    assert!(promoted.is_signer(index));
    assert!(promoted.is_maybe_writable_with_reserved_addresses(index, Some(&reserved)));
    assert!(!instructions[0].accounts[2].is_writable);
    assert!(!instructions[0].accounts[2].is_signer);
}

#[test]
fn unsigned_compile_binds_payer_hash_program_accounts_data_and_instruction_order() {
    let source = instructions();
    let baseline = compile(&source).serialize();
    for change in 0..5 {
        let mut modified = source.clone();
        match change {
            0 => modified[0].program_id = key(6),
            1 => modified[0].accounts[1].pubkey = key(7),
            2 => modified[0].accounts[1].is_writable = false,
            3 => modified[0].data[0] ^= 1,
            4 => modified.push(modified[0].clone()),
            _ => unreachable!(),
        }
        assert_ne!(baseline, compile(&modified).serialize(), "change {change}");
    }
    for (payer, hash) in [
        (key(7), Hash::new_from_array([9; 32])),
        (key(1), Hash::new_from_array([8; 32])),
    ] {
        let modified = compile_message(
            &payer,
            &source,
            &[],
            hash,
            TradeTransactionVersion::V0,
            v1::TransactionConfig::empty(),
        )
        .unwrap();
        assert_ne!(baseline, modified.serialize());
    }
    let mut pair = source;
    pair.push(Instruction {
        program_id: key(6),
        accounts: vec![AccountMeta::new_readonly(key(2), false)],
        data: vec![44],
    });
    let forward = compile(&pair).serialize();
    pair.reverse();
    assert_ne!(forward, compile(&pair).serialize());
}

#[test]
fn unsigned_compile_owns_output_and_does_not_mutate_borrowed_instructions() {
    let mut source = instructions();
    let original = source.clone();
    let message = compile(&source);
    let bytes = message.serialize();
    assert_eq!(source, original);
    source[0].data.fill(99);
    source[0].accounts.clear();
    source.clear();
    assert_eq!(message.serialize(), bytes);
}

#[test]
fn unsigned_compile_accepts_oversize_and_default_hash_without_authorizing_them() {
    let mut source = instructions();
    source[0].data.resize(1400, 7);
    let message = compile_message(
        &key(1),
        &source,
        &[],
        Hash::default(),
        TradeTransactionVersion::V0,
        v1::TransactionConfig::empty(),
    )
    .unwrap();
    assert!(message.serialize().len() > 1232);
    assert_eq!(*message.recent_blockhash(), Hash::default());
    assert!(message.static_account_keys().contains(&key(5)));
}

#[test]
fn unsigned_compile_fee_strategy_clone_is_not_an_immutable_fee_snapshot() {
    use sol_trade_sdk::{
        common::{GasFeeStrategy, GasFeeStrategyType},
        swqos::{SwqosType, TradeType},
    };

    let original = GasFeeStrategy::new();
    original.set(
        SwqosType::Temporal,
        TradeType::Buy,
        GasFeeStrategyType::Normal,
        300_000,
        1_000,
        0.001,
    );
    let copied_values = original.get_strategies(TradeType::Buy);
    let shared = original.clone();
    shared.update_buy_tip(0.002);
    let updated = original.get_strategies(TradeType::Buy);
    assert_eq!(copied_values.len(), 1);
    assert_eq!(updated.len(), 1);
    assert_eq!(copied_values[0].2.tip, 0.001);
    assert_eq!(updated[0].2.tip, 0.002);
    shared.clear();
    assert!(original.get_strategies(TradeType::Buy).is_empty());
    assert_eq!(copied_values[0].2.cu_limit, 300_000);
    assert_eq!(copied_values[0].2.cu_price, 1_000);
}
