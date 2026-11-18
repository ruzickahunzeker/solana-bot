//! Validated post bank plus captured recipient metadata, not a fee-policy or signing permit.
use super::{fresh, token_amount, NativeQuoteProbe};
use crate::adapters::sdk::pump_curve_evidence::{
    native_funding::{NativeBankPhase, NativeFundingPlan, NativeFundingSnapshot},
    PumpCurveAccounts,
};
use crate::adapters::sdk::pumpapi_assets::{decode_data, MAX_ACCOUNT_BYTES};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use solana_client::rpc_response::{Response, RpcSimulateTransactionResult};
use solana_sdk::{message::Message, pubkey::Pubkey};

pub(super) struct RecipientMetadata {
    pub(super) key: Pubkey,
    pub(super) owner: Pubkey,
    pub(super) lamports: u64,
    pub(super) data_len: usize,
}

pub(super) struct NativePostBank {
    pub(super) snapshot: NativeFundingSnapshot,
    // Capturing an owner does not prove that the protocol permits that owner.
    pub(super) recipients: [RecipientMetadata; 3],
}

impl NativeQuoteProbe {
    pub(super) fn inspect_post_bank(&self, response: &Value) -> Result<NativePostBank> {
        self.packet()?;
        ensure!(
            response.get("value").and_then(|value| value.get("err")) == Some(&Value::Null),
            "native Pump post bank requires explicit simulation success"
        );
        let response: Response<RpcSimulateTransactionResult> = Response::deserialize(response)
            .context("native Pump post bank response is malformed")?;
        ensure!(
            response.value.replacement_blockhash.is_none()
                && response
                    .value
                    .loaded_addresses
                    .as_ref()
                    .is_none_or(|loaded| loaded.writable.is_empty() && loaded.readonly.is_empty()),
            "native Pump post bank changed its fixed message context"
        );
        let values = response
            .value
            .accounts
            .as_ref()
            .context("native Pump post bank omitted accounts")?;
        ensure!(
            values.len() == 11,
            "native Pump post bank account count differs"
        );
        let before = &self.funding;
        let plan = NativeFundingPlan::new(
            PumpCurveAccounts::new(before.keys[1], Some(before.keys[0]), before.curve.quote)?,
            before.keys[6],
            before.curve.base_mint_layout.program,
            before.side,
        )?;
        ensure!(
            plan.keys() == &before.keys,
            "native Pump post bank canonical roles differ"
        );
        let mut post = plan.decode_bank(
            plan.keys(),
            response.context.slot,
            before.curve.slot,
            &values[..8],
            NativeBankPhase::AfterSimulation,
        )?;
        // Post decoding must not renew the original preparation window.
        post.started = before.started;
        ensure!(
            post.curve.base_mint_layout.decimals == before.curve.base_mint_layout.decimals
                && post.curve.base_mint_layout.supply == before.curve.base_mint_layout.supply
                && post.curve.curve.total_supply == before.curve.curve.total_supply
                && post.curve.curve.total_supply == post.curve.base_mint_layout.supply,
            "native Pump post bank mint precision or supply changed"
        );
        post.inspect_native_destinations(&self.instruction)?;
        ensure!(
            Message::new_with_blockhash(
                &[self.instruction.clone()],
                Some(&before.keys[6]),
                &self.transaction.message.recent_blockhash
            ) == self.transaction.message,
            "native Pump frozen instruction and packet differ"
        );
        let keys = &self.transaction.message.account_keys;
        let index = |key| {
            keys.iter()
                .position(|actual| *actual == key)
                .context("native Pump post bank role is not in message")
        };
        let native = response
            .value
            .post_balances
            .as_ref()
            .context("native Pump post bank omitted native balances")?;
        ensure!(
            native.len() == keys.len(),
            "native Pump post bank native index count differs"
        );
        for (i, key) in post.keys.iter().enumerate() {
            if let Some(index) = keys.iter().position(|actual| actual == key) {
                let account = values[i]
                    .as_ref()
                    .context("native Pump post bank message account is missing")?;
                ensure!(
                    account.lamports == native[index],
                    "native Pump post bank lamports contradict indexed balances"
                );
            }
        }
        let tokens = response
            .value
            .post_token_balances
            .as_ref()
            .context("native Pump post bank omitted token balances")?;
        ensure!(
            tokens.len() == 2,
            "native Pump post bank token index count differs"
        );
        let mint = post.keys[1];
        let decimals = post.curve.base_mint_layout.decimals;
        ensure!(
            Some(token_amount(
                tokens,
                index(post.keys[5])?,
                mint,
                post.keys[6],
                decimals
            )?) == post.user_base_units
                && token_amount(tokens, index(post.keys[4])?, mint, post.keys[0], decimals)?
                    == post.curve_base_units,
            "native Pump post bank token data contradict indexed balances"
        );
        let buy = post.side == super::Side::Buy;
        let positions = [
            1,
            if buy { 9 } else { 8 },
            self.instruction.accounts.len() - 1,
        ];
        let recipient = |i: usize| -> Result<RecipientMetadata> {
            let key = self.instruction.accounts[positions[i - 8]].pubkey;
            let account = values[i]
                .as_ref()
                .context("native Pump post recipient is missing")?;
            ensure!(
                account.lamports == native[index(key)?],
                "native Pump post recipient lamports contradict indexed balances"
            );
            let owner: Pubkey = account
                .owner
                .parse()
                .context("native Pump post recipient owner is malformed")?;
            let data = decode_data(
                account,
                owner,
                MAX_ACCOUNT_BYTES,
                "native Pump post recipient metadata",
            )?;
            Ok(RecipientMetadata {
                key,
                owner,
                lamports: account.lamports,
                data_len: data.len(),
            })
        };
        let recipients = [recipient(8)?, recipient(9)?, recipient(10)?];
        fresh(before.started)?;
        Ok(NativePostBank {
            snapshot: post,
            recipients,
        })
    }
}

#[cfg(test)]
pub(super) mod tests;
