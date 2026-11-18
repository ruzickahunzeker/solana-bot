use super::SdkTradeClient;
use crate::{
    application::sol_transfer_policy::calculate_sol_transfer_amount,
    domain::{sol_transfer::SolTransferSnapshot, submission::SignedSubmission, trade::Address},
};
#[cfg(test)]
use anyhow::bail;
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::swqos::SwqosType;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    hash::Hash,
    message::Message,
    pubkey::Pubkey,
    signature::{Signature, Signer},
    transaction::Transaction,
};
use solana_system_interface::instruction as system_instruction;
use std::{sync::Arc, time::Duration};

mod confirmation;
mod preparation;

const PREPARE_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(12);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct PreparedSolTransfer {
    rpc: Arc<RpcClient>,
    message: Message,
    blockhash: Hash,
    snapshot: SolTransferSnapshot,
    rent_exempt_minimum: u64,
    fixed_lamports: Option<u64>,
    percent: Option<u64>,
}

impl PreparedSolTransfer {
    pub(crate) fn snapshot(&self) -> &SolTransferSnapshot {
        &self.snapshot
    }
    fn ensure_source(&self, wallet: &Pubkey) -> Result<()> {
        ensure!(
            Address::from_bytes(wallet.to_bytes()) == self.snapshot.source,
            "prepared SOL transfer belongs to a different wallet"
        );
        let (amount, reserve) = calculate_sol_transfer_amount(
            self.snapshot.balance_before_lamports,
            self.snapshot.fee_lamports,
            self.rent_exempt_minimum,
            self.fixed_lamports,
            self.percent,
        )?;
        ensure!(
            amount == self.snapshot.amount_lamports && reserve == self.snapshot.reserve_lamports,
            "prepared SOL transfer amount does not match audited balance and fees"
        );
        let expected = Message::new_with_blockhash(
            &[system_instruction::transfer(
                wallet,
                &Pubkey::new_from_array(self.snapshot.destination.to_bytes()),
                amount,
            )],
            Some(wallet),
            &self.blockhash,
        );
        ensure!(
            self.message == expected,
            "prepared SOL transfer message does not match audited transfer"
        );
        Ok(())
    }
}

pub(crate) struct SignedSolTransfer {
    prepared: PreparedSolTransfer,
    transaction: Transaction,
    submission: SignedSubmission,
}

impl SignedSolTransfer {
    fn new(prepared: PreparedSolTransfer, transaction: Transaction) -> Result<Self> {
        prepared.ensure_source(&Pubkey::new_from_array(prepared.snapshot.source.to_bytes()))?;
        ensure!(
            transaction.message == prepared.message
                && transaction.signatures.len() == 1
                && transaction.signatures[0] != Signature::default(),
            "SOL transfer requires the audited message and one signature"
        );
        let submission = SignedSubmission {
            signature: transaction.signatures[0].to_string(),
            bytes: bincode::serialize(&transaction)?,
        };
        Ok(Self {
            prepared,
            transaction,
            submission,
        })
    }

    pub(crate) fn submission(&self) -> &SignedSubmission {
        &self.submission
    }

    pub(crate) fn snapshot(&self) -> &SolTransferSnapshot {
        &self.prepared.snapshot
    }

    fn ensure_source(&self, wallet: &Pubkey) -> Result<()> {
        self.prepared.ensure_source(wallet)?;
        let submission = self.submission();
        ensure!(
            self.transaction.message == self.prepared.message
                && self.transaction.signatures.len() == 1
                && self.transaction.signatures[0].to_string() == submission.signature
                && bincode::serialize(&self.transaction)? == submission.bytes,
            "signed SOL transfer does not match its immutable submission"
        );
        Ok(())
    }
}

impl SdkTradeClient {
    pub(crate) async fn prepare_sol_transfer(
        &self,
        read_rpc: &crate::adapters::rpc::read_runtime::SharedReadRuntime,
        destination: &Pubkey,
        fixed_lamports: Option<u64>,
        percent: Option<u64>,
    ) -> Result<PreparedSolTransfer> {
        self.ensure_broadcast_allowed()?;
        let source = self
            .lanes
            .first()
            .context("wallet has no execution lane")?
            .client
            .payer
            .pubkey();
        let rpcs = self
            .lanes
            .iter()
            .filter(|lane| lane.swqos_type == SwqosType::Default)
            .map(|lane| lane.client.get_rpc().clone());
        preparation::prepare(
            source,
            *destination,
            fixed_lamports,
            percent,
            rpcs,
            read_rpc,
            PREPARE_TIMEOUT,
        )
        .await
    }

    pub(crate) fn sign_sol_transfer(
        &self,
        prepared: PreparedSolTransfer,
    ) -> Result<SignedSolTransfer> {
        self.ensure_broadcast_allowed()?;
        let payer = &self
            .lanes
            .first()
            .context("wallet has no execution lane")?
            .client
            .payer;
        prepared.ensure_source(&payer.pubkey())?;
        let transaction = Transaction::new(
            &[payer.as_ref()],
            prepared.message.clone(),
            prepared.blockhash,
        );
        SignedSolTransfer::new(prepared, transaction)
    }

    pub(crate) async fn send_signed_sol_transfer(
        &self,
        signed: &SignedSolTransfer,
    ) -> Result<Signature> {
        self.ensure_broadcast_allowed()?;
        let wallet = self
            .lanes
            .first()
            .context("wallet has no execution lane")?
            .client
            .payer
            .pubkey();
        broadcast_signed(signed, &wallet, SEND_TIMEOUT).await
    }

    pub(crate) async fn confirm_sol_transfer(
        &self,
        read_rpc: &crate::adapters::rpc::read_runtime::SharedReadRuntime,
        signed: &SignedSolTransfer,
        signature: &Signature,
    ) -> bool {
        confirmation::confirm(signed, signature, read_rpc, CONFIRM_TIMEOUT).await
    }
}

#[cfg(test)]
async fn prepare_with_rpcs(
    source: Pubkey,
    destination: Pubkey,
    fixed_lamports: Option<u64>,
    percent: Option<u64>,
    rpcs: impl Iterator<Item = Arc<RpcClient>> + Send,
    timeout: Duration,
) -> Result<PreparedSolTransfer> {
    if source == destination {
        bail!("source and destination wallets must be different");
    }
    let mut prepared = None;
    let mut last_error = None;
    for rpc in rpcs {
        let result = tokio::time::timeout(timeout, async {
            let balance = rpc.get_balance(&source).await?;
            let blockhash = rpc.get_latest_blockhash().await?;
            let estimate_instruction = system_instruction::transfer(&source, &destination, 1);
            let mut message = Message::new(&[estimate_instruction], Some(&source));
            message.recent_blockhash = blockhash;
            let fee = rpc.get_fee_for_message(&message).await?;
            let rent = rpc.get_minimum_balance_for_rent_exemption(0).await?;
            Ok::<_, solana_client::client_error::ClientError>((rpc, balance, blockhash, fee, rent))
        })
        .await;
        match result {
            Ok(Ok(value)) => {
                prepared = Some(value);
                break;
            }
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => last_error = Some("RPC transfer preparation timed out".to_string()),
        }
    }
    let (rpc, balance, blockhash, fee, rent) = prepared.with_context(|| {
        format!(
            "unable to prepare SOL transfer: {}",
            last_error.unwrap_or_else(|| "no Default RPC lane configured".into()),
        )
    })?;
    let (amount, reserve) =
        calculate_sol_transfer_amount(balance, fee, rent, fixed_lamports, percent)?;
    let message = Message::new_with_blockhash(
        &[system_instruction::transfer(&source, &destination, amount)],
        Some(&source),
        &blockhash,
    );
    Ok(PreparedSolTransfer {
        rpc,
        message,
        blockhash,
        rent_exempt_minimum: rent,
        fixed_lamports,
        percent,
        snapshot: SolTransferSnapshot {
            source: Address::from_bytes(source.to_bytes()),
            destination: Address::from_bytes(destination.to_bytes()),
            amount_lamports: amount,
            fee_lamports: fee,
            balance_before_lamports: balance,
            reserve_lamports: reserve,
        },
    })
}

async fn broadcast_signed(
    signed: &SignedSolTransfer,
    wallet: &Pubkey,
    timeout: Duration,
) -> Result<Signature> {
    signed.ensure_source(wallet)?;
    let snapshot = signed.snapshot();
    let balance = snapshot.balance_before_lamports;
    let amount = snapshot.amount_lamports;
    let fee = snapshot.fee_lamports;
    let reserve = snapshot.reserve_lamports;
    // The F2-C application compares this receipt with submission().signature after durable recording.
    tokio::time::timeout(timeout, signed.prepared.rpc.send_transaction(&signed.transaction))
        .await.context("SOL transfer submission timed out")?.with_context(|| format!(
            "SOL transfer submission failed: balance={balance}, amount={amount}, fee={fee}, reserve={reserve} lamports",
        ))
}

#[cfg(test)]
async fn confirm_signed(
    signed: &SignedSolTransfer,
    signature: &Signature,
    timeout: Duration,
) -> bool {
    matches!(
        tokio::time::timeout(timeout, signed.prepared.rpc.confirm_transaction(signature)).await,
        Ok(Ok(true))
    )
}

#[cfg(test)]
mod tests;
