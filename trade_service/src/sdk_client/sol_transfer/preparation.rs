use super::*;
use crate::adapters::rpc::{
    read_http::ReadHttpTransport, read_runtime::SharedReadRuntime,
    sol_transfer_reads::SolTransferReadSession,
};
use anyhow::bail;

pub(super) async fn prepare<T: ReadHttpTransport>(
    source: Pubkey,
    destination: Pubkey,
    fixed_lamports: Option<u64>,
    percent: Option<u64>,
    rpcs: impl Iterator<Item = Arc<RpcClient>> + Send,
    runtime: &SharedReadRuntime<T>,
    timeout: Duration,
) -> Result<PreparedSolTransfer> {
    if source == destination {
        bail!("source and destination wallets must be different");
    }
    let mut prepared = None;
    let mut last_error = None;
    for rpc in rpcs {
        let result = tokio::time::timeout(timeout, async {
            let reads = SolTransferReadSession::new(
                runtime,
                &rpc.url(),
                rpc.commitment(),
                source,
                destination,
            )?;
            let (balance, blockhash, fee, rent) = reads.snapshot().await?;
            Ok::<_, anyhow::Error>((rpc, balance, blockhash, fee, rent))
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
