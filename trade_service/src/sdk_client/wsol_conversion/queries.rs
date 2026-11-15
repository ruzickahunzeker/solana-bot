use super::instructions::build_wsol_instructions;
use crate::application::{
    wsol_failure::WsolFailure,
    wsol_queries::{WsolQuoteEstimate, WsolQuoteInput},
    wsol_submission_types::WsolDirection,
};
use anyhow::{bail, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::{message::Message, pubkey::Pubkey};
use spl_associated_token_account_interface::address::get_associated_token_address_with_program_id;

pub(crate) async fn estimate(
    owner: Pubkey,
    rpc: &RpcClient,
    request: &WsolQuoteInput,
) -> Result<WsolQuoteEstimate> {
    let mint = crate::models::WSOL_MINT.parse::<Pubkey>()?;
    let token_program = spl_token_interface::ID;
    let ata = get_associated_token_address_with_program_id(&owner, &mint, &token_program);
    let sol_balance = rpc.get_balance(&owner).await?;
    let ata_account = rpc
        .get_account_with_commitment(&ata, CommitmentConfig::processed())
        .await?
        .value;
    let (wsol_balance, source_rent) = match ata_account.as_ref() {
        Some(account) => {
            if account.owner != token_program {
                bail!(WsolFailure::integrity(
                    "WSOL ATA has an unexpected program owner"
                ));
            }
            let token = spl_token_interface::state::Account::unpack(&account.data)
                .map_err(WsolFailure::integrity)?;
            if token.mint != mint || token.owner != owner || token.is_native.is_none() {
                bail!(WsolFailure::integrity(
                    "WSOL ATA mint or owner does not match wallet"
                ));
            }
            (token.amount, account.lamports.saturating_sub(token.amount))
        }
        None => (0, 0),
    };
    let source_balance = match request.direction {
        WsolDirection::Wrap => sol_balance,
        WsolDirection::Unwrap => wsol_balance,
    };
    let amount = if request.all {
        source_balance
    } else {
        request
            .amount_lamports
            .ok_or_else(|| WsolFailure::rejected("amount_lamports is required"))?
    };
    let account_rent = rpc
        .get_minimum_balance_for_rent_exemption(spl_token_interface::state::Account::LEN)
        .await?;
    let built = build_wsol_instructions(
        &owner,
        request.direction,
        amount,
        wsol_balance,
        ata_account.is_some(),
        account_rent,
    )?;
    let mut message = Message::new(&built.instructions, Some(&owner));
    message.recent_blockhash = rpc.get_latest_blockhash().await?;
    let fee = rpc.get_fee_for_message(&message).await?;
    let rent_funding = match request.direction {
        WsolDirection::Wrap if ata_account.is_none() => account_rent,
        WsolDirection::Unwrap => built.temporary_rent_lamports,
        _ => 0,
    };
    let sol_spend = fee
        .checked_add(rent_funding)
        .and_then(|value| value.checked_add(100_000))
        .and_then(|value| {
            if request.direction == WsolDirection::Wrap {
                value.checked_add(amount)
            } else {
                Some(value)
            }
        })
        .ok_or_else(|| WsolFailure::rejected("conversion SOL budget overflow"))?;
    if sol_balance < sol_spend {
        bail!(WsolFailure::rejected(
            "SOL balance insufficient for amount, fee, rent and reserve"
        ));
    }
    Ok(WsolQuoteEstimate {
        amount_lamports: amount,
        source_balance_lamports: source_balance,
        ata_exists: ata_account.is_some(),
        estimated_network_fee_lamports: fee,
        temporary_rent_lamports: built.temporary_rent_lamports,
        rent_refund_lamports: if request.direction == WsolDirection::Unwrap
            && amount == wsol_balance
        {
            source_rent
        } else {
            0
        },
    })
}
