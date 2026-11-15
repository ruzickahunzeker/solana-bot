use anyhow::{bail, Result};
use solana_sdk::{hash::Hash, message::Message, pubkey::Pubkey};

use super::instructions::canonical_wsol_instructions;
use crate::application::wsol_failure::WsolFailure;
use crate::application::wsol_submission_types::{WsolDirection, WsolQuoteSnapshot};

/// Evidence retained from the original preparation, not refreshed by signing or simulation.
pub(super) struct WsolAuthorization {
    pub(super) quote: WsolQuoteSnapshot,
    pub(super) sol_balance_lamports: u64,
    pub(super) wsol_balance_lamports: u64,
    pub(super) account_rent_lamports: u64,
    pub(super) rent_refund_lamports: u64,
    pub(super) network_fee_lamports: u64,
    pub(super) blockhash: Hash,
}

impl WsolAuthorization {
    pub(super) fn validate_budget(&self) -> Result<()> {
        let quote = &self.quote;
        let unwrap = quote.direction == WsolDirection::Unwrap;
        quote
            .validate_current_balance(if unwrap {
                self.wsol_balance_lamports
            } else {
                self.sol_balance_lamports
            })
            .map_err(WsolFailure::rejected)?;
        let partial_unwrap = unwrap && quote.amount_lamports < self.wsol_balance_lamports;
        let temporary_rent = if partial_unwrap {
            self.account_rent_lamports
        } else {
            0
        };
        if temporary_rent != quote.temporary_rent_lamports {
            bail!(WsolFailure::rejected(
                "temporary rent changed; request a new quote"
            ));
        }
        let refund = if unwrap && !partial_unwrap {
            self.rent_refund_lamports
        } else {
            0
        };
        if refund != quote.rent_refund_lamports {
            bail!(WsolFailure::rejected(
                "WSOL account rent changed; request a new quote"
            ));
        }
        if self.network_fee_lamports > quote.estimated_network_fee_lamports {
            bail!(WsolFailure::rejected(
                "network fee increased; request a new quote"
            ));
        }
        let rent_funding = if !unwrap && !quote.ata_exists {
            self.account_rent_lamports
        } else {
            temporary_rent
        };
        let required_sol = self
            .network_fee_lamports
            .checked_add(rent_funding)
            .and_then(|value| value.checked_add(100_000))
            .and_then(|value| {
                if unwrap {
                    Some(value)
                } else {
                    value.checked_add(quote.amount_lamports)
                }
            })
            .ok_or_else(|| WsolFailure::rejected("conversion SOL budget overflow"))?;
        if self.sol_balance_lamports < required_sol {
            bail!(WsolFailure::rejected(
                "SOL balance insufficient for conversion fee, rent and reserve"
            ));
        }
        Ok(())
    }

    pub(super) fn validate(
        &self,
        wallet: &Pubkey,
        requested_quote: &WsolQuoteSnapshot,
        message: &Message,
        temporary_signer: Option<Pubkey>,
        now_ms: u64,
    ) -> Result<()> {
        if &self.quote != requested_quote {
            bail!(WsolFailure::integrity(
                "prepared WSOL conversion does not match requested quote"
            ));
        }
        let quote = &self.quote;
        quote
            .validate_binding(
                &requested_quote.wallet_id,
                &wallet.to_string(),
                &requested_quote.owner_type,
                &requested_quote.owner_id,
                now_ms,
            )
            .map_err(WsolFailure::rejected)?;
        self.validate_budget()?;
        let instructions = canonical_wsol_instructions(
            wallet,
            quote.direction,
            quote.amount_lamports,
            self.wsol_balance_lamports,
            quote.ata_exists,
            self.account_rent_lamports,
            temporary_signer,
        )?;
        let expected = Message::new_with_blockhash(&instructions, Some(wallet), &self.blockhash);
        if *message != expected {
            bail!(WsolFailure::integrity(
                "WSOL conversion message does not match authorized instructions"
            ));
        }
        Ok(())
    }
}
