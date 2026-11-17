use super::{build_wsol_instructions, UnsignedWsolTransaction, WsolAuthorization};
use crate::{
    adapters::rpc::{
        read_http::ReadHttpTransport, read_runtime::SharedReadRuntime,
        wsol_quote_reads::WsolQuoteReadSession,
    },
    application::{
        wsol_failure::WsolFailure,
        wsol_submission_types::{WsolDirection as Direction, WsolQuoteSnapshot},
    },
};
use anyhow::{bail, Result};
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::{message::Message, pubkey::Pubkey};

pub(crate) async fn prepare<T: ReadHttpTransport>(
    owner: Pubkey,
    runtime: &SharedReadRuntime<T>,
    url: &str,
    commitment: CommitmentConfig,
    quote: &WsolQuoteSnapshot,
) -> Result<UnsignedWsolTransaction> {
    if owner.to_string() != quote.wallet_pubkey {
        bail!(WsolFailure::integrity(
            "loaded wallet signer does not match quote"
        ));
    }
    let mint = crate::models::WSOL_MINT.parse::<Pubkey>()?;
    let token_program = spl_token_interface::ID;
    // A fresh scope rechecks state; no quote-time account/hash/fee observation is reused.
    let mut reads = WsolQuoteReadSession::new(runtime, url, commitment, owner)?;
    let sol_balance = reads.balance().await?;
    let ata_account = reads.ata().await?;
    if ata_account.is_some() != quote.ata_exists {
        bail!(WsolFailure::rejected(
            "WSOL account state changed; request a new quote"
        ));
    }
    let wsol_balance = match ata_account.as_ref() {
        Some(account) => {
            if account.owner != token_program {
                bail!(WsolFailure::integrity("WSOL ATA program owner changed"));
            }
            let token = spl_token_interface::state::Account::unpack(&account.data)
                .map_err(WsolFailure::integrity)?;
            if token.mint != mint || token.owner != owner || token.is_native.is_none() {
                bail!(WsolFailure::integrity("WSOL ATA mint or owner changed"));
            }
            token.amount
        }
        None => 0,
    };
    let current_refund = ata_account
        .as_ref()
        .map(|account| account.lamports.saturating_sub(wsol_balance))
        .unwrap_or(0);
    if quote.direction == Direction::Unwrap && quote.amount_lamports == wsol_balance {
        if current_refund != quote.rent_refund_lamports {
            bail!(WsolFailure::rejected(
                "WSOL account rent changed; request a new quote"
            ));
        }
    }
    let current_source_balance = match quote.direction {
        Direction::Wrap => sol_balance,
        Direction::Unwrap => wsol_balance,
    };
    quote
        .validate_current_balance(current_source_balance)
        .map_err(WsolFailure::rejected)?;
    let account_rent = reads.rent().await?;
    let built = build_wsol_instructions(
        &owner,
        quote.direction,
        quote.amount_lamports,
        wsol_balance,
        ata_account.is_some(),
        account_rent,
    )?;
    if built.temporary_rent_lamports != quote.temporary_rent_lamports {
        bail!(WsolFailure::rejected(
            "temporary rent changed; request a new quote"
        ));
    }
    let blockhash = reads.blockhash().await?;
    let mut message = Message::new(&built.instructions, Some(&owner));
    message.recent_blockhash = blockhash;
    let fee = reads.fee(&message).await?;
    let authorization = WsolAuthorization {
        quote: quote.clone(),
        sol_balance_lamports: sol_balance,
        wsol_balance_lamports: wsol_balance,
        account_rent_lamports: account_rent,
        rent_refund_lamports: current_refund,
        network_fee_lamports: fee,
        blockhash,
    };
    authorization.validate_budget()?;
    Ok(UnsignedWsolTransaction {
        message,
        owner,
        temporary_signer: built.temporary_account,
        authorization,
    })
}

#[cfg(test)]
pub(crate) mod tests;
