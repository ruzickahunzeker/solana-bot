use crate::{
    adapters::{
        rpc::{ordinary_observation::SingleShotStatusReader, read_runtime::SharedReadRuntime},
        sdk::damm_probe::{DammBuyContext, DammBuySignedCandidate},
        sdk::damm_sell_message::{DammSellContext, DammSignedCandidate},
    },
    application::{ports::reconciliation::SignatureStatusReader, trade_admission::AdmittedTrade},
    domain::{reconciliation::ChainCommitment, submission::SignedSubmission},
};
use anyhow::{ensure, Context, Result};
use bincode::Options;
use sol_trade_sdk::{
    swqos::{SwqosClient, SwqosType, TradeType},
    TradingClient,
};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig};
use solana_commitment_config::CommitmentLevel;
use solana_sdk::{
    message::VersionedMessage, pubkey::Pubkey, signature::Signature,
    transaction::VersionedTransaction,
};
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::{str::FromStr, sync::Arc, time::Duration};

/// A keyless, fixed sender. It has no SDK build/sign queue or route fallback method.
pub(crate) struct OriginalSdkSender<R = SingleShotStatusReader> {
    sender: Arc<SwqosClient>,
    rpc: Arc<RpcClient>,
    observer: R,
    provider: SwqosType,
    check_min_tip: bool,
}

impl OriginalSdkSender {
    pub(crate) fn from_client(
        client: &TradingClient,
        provider: SwqosType,
        runtime: Arc<SharedReadRuntime>,
    ) -> Result<Self> {
        super::ordinary_lanes::ensure_sender_layout(
            provider,
            &client
                .infrastructure
                .swqos_clients
                .iter()
                .map(|sender| sender.get_swqos_type())
                .collect::<Vec<_>>(),
            client.middleware_manager.is_some(),
        )?;
        let rpc = client.get_rpc().clone();
        let observer = SingleShotStatusReader::with_selected_runtime(runtime, &rpc.url())?;
        Ok(Self {
            sender: client.infrastructure.swqos_clients[0].clone(),
            rpc,
            observer,
            provider,
            check_min_tip: client.check_min_tip,
        })
    }
}

impl<R: SignatureStatusReader> OriginalSdkSender<R> {
    pub(crate) async fn broadcast_pump(
        &self,
        execution: &AdmittedTrade,
        context: super::pump_prepared::PumpPreparedContext,
        artifact: &super::pump_prepared::PumpSignedCandidate<'_>,
        signed: &SignedSubmission,
        wait_confirmed: bool,
        verification_ms: u64,
    ) -> Result<String> {
        ensure!(
            context.provider == self.provider && self.sender.get_swqos_type() == self.provider,
            "native Pump original sender provider changed"
        );
        artifact.check_submission(execution, context, signed)?;
        let direction = if execution.command().amount.side() == crate::domain::trade::Side::Buy {
            TradeType::Buy
        } else {
            TradeType::Sell
        };
        self.send_packet_for(direction, signed, wait_confirmed, verification_ms)
            .await
    }
    pub(crate) fn freeze_tip(&self, tip_sol: f64) -> Result<Option<Pubkey>> {
        ensure!(
            tip_sol.is_finite() && tip_sol >= 0.0,
            "SDK original tip is invalid"
        );
        let actual_tip = if self.provider == SwqosType::Default {
            0.0
        } else {
            tip_sol
        };
        let lamports = (actual_tip * 1_000_000_000.0).min(u64::MAX as f64).round() as u64;
        ensure!(
            self.sender
                .configured_min_tip_lamports()
                .is_none_or(|minimum| lamports >= minimum),
            "SDK original tip is below the configured route minimum"
        );
        if self.provider != SwqosType::Default && self.check_min_tip {
            ensure!(
                actual_tip >= self.sender.min_tip_sol(),
                "SDK original tip is below the provider minimum"
            );
        }
        if self.provider == SwqosType::Default || tip_sol == 0.0 {
            return Ok(None);
        }
        let tip = self.sender.get_tip_account()?;
        Ok(Some(
            Pubkey::from_str(&tip).context("SDK original tip account is invalid")?,
        ))
    }

    pub(crate) async fn broadcast(
        &self,
        execution: &AdmittedTrade,
        context: DammSellContext,
        artifact: &DammSignedCandidate,
        signed: &SignedSubmission,
        wait_confirmed: bool,
        verification_ms: u64,
    ) -> Result<String> {
        ensure!(
            context.provider == self.provider && self.sender.get_swqos_type() == self.provider,
            "SDK original sender provider changed"
        );
        artifact.check_submission(execution, context, signed)?;
        self.send_packet(signed, wait_confirmed, verification_ms)
            .await
    }

    async fn send_packet(
        &self,
        signed: &SignedSubmission,
        wait_confirmed: bool,
        verification_ms: u64,
    ) -> Result<String> {
        self.send_packet_for(TradeType::Sell, signed, wait_confirmed, verification_ms)
            .await
    }

    pub(crate) async fn broadcast_buy(
        &self,
        execution: &AdmittedTrade,
        context: DammBuyContext,
        artifact: &DammBuySignedCandidate,
        signed: &SignedSubmission,
        wait_confirmed: bool,
        verification_ms: u64,
    ) -> Result<String> {
        ensure!(
            context.provider == self.provider && self.sender.get_swqos_type() == self.provider,
            "SDK original buy sender provider changed"
        );
        artifact.check_submission(execution, context, signed)?;
        self.send_packet_for(TradeType::Buy, signed, wait_confirmed, verification_ms)
            .await
    }

    async fn send_packet_for(
        &self,
        direction: TradeType,
        signed: &SignedSubmission,
        wait_confirmed: bool,
        verification_ms: u64,
    ) -> Result<String> {
        let transaction = decode_packet(signed)?;
        if self.provider == SwqosType::Default {
            let receipt = tokio::time::timeout(
                Duration::from_secs(3),
                self.rpc.send_transaction_with_config(
                    &transaction,
                    RpcSendTransactionConfig {
                        skip_preflight: true,
                        preflight_commitment: Some(CommitmentLevel::Processed),
                        encoding: Some(UiTransactionEncoding::Base64),
                        max_retries: Some(3),
                        min_context_slot: Some(0),
                    },
                ),
            )
            .await
            .context("SDK original RPC submission timed out")??;
            ensure!(
                receipt.to_string() == signed.signature,
                "SDK RPC returned a different signature; reconcile original signature"
            );
            if wait_confirmed {
                self.verify_original(&signed.signature, true, 15_000)
                    .await?;
            }
            return Ok(signed.signature.clone());
        }
        // Provider APIs erase the receipt. Their Ok alone is not signature evidence.
        let sent = tokio::time::timeout(
            Duration::from_secs(3),
            self.sender.send_transaction(direction, &transaction, false),
        )
        .await;
        let send_error = match sent {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(crate::sanitize::redact_urls(&error.to_string())),
            Err(_) => Some("SDK provider submission timed out; preserve original signature".into()),
        };
        let budget = if wait_confirmed {
            15_000
        } else {
            verification_ms.min(15_000)
        };
        if budget == 0 {
            anyhow::bail!(
                "{}",
                send_error.unwrap_or_else(|| {
                    "SDK provider has no verifiable receipt; reconcile original signature".into()
                })
            );
        }
        match self
            .verify_original(&signed.signature, wait_confirmed, budget)
            .await
        {
            Ok(()) => {
                if let Some(error) = send_error {
                    tracing::warn!(error = %error, "original SDK signature observed despite transport error; no resend");
                }
                Ok(signed.signature.clone())
            }
            Err(error) => Err(anyhow::anyhow!(
                "{}{}",
                crate::sanitize::redact_urls(&error.to_string()),
                send_error.map_or_else(String::new, |error| format!("; transport: {error}"))
            )),
        }
    }

    async fn verify_original(
        &self,
        signature: &str,
        confirmed: bool,
        budget_ms: u64,
    ) -> Result<()> {
        tokio::time::timeout(Duration::from_millis(budget_ms), async {
            loop {
                if let Some(observation) = self.observer.lookup_signature(signature).await? {
                    ensure!(observation.signature == signature && observation.slot > 0, "SDK original signature observation identity is invalid");
                    ensure!(observation.error.is_none(), "SDK original signature observed transaction error; reconcile original signature");
                    if !confirmed || observation.commitment >= ChainCommitment::Confirmed { return Ok(()); }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.context("SDK original signature not verified within the observation budget")?
    }
}

fn decode_packet(signed: &SignedSubmission) -> Result<VersionedTransaction> {
    ensure!(
        !signed.bytes.is_empty() && signed.bytes.len() <= 1232,
        "SDK original packet size is invalid"
    );
    let transaction: VersionedTransaction = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(1232)
        .reject_trailing_bytes()
        .deserialize(&signed.bytes)?;
    transaction
        .sanitize()
        .map_err(|_| anyhow::anyhow!("SDK original packet failed sanitization"))?;
    ensure!(
        matches!(&transaction.message, VersionedMessage::Legacy(_))
            && transaction.message.header().num_required_signatures == 1
            && transaction.signatures.len() == 1,
        "SDK original packet version or signer count is invalid"
    );
    let signer = transaction
        .message
        .static_account_keys()
        .first()
        .context("SDK original payer missing")?;
    let signature = transaction.signatures[0];
    ensure!(
        signature != Signature::default()
            && signature.to_string() == signed.signature
            && signature.verify(signer.as_ref(), &transaction.message.serialize())
            && bincode::serialize(&transaction)? == signed.bytes,
        "SDK original signed packet is not canonical or verifiable"
    );
    Ok(transaction)
}

#[cfg(test)]
mod tests;
