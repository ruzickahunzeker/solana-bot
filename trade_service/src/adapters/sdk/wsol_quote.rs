use crate::{
    adapters::rpc::{
        read_http::ReadHttpTransport, read_runtime::SharedReadRuntime,
        wsol_quote_reads::WsolQuoteReadSession,
    },
    application::{
        wsol_failure::WsolFailure,
        wsol_queries::{WsolQuoteEstimate, WsolQuoteInput},
        wsol_submission_types::WsolDirection,
    },
};
use anyhow::{bail, Result};
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;

pub(crate) async fn estimate<T: ReadHttpTransport>(
    runtime: &SharedReadRuntime<T>,
    url: &str,
    commitment: CommitmentConfig,
    owner: Pubkey,
    request: &WsolQuoteInput,
) -> Result<WsolQuoteEstimate> {
    let mut reads = WsolQuoteReadSession::new(runtime, url, commitment, owner)?;
    let mint = crate::models::WSOL_MINT.parse::<Pubkey>()?;
    let token_program = spl_token_interface::ID;
    let sol_balance = reads.balance().await?;
    let ata_account = reads.ata().await?;
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
    let account_rent = reads.rent().await?;
    let (mut message, temporary_rent_lamports) = crate::sdk_client::wsol_conversion::quote_message(
        &owner,
        request.direction,
        amount,
        wsol_balance,
        ata_account.is_some(),
        account_rent,
    )?;
    message.recent_blockhash = reads.blockhash().await?;
    let fee = reads.fee(&message).await?;
    let rent_funding = match request.direction {
        WsolDirection::Wrap if ata_account.is_none() => account_rent,
        WsolDirection::Unwrap => temporary_rent_lamports,
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
        temporary_rent_lamports,
        rent_refund_lamports: if request.direction == WsolDirection::Unwrap
            && amount == wsol_balance
        {
            source_rent
        } else {
            0
        },
    })
}

#[cfg(test)]
mod tests;
