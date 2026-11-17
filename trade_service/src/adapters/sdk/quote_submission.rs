use anyhow::{ensure, Result};
use solana_sdk::{pubkey::Pubkey, signature::Signature, transaction::VersionedTransaction};

use crate::aggregator_quote;
use crate::application::ports::quote_execution::OrdinaryQuoteBackend;
use crate::application::ports::quote_execution::QuoteSubmissionBackend;
use crate::domain::quote::{BoundQuote, QuotePreflight};
use crate::domain::submission::SignedSubmission;
use crate::sdk_client::SdkTradeClient;

pub(crate) struct SdkQuoteSubmission<'a>(
    pub &'a SdkTradeClient,
    pub &'a crate::adapters::rpc::read_runtime::SharedReadRuntime,
);

/// Only this adapter can construct one after prepare/audit; callers cannot edit its message.
pub(crate) struct PreparedQuoteTransaction {
    transaction: VersionedTransaction,
    wallet_pubkey: String,
    expires_at_ms: u64,
    signable: bool,
}

impl PreparedQuoteTransaction {
    fn from_preflight(
        transaction: VersionedTransaction,
        bound: &BoundQuote,
        preflight: &QuotePreflight,
    ) -> Self {
        Self {
            transaction,
            wallet_pubkey: bound.wallet_pubkey.clone(),
            expires_at_ms: bound.expires_at_ms,
            signable: preflight.quote_id == bound.quote_id
                && preflight.wallet_id == bound.wallet_id
                && preflight.quote_still_valid
                && preflight.unsigned_transaction_built
                && preflight.aggregator_simulation_ok
                && preflight.local_simulation_ok
                && preflight.instruction_audit_passed
                && preflight.executable,
        }
    }

    fn ensure_signable(&self, wallet: &Pubkey) -> Result<()> {
        ensure!(
            self.signable,
            "aggregator preflight did not produce an executable transaction"
        );
        ensure!(
            wallet.to_string() == self.wallet_pubkey,
            "prepared aggregator transaction belongs to a different wallet"
        );
        ensure!(
            self.transaction.message.static_account_keys().first() == Some(wallet),
            "prepared aggregator payer does not match wallet"
        );
        ensure!(
            self.transaction.message.header().num_required_signatures == 1
                && self.transaction.signatures.len() == 1
                && self.transaction.signatures[0] == Signature::default(),
            "prepared aggregator transaction must be unsigned with exactly one wallet signer"
        );
        Ok(())
    }

    fn ensure_current_at(&self, wallet: &Pubkey, now_ms: u128) -> Result<()> {
        self.ensure_signable(wallet)?;
        ensure!(
            now_ms < u128::from(self.expires_at_ms),
            "aggregator quote expired before signing"
        );
        Ok(())
    }

    pub(crate) fn into_verified_message(
        self,
        wallet: &Pubkey,
    ) -> Result<solana_sdk::message::VersionedMessage> {
        self.ensure_current_at(wallet, crate::state::now_ms())?;
        Ok(self.transaction.message)
    }

    #[cfg(test)]
    pub(crate) fn unverified_for_policy_test(transaction: VersionedTransaction) -> Self {
        Self {
            transaction,
            wallet_pubkey: String::new(),
            expires_at_ms: 0,
            signable: false,
        }
    }
}

impl QuoteSubmissionBackend for SdkQuoteSubmission<'_> {
    type Prepared = PreparedQuoteTransaction;

    fn ensure_live_allowed(&self) -> Result<()> {
        self.0.ensure_broadcast_allowed()
    }

    async fn prepare(&self, bound: &BoundQuote) -> Result<(Self::Prepared, QuotePreflight)> {
        let (transaction, preflight) =
            aggregator_quote::prepare_transaction(bound, self.0, self.1).await?;
        let prepared = PreparedQuoteTransaction::from_preflight(transaction, bound, &preflight);
        Ok((prepared, preflight))
    }

    fn sign(&self, unsigned: Self::Prepared) -> Result<SignedSubmission> {
        self.0.sign_prepared_quote(unsigned)
    }

    async fn broadcast(&self, signed: &SignedSubmission) -> Result<String> {
        let transaction = bincode::deserialize::<VersionedTransaction>(&signed.bytes)?;
        ensure!(
            transaction
                .signatures
                .first()
                .map(ToString::to_string)
                .as_deref()
                == Some(&signed.signature),
            "persisted transaction signature mismatch"
        );
        Ok(self
            .0
            .send_aggregator_transaction(&transaction)
            .await?
            .to_string())
    }
}

impl OrdinaryQuoteBackend for SdkQuoteSubmission<'_> {
    fn prepared_message(&self, prepared: &Self::Prepared) -> Result<Vec<u8>> {
        prepared.ensure_signable(&self.0.wallet())?;
        Ok(prepared.transaction.message.serialize())
    }
    fn signed_message(&self, signed: &SignedSubmission) -> Result<Vec<u8>> {
        verified_signed_message(signed, &self.0.wallet())
    }
}

impl crate::application::ports::reconciliation::SignatureStatusReader for SdkQuoteSubmission<'_> {
    async fn lookup_signature(
        &self,
        signature: &str,
    ) -> Result<Option<crate::domain::reconciliation::ChainObservation>> {
        crate::adapters::rpc::signature_status::RpcSignatureStatusReader {
            rpcs: self.0.rpcs(),
        }
        .lookup_signature(signature)
        .await
    }
}

fn verified_signed_message(signed: &SignedSubmission, wallet: &Pubkey) -> Result<Vec<u8>> {
    let transaction: VersionedTransaction = bincode::deserialize(&signed.bytes)?;
    let message = transaction.message.serialize();
    ensure!(
        transaction.message.static_account_keys().first() == Some(wallet)
            && transaction.message.header().num_required_signatures == 1
            && transaction.signatures.len() == 1
            && transaction.signatures[0].to_string() == signed.signature
            && transaction.signatures[0].verify(wallet.as_ref(), &message),
        "ordinary signed transaction wallet or signature mismatch"
    );
    ensure!(
        bincode::serialize(&transaction)? == signed.bytes,
        "ordinary signed transaction is not canonical"
    );
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use solana_sdk::{
        instruction::Instruction,
        message::{Message, VersionedMessage},
    };

    fn fixture() -> (BoundQuote, QuotePreflight, Pubkey) {
        let value: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/contracts/sol_buy_v1.json"
        ))
        .unwrap();
        let mut bound: BoundQuote = serde_json::from_value(value["bound"].clone()).unwrap();
        let preflight: QuotePreflight = serde_json::from_value(value["preflight"].clone()).unwrap();
        let wallet = Pubkey::new_from_array([1; 32]);
        bound.wallet_pubkey = wallet.to_string();
        (bound, preflight, wallet)
    }

    fn unsigned(wallet: Pubkey) -> VersionedTransaction {
        let instruction = Instruction {
            program_id: Pubkey::new_from_array([2; 32]),
            accounts: vec![],
            data: vec![7],
        };
        VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::Legacy(Message::new(&[instruction], Some(&wallet))),
        }
    }

    #[test]
    fn expired_prepared_quote_cannot_release_a_message_to_the_signer() {
        let (mut bound, preflight, wallet) = fixture();
        bound.expires_at_ms = 0;
        let prepared =
            PreparedQuoteTransaction::from_preflight(unsigned(wallet), &bound, &preflight);
        let result = prepared.into_verified_message(&wallet);
        assert!(
            result.is_err(),
            "an expired quote must not release its message for signing"
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "aggregator quote expired before signing"
        );
    }

    #[test]
    fn prepared_quote_expiry_is_a_snapshot_with_an_exclusive_deadline() {
        let (mut bound, preflight, wallet) = fixture();
        bound.expires_at_ms = 100;
        let prepared =
            PreparedQuoteTransaction::from_preflight(unsigned(wallet), &bound, &preflight);
        bound.expires_at_ms = u64::MAX;
        assert_eq!(prepared.expires_at_ms, 100);
        for now in [0, 99] {
            assert!(prepared.ensure_current_at(&wallet, now).is_ok());
        }
        for now in [100, 101, u128::from(u64::MAX), u128::MAX] {
            assert_eq!(
                prepared
                    .ensure_current_at(&wallet, now)
                    .unwrap_err()
                    .to_string(),
                "aggregator quote expired before signing"
            );
        }
        let future = PreparedQuoteTransaction::from_preflight(unsigned(wallet), &bound, &preflight);
        assert!(future
            .ensure_current_at(&wallet, u128::from(u64::MAX) - 1)
            .is_ok());
        assert!(future
            .ensure_current_at(&wallet, u128::from(u64::MAX))
            .is_err());
    }

    #[test]
    fn unexpired_prepared_quote_releases_exact_legacy_or_v0_message_without_signing() {
        let (mut bound, preflight, wallet) = fixture();
        bound.expires_at_ms = u64::MAX;
        for versioned in [false, true] {
            let mut transaction = unsigned(wallet);
            if versioned {
                let VersionedMessage::Legacy(message) = transaction.message else {
                    unreachable!()
                };
                transaction.message = VersionedMessage::V0(solana_sdk::message::v0::Message {
                    header: message.header,
                    account_keys: message.account_keys,
                    recent_blockhash: message.recent_blockhash,
                    instructions: message.instructions,
                    address_table_lookups: vec![],
                });
            }
            let expected = transaction.message.serialize();
            assert_eq!(transaction.signatures, vec![Signature::default()]);
            let message = PreparedQuoteTransaction::from_preflight(transaction, &bound, &preflight)
                .into_verified_message(&wallet)
                .unwrap();
            assert_eq!(message.serialize(), expected);
        }
    }

    #[test]
    fn bound_wallet_and_unsigned_shape_are_checked_without_signer_access() {
        let (bound, preflight, wallet) = fixture();
        let prepared =
            PreparedQuoteTransaction::from_preflight(unsigned(wallet), &bound, &preflight);
        assert!(prepared.ensure_signable(&wallet).is_ok());
        assert!(prepared
            .ensure_signable(&Pubkey::new_from_array([3; 32]))
            .is_err());
        let other_payer = PreparedQuoteTransaction::from_preflight(
            unsigned(Pubkey::new_from_array([3; 32])),
            &bound,
            &preflight,
        );
        assert!(other_payer.ensure_signable(&wallet).is_err());
    }

    #[test]
    fn every_preflight_permission_is_required_even_if_application_misuses_sign() {
        let (bound, preflight, wallet) = fixture();
        for field in [
            "quote_still_valid",
            "unsigned_transaction_built",
            "aggregator_simulation_ok",
            "local_simulation_ok",
            "instruction_audit_passed",
            "executable",
        ] {
            let mut value = serde_json::to_value(&preflight).unwrap();
            value[field] = Value::Bool(false);
            let changed = serde_json::from_value(value).unwrap();
            let prepared =
                PreparedQuoteTransaction::from_preflight(unsigned(wallet), &bound, &changed);
            assert!(prepared.ensure_signable(&wallet).is_err(), "{field}");
        }
    }

    #[test]
    fn a_different_quote_or_wallet_preflight_cannot_authorize_the_message() {
        let (bound, preflight, wallet) = fixture();
        for field in ["quote_id", "wallet_id"] {
            let mut value = serde_json::to_value(&preflight).unwrap();
            value[field] = Value::String("different-binding".into());
            let changed = serde_json::from_value(value).unwrap();
            assert!(
                PreparedQuoteTransaction::from_preflight(unsigned(wallet), &bound, &changed)
                    .ensure_signable(&wallet)
                    .is_err()
            );
        }
    }

    #[test]
    fn prefilled_or_missing_signatures_and_multisigner_messages_are_rejected() {
        let (bound, preflight, wallet) = fixture();
        for variant in 0..4 {
            let mut transaction = unsigned(wallet);
            match variant {
                0 => transaction.signatures.clear(),
                1 => transaction.signatures[0] = Signature::from([9; 64]),
                2 => transaction.signatures.push(Signature::default()),
                3 => {
                    let VersionedMessage::Legacy(message) = &mut transaction.message else {
                        unreachable!()
                    };
                    message.header.num_required_signatures = 2;
                }
                _ => unreachable!(),
            }
            assert!(
                PreparedQuoteTransaction::from_preflight(transaction, &bound, &preflight)
                    .ensure_signable(&wallet)
                    .is_err()
            );
        }
    }

    #[test]
    fn ordinary_signed_verifier_rejects_invalid_bytes_without_access_to_a_signer() {
        let (_, _, wallet) = fixture();
        for variant in 0..7 {
            let mut transaction = unsigned(wallet);
            let mut signature = Signature::default().to_string();
            match variant {
                0 => {}
                1 => transaction.signatures.clear(),
                2 => transaction.signatures.push(Signature::default()),
                3 => {
                    let VersionedMessage::Legacy(message) = &mut transaction.message else {
                        unreachable!()
                    };
                    message.header.num_required_signatures = 2;
                }
                4 => transaction = unsigned(Pubkey::new_from_array([3; 32])),
                5 => signature = Signature::from([9; 64]).to_string(),
                6 => {
                    transaction.signatures[0] = Signature::from([9; 64]);
                    signature = transaction.signatures[0].to_string();
                }
                _ => unreachable!(),
            }
            let signed = SignedSubmission {
                signature,
                bytes: bincode::serialize(&transaction).unwrap(),
            };
            assert!(
                verified_signed_message(&signed, &wallet).is_err(),
                "{variant}"
            );
        }
        assert!(verified_signed_message(
            &SignedSubmission {
                signature: Signature::default().to_string(),
                bytes: vec![255],
            },
            &wallet,
        )
        .is_err());
    }
}
