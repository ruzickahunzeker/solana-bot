use crate::application::wsol_failure::WsolFailure;
use crate::application::wsol_submission_types::WsolDirection;
use anyhow::{bail, Result};
use solana_program_pack::Pack;
use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};
use solana_system_interface::instruction as system_instruction;
use spl_associated_token_account_interface::{
    address::get_associated_token_address_with_program_id,
    instruction::create_associated_token_account_idempotent,
};
use spl_token_interface::instruction as token_instruction;
pub(super) struct WsolInstructions {
    pub(super) instructions: Vec<Instruction>,
    pub(super) temporary_account: Option<Keypair>,
    pub(super) temporary_rent_lamports: u64,
}

pub(super) fn build_wsol_instructions(
    owner: &Pubkey,
    direction: WsolDirection,
    amount_lamports: u64,
    wsol_balance_lamports: u64,
    ata_exists: bool,
    token_account_rent_lamports: u64,
) -> Result<WsolInstructions> {
    let temporary_account = (direction == WsolDirection::Unwrap
        && ata_exists
        && amount_lamports > 0
        && amount_lamports < wsol_balance_lamports)
        .then(Keypair::new);
    let instructions = canonical_wsol_instructions(
        owner,
        direction,
        amount_lamports,
        wsol_balance_lamports,
        ata_exists,
        token_account_rent_lamports,
        temporary_account.as_ref().map(Signer::pubkey),
    )?;
    Ok(WsolInstructions {
        instructions,
        temporary_rent_lamports: if temporary_account.is_some() {
            token_account_rent_lamports
        } else {
            0
        },
        temporary_account,
    })
}

pub(super) fn canonical_wsol_instructions(
    owner: &Pubkey,
    direction: WsolDirection,
    amount_lamports: u64,
    wsol_balance_lamports: u64,
    ata_exists: bool,
    token_account_rent_lamports: u64,
    temporary_account: Option<Pubkey>,
) -> Result<Vec<Instruction>> {
    if amount_lamports == 0 {
        bail!(WsolFailure::rejected("conversion amount must be positive"));
    }
    let mint = crate::models::WSOL_MINT.parse::<Pubkey>()?;
    let token_program = spl_token_interface::ID;
    let ata = get_associated_token_address_with_program_id(owner, &mint, &token_program);
    match direction {
        WsolDirection::Wrap => {
            if temporary_account.is_some() {
                bail!("WSOL wrap does not authorize a temporary signer");
            }
            let mut instructions = Vec::new();
            if !ata_exists {
                instructions.push(create_associated_token_account_idempotent(
                    owner,
                    owner,
                    &mint,
                    &token_program,
                ));
            }
            instructions.push(system_instruction::transfer(owner, &ata, amount_lamports));
            instructions.push(token_instruction::sync_native(&token_program, &ata)?);
            Ok(instructions)
        }
        WsolDirection::Unwrap => {
            if !ata_exists || amount_lamports > wsol_balance_lamports {
                bail!(WsolFailure::rejected(
                    "WSOL balance insufficient for requested unwrap"
                ));
            }
            if amount_lamports == wsol_balance_lamports {
                if temporary_account.is_some() {
                    bail!("full WSOL unwrap does not authorize a temporary signer");
                }
                return Ok(vec![token_instruction::close_account(
                    &token_program,
                    &ata,
                    owner,
                    owner,
                    &[],
                )?]);
            }
            let temp = temporary_account.ok_or_else(|| {
                anyhow::anyhow!("partial WSOL unwrap requires the original temporary signer")
            })?;
            if temp == *owner || temp == ata {
                bail!("WSOL temporary signer must not alias the wallet or source account");
            }
            let instructions = vec![
                system_instruction::create_account(
                    owner,
                    &temp,
                    token_account_rent_lamports,
                    spl_token_interface::state::Account::LEN as u64,
                    &token_program,
                ),
                token_instruction::initialize_account3(&token_program, &temp, &mint, owner)?,
                token_instruction::transfer_checked(
                    &token_program,
                    &ata,
                    &mint,
                    &temp,
                    owner,
                    &[],
                    amount_lamports,
                    9,
                )?,
                token_instruction::close_account(&token_program, &temp, owner, owner, &[])?,
            ];
            Ok(instructions)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WsolDirection;
    use solana_sdk::signature::Signer;

    #[test]
    fn wsol_canonical_messages_match_frozen_legacy_bytes() {
        use sha2::{Digest, Sha256};
        use solana_sdk::{hash::Hash, message::Message, pubkey::Pubkey};
        let owner = Pubkey::from([1; 32]);
        let temp_fixture = Pubkey::from([2; 32]);
        for ((direction, amount, balance, exists), expected) in [
            (WsolDirection::Wrap, 50, 0, false),
            (WsolDirection::Wrap, 50, 100, true),
            (WsolDirection::Unwrap, 100, 100, true),
            (WsolDirection::Unwrap, 50, 100, true),
        ]
        .into_iter()
        .zip([
            "1a33609bff7171f17ea0fd36b73eab48b9aeb8543167d9aaafc6a0b8dde99c35",
            "feb624c684eec9c429d2f82ab4de169e77b9c5c9cedea7926fd42e2f42706131",
            "33b2142920b47f13fb5e7ece8fff2a87f4c20f8331ac918e0a880ff35c7d3944",
            "7df1a54a1f96f1a43410ff1315822883fa27e10514d663ef5b55d8547b4a9342",
        ]) {
            let mut built = super::build_wsol_instructions(
                &owner, direction, amount, balance, exists, 2_039_280,
            )
            .unwrap();
            if let Some(temp) = &built.temporary_account {
                for instruction in &mut built.instructions {
                    for account in &mut instruction.accounts {
                        if account.pubkey == temp.pubkey() {
                            account.pubkey = temp_fixture;
                        }
                    }
                }
            }
            let message = Message::new_with_blockhash(
                &built.instructions,
                Some(&owner),
                &Hash::new_from_array([8; 32]),
            );
            assert_eq!(
                format!(
                    "{:x}",
                    Sha256::digest(bincode::serialize(&message).unwrap())
                ),
                expected
            );
        }
    }
    #[test]
    fn partial_unwrap_preserves_source_account_and_full_unwrap_closes_it() {
        let owner = solana_sdk::pubkey::Pubkey::new_unique();
        let partial =
            super::build_wsol_instructions(&owner, WsolDirection::Unwrap, 50, 100, true, 2_039_280)
                .unwrap();
        assert_eq!(partial.instructions.len(), 4);
        assert!(partial.temporary_account.is_some());
        let temporary_pubkey = partial.temporary_account.as_ref().unwrap().pubkey();
        assert_eq!(partial.instructions[3].accounts[0].pubkey, temporary_pubkey);
        assert_eq!(partial.temporary_rent_lamports, 2_039_280);
        let full = super::build_wsol_instructions(
            &owner,
            WsolDirection::Unwrap,
            100,
            100,
            true,
            2_039_280,
        )
        .unwrap();
        assert_eq!(full.instructions.len(), 1);
        assert!(full.temporary_account.is_none());
        let ata = spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(
            &owner,
            &crate::models::WSOL_MINT.parse().unwrap(),
            &spl_token_interface::ID,
        );
        assert_eq!(full.instructions[0].accounts[0].pubkey, ata);
        assert_eq!(full.temporary_rent_lamports, 0);
    }

    #[test]
    fn wrap_creates_ata_only_when_missing_and_unwrap_rejects_overdraft() {
        let owner = solana_sdk::pubkey::Pubkey::new_unique();
        let new_ata = super::build_wsol_instructions(
            &owner,
            WsolDirection::Wrap,
            1_000_000,
            0,
            false,
            2_039_280,
        )
        .unwrap();
        assert_eq!(new_ata.instructions.len(), 3);
        let existing_ata = super::build_wsol_instructions(
            &owner,
            WsolDirection::Wrap,
            1_000_000,
            0,
            true,
            2_039_280,
        )
        .unwrap();
        assert_eq!(existing_ata.instructions.len(), 2);
        assert!(super::build_wsol_instructions(
            &owner,
            WsolDirection::Unwrap,
            101,
            100,
            true,
            2_039_280,
        )
        .is_err());
    }
}
