use super::*;
use solana_account_decoder::UiAccountData;
use solana_client::rpc_response::RpcKeyedAccount;

impl<T: ReadHttpTransport> SharedReadRuntime<T> {
    /// V1 construction hint only, not a verified ATA, balance or signing permission.
    pub(crate) async fn preparation_account_exists(
        &self,
        url: &str,
        wallet: Address,
        mint: Address,
    ) -> Result<bool> {
        let deadline = Instant::now() + STEP;
        let index = *self
            .candidate_indices(&[url.to_owned()])
            .first()
            .ok_or_else(|| unavailable("preparation RPC endpoint is not registered"))?;
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            let _slot = self.slots[Class::PreparationBalance.index()]
                .clone()
                .try_acquire_owned()
                .map_err(|_| unavailable("preparation RPC quota busy"))?;
            let (_, value) = self
                .preparation_step(
                    index,
                    |id| ReadRequest::TokenAccounts { id, wallet, mint },
                    deadline,
                )
                .await?;
            let exists = presence(value)?;
            if Instant::now() >= deadline {
                return Err(unavailable("preparation account presence deadline expired"));
            }
            Ok(exists)
        })
        .await
        .map_err(|_| unavailable("preparation account presence read timed out"))?
    }
}

fn presence(value: Value) -> Result<bool> {
    let count = value
        .as_array()
        .ok_or_else(|| invalid("preparation account presence metadata is invalid"))?
        .len();
    if count > 1000 {
        return Err(invalid(
            "preparation account presence scan exceeds account limit",
        ));
    }
    // Preserve the original SDK deserialization and program-label predicate.
    // Other account identity/state checks belong to the later protocol evidence.
    let accounts: Vec<RpcKeyedAccount> = serde_json::from_value(value)
        .map_err(|_| invalid("preparation account presence metadata is invalid"))?;
    Ok(accounts
        .into_iter()
        .any(|account| match account.account.data {
            UiAccountData::Json(parsed) => {
                parsed.program == "spl-token" || parsed.program == "spl-token-2022"
            }
            _ => false,
        }))
}
