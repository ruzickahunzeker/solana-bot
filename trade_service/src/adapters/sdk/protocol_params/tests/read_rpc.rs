//! Address-scripted, socket-free transport for exercising the real SDK loaders.
use base64::Engine;
use serde_json::{json, Value};
use solana_client::{
    client_error::{ClientError as Error, ClientErrorKind as ErrorKind, Result},
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_sdk::{account::Account, pubkey::Pubkey};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub(super) struct ReadRpc {
    accounts: HashMap<String, Option<Account>>,
    epoch: Option<u64>,
    program_scans: HashMap<Pubkey, Vec<(Pubkey, Account)>>,
    calls: Arc<Mutex<Vec<(RpcRequest, Value)>>>,
    violations: Arc<Mutex<Vec<String>>>,
}

impl ReadRpc {
    pub(super) fn account(&mut self, address: Pubkey, account: Account) {
        self.accounts.insert(address.to_string(), Some(account));
    }

    pub(super) fn absent(&mut self, address: Pubkey) {
        self.accounts.insert(address.to_string(), None);
    }

    pub(super) fn epoch(&mut self, epoch: u64) {
        self.epoch = Some(epoch);
    }
    pub(super) fn program_scan(&mut self, program: Pubkey, addresses: &[Pubkey]) {
        self.program_scans.insert(
            program,
            addresses
                .iter()
                .map(|address| {
                    (
                        *address,
                        self.accounts
                            .get(&address.to_string())
                            .unwrap()
                            .clone()
                            .unwrap(),
                    )
                })
                .collect(),
        );
    }

    pub(super) fn merge(&mut self, other: Self) {
        self.accounts.extend(other.accounts);
        self.program_scans.extend(other.program_scans);
    }

    pub(super) fn client(&self) -> RpcClient {
        RpcClient::new_sender(self.clone(), RpcClientConfig::default())
    }

    pub(super) fn calls(&self) -> Vec<(RpcRequest, Value)> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn assert_read_only(&self) {
        assert!(self.violations.lock().unwrap().is_empty());
        assert!(self.calls().iter().all(|(method, _)| matches!(
            method,
            RpcRequest::GetAccountInfo | RpcRequest::GetMultipleAccounts | RpcRequest::GetEpochInfo
        ) || (*method
            == RpcRequest::GetProgramAccounts
            && !self.program_scans.is_empty())));
    }

    fn reject(&self, message: String) -> Error {
        self.violations.lock().unwrap().push(message.clone());
        ErrorKind::Custom(message).into()
    }

    fn lookup(&self, address: &Value) -> Result<Value> {
        let address = address
            .as_str()
            .ok_or_else(|| self.reject("fixture account address is not a string".into()))?;
        let account = self
            .accounts
            .get(address)
            .ok_or_else(|| self.reject(format!("unscripted fixture account: {address}")))?;
        Ok(account.as_ref().map_or(Value::Null, |account| {
            json!({
                "lamports": account.lamports,
                "owner": account.owner.to_string(),
                "data": [base64::engine::general_purpose::STANDARD.encode(&account.data), "base64"],
                "executable": account.executable,
                "rentEpoch": account.rent_epoch,
                "space": account.data.len()
            })
        }))
    }

    pub(super) fn respond(&self, request: RpcRequest, params: Value) -> Result<Value> {
        self.calls.lock().unwrap().push((request, params.clone()));
        let value = match request {
            RpcRequest::GetAccountInfo => self.lookup(&params[0])?,
            RpcRequest::GetMultipleAccounts => Value::Array(
                params[0]
                    .as_array()
                    .ok_or_else(|| self.reject("fixture account list is not an array".into()))?
                    .iter()
                    .map(|address| self.lookup(address))
                    .collect::<Result<Vec<_>>>()?,
            ),
            RpcRequest::GetEpochInfo => {
                let epoch = self
                    .epoch
                    .ok_or_else(|| self.reject("unscripted fixture epoch".into()))?;
                return Ok(json!({"epoch":epoch,"slotIndex":1,"slotsInEpoch":432000,
                    "absoluteSlot":epoch * 432000 + 1,"blockHeight":42,"transactionCount":100}));
            }
            RpcRequest::GetProgramAccounts if !self.program_scans.is_empty() => {
                let program = params[0].as_str().unwrap().parse::<Pubkey>().unwrap();
                let accounts = self
                    .program_scans
                    .get(&program)
                    .ok_or_else(|| self.reject("wrong scripted program".into()))?;
                let config: solana_client::rpc_config::RpcProgramAccountsConfig =
                    serde_json::from_value(params[1].clone()).unwrap();
                let filters = config.filters.unwrap();
                let matches = accounts.iter().filter(|(_, account)| filters.iter().all(|filter| match filter {
                    solana_client::rpc_filter::RpcFilterType::DataSize(size) => *size == account.data.len() as u64,
                    solana_client::rpc_filter::RpcFilterType::Memcmp(filter) => filter.bytes_match(&account.data),
                    _ => false,
                })).map(|(address, account)| json!({"pubkey":address.to_string(),"account":{
                    "lamports":account.lamports,"owner":account.owner.to_string(),"data":[base64::engine::general_purpose::STANDARD.encode(&account.data),"base64"],"executable":account.executable,"rentEpoch":account.rent_epoch,"space":account.data.len()
                }})).collect::<Vec<_>>();
                return Ok(json!(matches));
            }
            _ => return Err(self.reject(format!("forbidden fixture RPC method: {request}"))),
        };
        Ok(json!({"context":{"slot":42},"value":value}))
    }
}

// Spell out async_trait's boxed future ABI to avoid adding a production dependency for tests.
impl RpcSender for ReadRpc {
    fn send<'life0, 'async_trait>(
        &'life0 self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { self.respond(request, params) })
    }

    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats {
            request_count: self.calls.lock().unwrap().len(),
            ..Default::default()
        }
    }

    fn url(&self) -> String {
        "memory://sdk-parameter-fixtures".into()
    }
}

pub(super) fn account(owner: Pubkey, data: Vec<u8>) -> Account {
    Account {
        lamports: 2_039_280,
        data,
        owner,
        executable: false,
        rent_epoch: 0,
    }
}

#[tokio::test]
async fn fixture_transport_repeats_address_reads_and_preserves_explicit_missing_accounts() {
    let mut fixture = ReadRpc::default();
    let address = Pubkey::new_from_array([1; 32]);
    let missing = Pubkey::new_from_array([2; 32]);
    let expected = account(Pubkey::new_from_array([3; 32]), vec![4, 5]);
    fixture.account(address, expected.clone());
    fixture.absent(missing);
    fixture.epoch(17);
    let rpc = fixture.client();
    for _ in 0..2 {
        assert_eq!(rpc.get_account(&address).await.unwrap(), expected);
    }
    assert_eq!(
        rpc.get_multiple_accounts(&[address, missing])
            .await
            .unwrap(),
        vec![Some(expected), None]
    );
    assert_eq!(rpc.get_epoch_info().await.unwrap().epoch, 17);
    fixture.assert_read_only();
    assert_eq!(fixture.calls().len(), 4);
    assert_eq!(fixture.get_transport_stats().request_count, 4);
}

#[tokio::test]
async fn fixture_transport_denies_unknown_accounts_state_and_all_money_methods() {
    let fixture = ReadRpc::default();
    for (method, params) in [
        (RpcRequest::SendTransaction, json!(["not-a-transaction"])),
        (
            RpcRequest::SimulateTransaction,
            json!(["not-a-transaction"]),
        ),
        (RpcRequest::GetBalance, json!(["unscripted"])),
        (RpcRequest::GetAccountInfo, json!(["unscripted"])),
        (RpcRequest::GetMultipleAccounts, json!([["unscripted"]])),
        (RpcRequest::GetEpochInfo, json!([])),
    ] {
        assert!(fixture.send(method, params).await.is_err());
    }
    assert_eq!(fixture.calls().len(), 6);
    assert_eq!(fixture.violations.lock().unwrap().len(), 6);
}
