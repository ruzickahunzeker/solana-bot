//! No wallet signer or external RPC; temporary construction keys never sign transactions.
use super::instructions::canonical_wsol_instructions;
use super::tests::unsigned_case;
use super::*;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_system_interface::instruction::SystemInstruction;
use spl_token_interface::instruction::TokenInstruction;

const SHAPES: [(Direction, u64, u64, bool); 4] = [
    (Direction::Wrap, 50, 0, false),
    (Direction::Wrap, 50, 100, true),
    (Direction::Unwrap, 100, 100, true),
    (Direction::Unwrap, 50, 100, true),
];

fn validate(prepared: &UnsignedWsolTransaction) -> Result<()> {
    prepared.validate(&prepared.owner, &prepared.authorization.quote, 1)
}

#[test]
fn wsol_authorization_decodes_exact_wrap_programs_accounts_and_amounts() {
    let owner = Pubkey::from([1; 32]);
    let mint = crate::models::WSOL_MINT.parse().unwrap();
    let ata = get_associated_token_address_with_program_id(&owner, &mint, &spl_token_interface::ID);
    for exists in [false, true] {
        let amount = 1_234_567_890;
        let instructions = canonical_wsol_instructions(
            &owner,
            Direction::Wrap,
            amount,
            0,
            exists,
            2_039_280,
            None,
        )
        .unwrap();
        let offset = usize::from(!exists);
        assert_eq!(instructions.len(), offset + 2);
        if !exists {
            assert_eq!(
                instructions[0].program_id,
                spl_associated_token_account_interface::program::ID
            );
            assert_eq!(instructions[0].data, [1]);
            assert_eq!(
                instructions[0].accounts,
                [
                    AccountMeta::new(owner, true),
                    AccountMeta::new(ata, false),
                    AccountMeta::new_readonly(owner, false),
                    AccountMeta::new_readonly(mint, false),
                    AccountMeta::new_readonly(solana_system_interface::program::ID, false),
                    AccountMeta::new_readonly(spl_token_interface::ID, false),
                ]
            );
        }
        assert_eq!(
            instructions[offset].program_id,
            solana_system_interface::program::ID
        );
        assert_eq!(
            bincode::deserialize::<SystemInstruction>(&instructions[offset].data).unwrap(),
            SystemInstruction::Transfer { lamports: amount }
        );
        assert_eq!(
            instructions[offset].accounts,
            [AccountMeta::new(owner, true), AccountMeta::new(ata, false)]
        );
        assert_eq!(instructions[offset + 1].program_id, spl_token_interface::ID);
        assert_eq!(
            TokenInstruction::unpack(&instructions[offset + 1].data).unwrap(),
            TokenInstruction::SyncNative
        );
        assert_eq!(
            instructions[offset + 1].accounts,
            [AccountMeta::new(ata, false)]
        );
    }
}

#[test]
fn wsol_authorization_decodes_partial_unwrap_rent_mint_precision_and_refund_destination() {
    let owner = Pubkey::from([1; 32]);
    let temp = Pubkey::from([2; 32]);
    let mint = crate::models::WSOL_MINT.parse().unwrap();
    let ata = get_associated_token_address_with_program_id(&owner, &mint, &spl_token_interface::ID);
    let amount = 1_234_567_890;
    let instructions = canonical_wsol_instructions(
        &owner,
        Direction::Unwrap,
        amount,
        amount + 1,
        true,
        2_039_280,
        Some(temp),
    )
    .unwrap();
    assert_eq!(instructions.len(), 4);
    assert_eq!(
        instructions[0].program_id,
        solana_system_interface::program::ID
    );
    assert_eq!(
        bincode::deserialize::<SystemInstruction>(&instructions[0].data).unwrap(),
        SystemInstruction::CreateAccount {
            lamports: 2_039_280,
            space: 165,
            owner: spl_token_interface::ID,
        }
    );
    assert_eq!(
        instructions[0].accounts,
        [AccountMeta::new(owner, true), AccountMeta::new(temp, true)]
    );
    for instruction in &instructions[1..] {
        assert_eq!(instruction.program_id, spl_token_interface::ID);
    }
    assert_eq!(
        TokenInstruction::unpack(&instructions[1].data).unwrap(),
        TokenInstruction::InitializeAccount3 { owner }
    );
    assert_eq!(
        instructions[1].accounts,
        [
            AccountMeta::new(temp, false),
            AccountMeta::new_readonly(mint, false)
        ]
    );
    assert_eq!(
        TokenInstruction::unpack(&instructions[2].data).unwrap(),
        TokenInstruction::TransferChecked {
            amount,
            decimals: 9
        }
    );
    assert_eq!(
        instructions[2].accounts,
        [
            AccountMeta::new(ata, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new(temp, false),
            AccountMeta::new_readonly(owner, true)
        ]
    );
    assert_eq!(
        TokenInstruction::unpack(&instructions[3].data).unwrap(),
        TokenInstruction::CloseAccount
    );
    assert_eq!(
        instructions[3].accounts,
        [
            AccountMeta::new(temp, false),
            AccountMeta::new(owner, false),
            AccountMeta::new_readonly(owner, true)
        ]
    );
    let full = canonical_wsol_instructions(
        &owner,
        Direction::Unwrap,
        amount,
        amount,
        true,
        2_039_280,
        None,
    )
    .unwrap();
    assert_eq!(
        full,
        [Instruction {
            program_id: spl_token_interface::ID,
            accounts: vec![
                AccountMeta::new(ata, false),
                AccountMeta::new(owner, false),
                AccountMeta::new_readonly(owner, true)
            ],
            data: TokenInstruction::CloseAccount.pack(),
        }]
    );
}

#[test]
fn wsol_authorization_binds_all_thirteen_quote_fields_and_exact_expiry_boundary() {
    let prepared = unsigned_case(Direction::Wrap, 50, 100, true);
    for field in 0..13 {
        let mut wrong = prepared.authorization.quote.clone();
        match field {
            0 => wrong.quote_id.push('x'),
            1 => wrong.wallet_id.push('x'),
            2 => wrong.wallet_pubkey.push('x'),
            3 => wrong.owner_type.push('x'),
            4 => wrong.owner_id.push('x'),
            5 => wrong.direction = Direction::Unwrap,
            6 => wrong.amount_lamports += 1,
            7 => wrong.source_balance_lamports += 1,
            8 => wrong.ata_exists = !wrong.ata_exists,
            9 => wrong.estimated_network_fee_lamports += 1,
            10 => wrong.temporary_rent_lamports += 1,
            11 => wrong.rent_refund_lamports += 1,
            12 => wrong.expires_at_ms += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            prepared
                .validate(&prepared.owner, &wrong, 1)
                .unwrap_err()
                .to_string(),
            "prepared WSOL conversion does not match requested quote",
            "field {field}"
        );
    }
    let quote = &prepared.authorization.quote;
    prepared
        .validate(&prepared.owner, quote, quote.expires_at_ms - 1)
        .unwrap();
    for now in [quote.expires_at_ms, u64::MAX] {
        assert_eq!(
            prepared
                .validate(&prepared.owner, quote, now)
                .unwrap_err()
                .to_string(),
            "conversion quote expired"
        );
    }
}

#[test]
fn wsol_authorization_rejects_every_instruction_data_program_and_account_index_mutation() {
    for (direction, amount, balance, exists) in SHAPES {
        let mut prepared = unsigned_case(direction, amount, balance, exists);
        validate(&prepared).unwrap();
        let original = prepared.message.clone();
        for index in 0..original.instructions.len() {
            for byte in 0..original.instructions[index].data.len() {
                prepared.message.instructions[index].data[byte] ^= 1;
                assert_eq!(
                    validate(&prepared).unwrap_err().to_string(),
                    "WSOL conversion message does not match authorized instructions"
                );
                prepared.message = original.clone();
            }
            prepared.message.instructions[index].program_id_index ^= 1;
            assert!(validate(&prepared).is_err());
            prepared.message = original.clone();
            for account in 0..original.instructions[index].accounts.len() {
                prepared.message.instructions[index].accounts[account] ^= 1;
                assert!(validate(&prepared).is_err());
                prepared.message = original.clone();
            }
        }
    }
}

#[test]
fn wsol_authorization_rejects_account_privilege_blockhash_and_instruction_list_changes() {
    for (direction, amount, balance, exists) in SHAPES {
        let mut prepared = unsigned_case(direction, amount, balance, exists);
        let original = prepared.message.clone();
        for key in 0..original.account_keys.len() {
            prepared.message.account_keys[key] = Pubkey::from([99; 32]);
            assert!(validate(&prepared).is_err());
            prepared.message = original.clone();
        }
        for mutation in 0..8 {
            match mutation {
                0 => prepared.message.header.num_readonly_signed_accounts += 1,
                1 => prepared.message.header.num_readonly_unsigned_accounts += 1,
                2 => {
                    prepared.message.recent_blockhash =
                        solana_sdk::hash::Hash::new_from_array([99; 32])
                }
                3 => prepared.message.account_keys.push(Pubkey::from([99; 32])),
                4 => {
                    prepared.message.instructions.pop();
                }
                5 => prepared
                    .message
                    .instructions
                    .push(original.instructions[0].clone()),
                6 => prepared.message.instructions.clear(),
                _ => {
                    if prepared.message.instructions.len() > 1 {
                        prepared.message.instructions.swap(0, 1);
                    } else {
                        prepared.message.instructions[0].data.push(0);
                    }
                }
            }
            assert!(validate(&prepared).is_err(), "mutation {mutation}");
            prepared.message = original.clone();
        }
    }
}

#[test]
fn wsol_authorization_requires_original_temporary_signer_and_disallows_extra_or_alias_signers() {
    let mut partial = unsigned_case(Direction::Unwrap, 50, 100, true);
    partial.temporary_signer = Some(Keypair::new());
    assert!(validate(&partial).is_err());
    partial.temporary_signer = None;
    assert!(validate(&partial).is_err());
    let mut wrap = unsigned_case(Direction::Wrap, 50, 0, false);
    wrap.temporary_signer = Some(Keypair::new());
    assert!(validate(&wrap).is_err());
    let owner = Pubkey::from([1; 32]);
    let mint = crate::models::WSOL_MINT.parse().unwrap();
    let ata = get_associated_token_address_with_program_id(&owner, &mint, &spl_token_interface::ID);
    for temp in [None, Some(owner), Some(ata)] {
        assert!(canonical_wsol_instructions(
            &owner,
            Direction::Unwrap,
            50,
            100,
            true,
            2_039_280,
            temp
        )
        .is_err());
    }
    for direction in [Direction::Wrap, Direction::Unwrap] {
        assert!(canonical_wsol_instructions(
            &owner,
            direction,
            100,
            100,
            true,
            2_039_280,
            Some(Pubkey::from([2; 32]))
        )
        .is_err());
    }
}

#[test]
fn wsol_authorization_retains_exact_fee_rent_reserve_and_overflow_budget_rules() {
    for (direction, amount, balance, exists) in SHAPES {
        let mut prepared = unsigned_case(direction, amount, balance, exists);
        let auth = &mut prepared.authorization;
        let rent = if (direction == Direction::Wrap && !exists)
            || (direction == Direction::Unwrap && amount < balance)
        {
            2_039_280
        } else {
            0
        };
        let required = 5_000
            + rent
            + 100_000
            + if direction == Direction::Wrap {
                amount
            } else {
                0
            };
        auth.sol_balance_lamports = required;
        if direction == Direction::Wrap {
            auth.quote.source_balance_lamports = required;
        }
        auth.validate_budget().unwrap();
        auth.sol_balance_lamports -= 1;
        if direction == Direction::Wrap {
            auth.quote.source_balance_lamports -= 1;
        }
        assert_eq!(
            auth.validate_budget().unwrap_err().to_string(),
            "SOL balance insufficient for conversion fee, rent and reserve"
        );
        auth.sol_balance_lamports += 1;
        if direction == Direction::Wrap {
            auth.quote.source_balance_lamports += 1;
        }
        auth.network_fee_lamports = auth.quote.estimated_network_fee_lamports + 1;
        assert_eq!(
            auth.validate_budget().unwrap_err().to_string(),
            "network fee increased; request a new quote"
        );
        auth.network_fee_lamports = u64::MAX;
        auth.quote.estimated_network_fee_lamports = u64::MAX;
        assert_eq!(
            auth.validate_budget().unwrap_err().to_string(),
            "conversion SOL budget overflow"
        );
    }
}

#[test]
fn wsol_authorization_rejects_changed_source_temporary_rent_refund_and_zero_or_excess_amount() {
    for (direction, amount, balance, exists) in SHAPES {
        for field in 0..5 {
            let mut prepared = unsigned_case(direction, amount, balance, exists);
            let auth = &mut prepared.authorization;
            match field {
                0 => auth.quote.source_balance_lamports += 1,
                1 => auth.quote.temporary_rent_lamports += 1,
                2 => auth.quote.rent_refund_lamports += 1,
                3 => auth.quote.amount_lamports = 0,
                _ => auth.quote.amount_lamports = auth.quote.source_balance_lamports + 1,
            }
            assert!(validate(&prepared).is_err(), "{direction:?} field {field}");
        }
    }
}

#[tokio::test]
async fn wsol_authorization_rejects_bad_or_expired_messages_before_unsigned_rpc() {
    use solana_client::{
        client_error::Result as RpcResult,
        rpc_client::RpcClientConfig,
        rpc_request::RpcRequest,
        rpc_sender::{RpcSender, RpcTransportStats},
    };
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };
    struct RejectRpc(Arc<AtomicUsize>);
    impl RpcSender for RejectRpc {
        fn send<'life0, 'async_trait>(
            &'life0 self,
            _: RpcRequest,
            _: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = RpcResult<serde_json::Value>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                self.0.fetch_add(1, Ordering::SeqCst);
                panic!("an unauthorized message must not reach memory RPC");
            })
        }
        fn get_transport_stats(&self) -> RpcTransportStats {
            RpcTransportStats::default()
        }
        fn url(&self) -> String {
            "memory://reject-unauthorized-wsol".into()
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let rpc = RpcClient::new_sender(RejectRpc(calls.clone()), RpcClientConfig::default());
    for (direction, amount, balance, exists) in SHAPES {
        let mut prepared = unsigned_case(direction, amount, balance, exists);
        prepared.message.instructions[0].data.push(0);
        assert_eq!(
            prepared
                .simulate(&rpc, &prepared.authorization.quote)
                .await
                .unwrap_err()
                .to_string(),
            "WSOL conversion message does not match authorized instructions"
        );
        let mut prepared = unsigned_case(direction, amount, balance, exists);
        prepared.authorization.quote.expires_at_ms = 0;
        assert_eq!(
            prepared
                .simulate(&rpc, &prepared.authorization.quote)
                .await
                .unwrap_err()
                .to_string(),
            "conversion quote expired"
        );
        let prepared = unsigned_case(direction, amount, balance, exists);
        let mut other = prepared.authorization.quote.clone();
        other.quote_id.push('x');
        assert_eq!(
            prepared
                .simulate(&rpc, &other)
                .await
                .unwrap_err()
                .to_string(),
            "prepared WSOL conversion does not match requested quote"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
