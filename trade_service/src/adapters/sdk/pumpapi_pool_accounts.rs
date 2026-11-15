use super::{
    pumpapi_accounts::ResolvedPumpApiAccounts, pumpapi_cpmm::check_cpmm_if_applicable,
    pumpapi_pumpswap::check_pumpswap,
};
use crate::{
    application::trade_command::TradeCommand,
    domain::trade::{Address, Pool},
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::transaction::VersionedTransaction;

/// None means this protocol is not checked yet, never a route/signing permission.
pub(crate) async fn check_pool_accounts(
    rpc: &RpcClient,
    wallet: Address,
    command: &TradeCommand,
    transaction: &VersionedTransaction,
    view: &ResolvedPumpApiAccounts,
) -> anyhow::Result<Option<u64>> {
    match command.route.pool {
        Pool::RaydiumCpmm | Pool::StonkFunSwap => {
            check_cpmm_if_applicable(rpc, wallet, command, transaction, view).await
        }
        Pool::PumpAmm => check_pumpswap(rpc, wallet, command, transaction, view)
            .await
            .map(Some),
        _ => Ok(None),
    }
}
