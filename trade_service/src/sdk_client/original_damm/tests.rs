use super::*;
use crate::{
    adapters::{
        persistence::ordinary_claims::SqliteOrdinaryExecutionClaims,
        sdk::damm_probe::test_support::{
            buy_runtime_fixture, change_token, clock, key, Fixture as ProbeFixture,
        },
        test_support::{
            read_rpc_server::{LocalServer, Reply},
            IsolatedTestProcess,
        },
    },
    application::{
        ordinary_claims::test_support::{admit_request_for_pubkey, Fixture},
        ports::ordinary_claims::{ExecutionClaim, OrdinaryExecutionClaimer},
    },
    config::{TradePersistenceConfig, TradePersistenceMode},
    domain::{blockhash::Blockhash, execution_policy::BroadcastPolicy, trade::TradeStatus},
    persistence::TradePersistence,
    sdk_client::SdkLane,
};
use base64::Engine;
use serde_json::{json, Value};
use sol_trade_sdk::{
    client::TradingInfrastructure,
    common::InfrastructureConfig,
    constants::{USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    instruction::utils::meteora_damm_v2::accounts,
    swqos::{SwqosClientTrait, SwqosType, TradeType},
    TradingClient,
};
use solana_client::{
    client_error::ClientError,
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_request::RpcRequest,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    hash::Hash,
    message::VersionedMessage,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    transaction::VersionedTransaction,
};
use sqlx::{Row, SqlitePool};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Good,
    BadRaw,
    ZeroOutput,
    BadPreview,
    WrongReceipt,
    ReceiptWriteFailure,
    CancelProbe,
    CancelSend,
}
#[derive(Default)]
struct Trace {
    calls: Mutex<Vec<(RpcRequest, Value)>>,
    simulations: Mutex<Vec<VersionedTransaction>>,
    sent: Mutex<Vec<Vec<u8>>>,
    binding: Mutex<Option<(SqlitePool, String)>>,
    entered: Notify,
    dropped: AtomicBool,
}
struct Dropped(Arc<Trace>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}
struct MetadataSender;
impl SwqosClientTrait for MetadataSender {
    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Default
    }
    fn get_tip_account(&self) -> Result<String> {
        panic!("Default must not request a provider tip")
    }
    fn send_transaction<'a, 'b, 'f>(
        &'a self,
        _: TradeType,
        _: &'b VersionedTransaction,
        _: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async {
            panic!("original SDK path must not enter high-level/provider Default send")
        })
    }
    fn send_transactions<'a, 'b, 'f>(
        &'a self,
        _: TradeType,
        _: &'b Vec<VersionedTransaction>,
        _: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'f>>
    where
        'a: 'f,
        'b: 'f,
        Self: 'f,
    {
        Box::pin(async { panic!("batch/SDK worker sending is forbidden") })
    }
}

#[derive(Clone)]
struct MemoryRpc {
    url: String,
    wallet: Pubkey,
    values: Vec<Value>,
    buy_result: Option<Value>,
    mode: Mode,
    trace: Arc<Trace>,
}
impl MemoryRpc {
    fn simulation(&self, tx: &VersionedTransaction) -> Value {
        let VersionedMessage::Legacy(message) = &tx.message else {
            panic!("fixed Legacy simulation required")
        };
        let input =
            sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &self.wallet,
                &key(4),
                &spl_token_interface::ID,
            );
        let output =
            sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &self.wallet,
                &USDC_TOKEN_ACCOUNT,
                &spl_token_interface::ID,
            );
        let moved = if self.mode == Mode::ZeroOutput {
            0
        } else {
            120
        };
        let mut post = self.values.clone();
        change_token(&mut post[6], |a| a.amount = 900);
        change_token(&mut post[7], |a| a.amount = 2000 + moved);
        change_token(&mut post[3], |a| a.amount += 100);
        change_token(&mut post[4], |a| a.amount -= moved);
        post[5] = clock(124, 17);
        let transfer = |from: Pubkey, mint: Pubkey, to: Pubkey, authority: Pubkey, amount: u64| {
            let ix = spl_token_interface::instruction::transfer_checked(
                &spl_token_interface::ID,
                &from,
                &mint,
                &to,
                &authority,
                &[],
                amount,
                6,
            )
            .unwrap();
            json!({"programId":ix.program_id.to_string(),"accounts":ix.accounts.iter().map(|a|a.pubkey.to_string()).collect::<Vec<_>>(),"data":bs58::encode(ix.data).into_string(),"stackHeight":2})
        };
        let index = message
            .instructions
            .iter()
            .position(|ix| {
                message.account_keys[usize::from(ix.program_id_index)] == accounts::METEORA_DAMM_V2
            })
            .unwrap();
        json!({"err":null,"logs":[],"accounts":post,"unitsConsumed":123,
        "innerInstructions":[{"index":index,"instructions":[
            transfer(input,key(4),key(2),self.wallet,100),
            transfer(key(3),USDC_TOKEN_ACCOUNT,output,accounts::AUTHORITY,moved)
        ]}]})
    }
}
impl RpcSender for MemoryRpc {
    fn send<'a, 'f>(
        &'a self,
        request: RpcRequest,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, ClientError>> + Send + 'f>>
    where
        'a: 'f,
        Self: 'f,
    {
        Box::pin(async move {
            self.trace
                .calls
                .lock()
                .unwrap()
                .push((request, params.clone()));
            match request {
                RpcRequest::GetAccountInfo => {
                    assert_eq!(params[0], key(1).to_string());
                    Ok(json!({"context":{"slot":123},"value":self.values[0]}))
                }
                RpcRequest::GetTokenAccountsByOwner if self.buy_result.is_some() => {
                    assert_eq!(params[0], self.wallet.to_string());
                    assert_eq!(params[1]["mint"], key(4).to_string());
                    let output = sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                        &self.wallet, &key(4), &spl_token_interface::ID,
                    );
                    Ok(
                        json!({"context":{"slot":42},"value":[{"pubkey":output.to_string(),"account":{
                            "owner":spl_token_interface::ID.to_string(),"lamports":2039280,"rentEpoch":0,"executable":false,"space":165,
                            "data":{"program":"spl-token","space":165,"parsed":{"type":"account","info":{"owner":self.wallet.to_string(),"mint":key(4).to_string(),"state":"initialized","tokenAmount":{"amount":"2000","decimals":6,"uiAmount":null,"uiAmountString":"0.002"}}}}
                        }}]}),
                    )
                }
                RpcRequest::GetMultipleAccounts => {
                    let addresses = params[0].as_array().unwrap();
                    if self.buy_result.is_some() {
                        let quote: Pubkey = self.buy_result.as_ref().unwrap()["preTokenBalances"]
                            [0]["mint"]
                            .as_str()
                            .unwrap()
                            .parse()
                            .unwrap();
                        let values = match addresses.len() {
                            2 => {
                                assert_eq!(
                                    params[0],
                                    json!([key(4).to_string(), quote.to_string()])
                                );
                                self.values[1..3].to_vec()
                            }
                            9 => {
                                let ata = |mint| {
                                    sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(&self.wallet,&mint,&spl_token_interface::ID).to_string()
                                };
                                assert_eq!(
                                    params[0],
                                    json!([
                                        key(1).to_string(),
                                        key(4).to_string(),
                                        quote.to_string(),
                                        key(2).to_string(),
                                        key(3).to_string(),
                                        solana_sdk::sysvar::clock::id().to_string(),
                                        ata(quote),
                                        ata(key(4)),
                                        self.wallet.to_string()
                                    ])
                                );
                                assert!(params[1]["minContextSlot"].is_null());
                                self.values.clone()
                            }
                            _ => panic!("unexpected original buy account query"),
                        };
                        return Ok(json!({"context":{"slot":42},"value":values}));
                    }
                    let values = match addresses.len() {
                        2 => {
                            assert_eq!(
                                *addresses,
                                vec![
                                    json!(key(4).to_string()),
                                    json!(USDC_TOKEN_ACCOUNT.to_string())
                                ]
                            );
                            self.values[1..3].to_vec()
                        }
                        8 => {
                            let input=sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(&self.wallet,&key(4),&spl_token_interface::ID);
                            let output=sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(&self.wallet,&USDC_TOKEN_ACCOUNT,&spl_token_interface::ID);
                            assert_eq!(
                                params[0],
                                json!([
                                    key(1).to_string(),
                                    key(4).to_string(),
                                    USDC_TOKEN_ACCOUNT.to_string(),
                                    key(2).to_string(),
                                    key(3).to_string(),
                                    solana_sdk::sysvar::clock::id().to_string(),
                                    input.to_string(),
                                    output.to_string()
                                ])
                            );
                            assert_eq!(params[1]["minContextSlot"], 123);
                            self.values.clone()
                        }
                        _ => panic!("unexpected SDK account query"),
                    };
                    Ok(json!({"context":{"slot":123},"value":values}))
                }
                RpcRequest::SimulateTransaction => {
                    let tx: VersionedTransaction = bincode::deserialize(
                        &base64::engine::general_purpose::STANDARD
                            .decode(params[0].as_str().unwrap())
                            .unwrap(),
                    )
                    .unwrap();
                    assert_eq!(tx.signatures, vec![Signature::default()]);
                    assert_eq!(
                        *tx.message.recent_blockhash(),
                        Hash::new_from_array([9; 32])
                    );
                    assert_eq!(tx.message.static_account_keys()[0], self.wallet);
                    assert_eq!(params[1]["sigVerify"], false);
                    assert_eq!(params[1]["replaceRecentBlockhash"], false);
                    self.trace.simulations.lock().unwrap().push(tx.clone());
                    if !params[1]["accounts"].is_null() {
                        assert_eq!(
                            params[1]["accounts"]["addresses"].as_array().unwrap().len(),
                            if self.buy_result.is_some() { 9 } else { 8 }
                        );
                        assert_eq!(
                            params[1]["minContextSlot"],
                            if self.buy_result.is_some() { 42 } else { 123 }
                        );
                        if self.mode == Mode::CancelProbe {
                            let _drop = Dropped(self.trace.clone());
                            self.trace.entered.notify_one();
                            std::future::pending::<()>().await;
                        }
                        if let Some(result) = &self.buy_result {
                            let mut result = result.clone();
                            if self.mode == Mode::ZeroOutput {
                                result["innerInstructions"][0]["instructions"][1]["parsed"]
                                    ["info"]["tokenAmount"]["amount"] = json!("0");
                            }
                            Ok(json!({"context":{"slot":43},"value":result}))
                        } else {
                            Ok(json!({"context":{"slot":124},"value":self.simulation(&tx)}))
                        }
                    } else {
                        let slot = if self.buy_result.is_some() { 43 } else { 124 };
                        assert_eq!(params[1]["minContextSlot"], slot);
                        let err = if self.mode == Mode::BadPreview {
                            json!("AccountNotFound")
                        } else {
                            Value::Null
                        };
                        Ok(
                            json!({"context":{"slot":slot},"value":{"err":err,"logs":[],"accounts":null,"unitsConsumed":123}}),
                        )
                    }
                }
                RpcRequest::SendTransaction => {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(params[0].as_str().unwrap())
                        .unwrap();
                    let tx: VersionedTransaction = bincode::deserialize(&bytes).unwrap();
                    assert_eq!(tx.message.static_account_keys()[0], self.wallet);
                    assert!(tx.signatures[0].verify(self.wallet.as_ref(), &tx.message.serialize()));
                    let (pool, id) = self.trace.binding.lock().unwrap().as_ref().unwrap().clone();
                    let row=sqlx::query("SELECT submission_stage,signed_signature,signed_transaction,unsigned_message FROM ordinary_execution_claims WHERE request_id=?").bind(id).fetch_one(&pool).await.unwrap();
                    assert_eq!(row.get::<&str, _>("submission_stage"), "broadcasting");
                    assert_eq!(row.get::<Vec<u8>, _>("signed_transaction"), bytes);
                    assert_eq!(
                        row.get::<Vec<u8>, _>("unsigned_message"),
                        tx.message.serialize()
                    );
                    assert_eq!(
                        row.get::<&str, _>("signed_signature"),
                        tx.signatures[0].to_string()
                    );
                    assert_eq!(
                        params[1],
                        json!({"encoding":"base64","skipPreflight":true,"preflightCommitment":"processed","maxRetries":3,"minContextSlot":0})
                    );
                    self.trace.sent.lock().unwrap().push(bytes);
                    if self.mode == Mode::CancelSend {
                        let _drop = Dropped(self.trace.clone());
                        self.trace.entered.notify_one();
                        std::future::pending::<()>().await;
                    }
                    Ok(json!(if self.mode == Mode::WrongReceipt {
                        Signature::new_unique().to_string()
                    } else {
                        tx.signatures[0].to_string()
                    }))
                }
                _ => panic!("forbidden RPC in actual SDK original execution: {request}"),
            }
        })
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        self.url.clone()
    }
}

struct Case {
    wallet: SdkTradeClient,
    f: Fixture,
    owner: Option<ExecutionOwner>,
    journal: SqliteOrdinaryExecutionClaims,
    pool: SqlitePool,
    runtime: Arc<SharedReadRuntime>,
    server: LocalServer,
    trace: Arc<Trace>,
    original_request: (String, String),
}
impl Case {
    async fn new(mode: Mode) -> Self {
        Self::with_payment(mode, None).await
    }
    async fn new_buy(mode: Mode, native: bool) -> Self {
        Self::with_payment(mode, Some(native)).await
    }
    async fn with_payment(mode: Mode, native: Option<bool>) -> Self {
        let keypair = Arc::new(Keypair::new());
        let wallet = keypair.pubkey();
        let mut source = ProbeFixture::new(false, false, USDC_TOKEN_ACCOUNT, true);
        change_token(&mut source.rpc.values[6], |a| a.owner = wallet);
        change_token(&mut source.rpc.values[7], |a| a.owner = wallet);
        source.rpc.values[5] = clock(123, 17);
        let (mut values, buy_result) = if let Some(native) = native {
            let (values, result) = buy_runtime_fixture(wallet, native);
            (values, Some(result))
        } else {
            (source.rpc.values.clone(), None)
        };
        if native.is_some() && mode == Mode::BadRaw {
            values[1]["owner"] = json!(key(99).to_string());
        }
        let input =
            sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &wallet,
                &key(4),
                &spl_token_interface::ID,
            );
        let parsed = json!([{"pubkey":input.to_string(),"account":{"owner":spl_token_interface::ID.to_string(),"lamports":10000000,"rentEpoch":0,"executable":false,"space":165,
            "data":{"program":"spl-token","space":165,"parsed":{"type":"account","info":{"owner":wallet.to_string(),"mint":key(4).to_string(),"state":"initialized","tokenAmount":{"amount":"1000","decimals":6,"uiAmount":null,"uiAmountString":"0.001"}}}}}}]);
        let mut raw = vec![
            source.rpc.values[1].clone(),
            source.rpc.values[6].clone(),
            Value::Null,
        ];
        if mode == Mode::BadRaw {
            raw[0]["owner"] = json!(key(99).to_string());
        }
        let server = LocalServer::new(Reply::Preparation {
            scan: Box::new(Reply::Success),
            raw: Box::new(Reply::Success),
            scan_value: parsed,
            raw_value: json!(raw),
        })
        .await;
        let trace = Arc::new(Trace::default());
        let rpc = Arc::new(RpcClient::new_sender(
            MemoryRpc {
                url: server.url.clone(),
                wallet,
                values,
                buy_result,
                mode,
                trace: trace.clone(),
            },
            RpcClientConfig::default(),
        ));
        let infrastructure = Arc::new(TradingInfrastructure {
            rpc,
            swqos_clients: Arc::new(vec![Arc::new(MetadataSender)]),
            config: InfrastructureConfig::new(
                server.url.clone(),
                vec![],
                CommitmentConfig::processed(),
            ),
            max_sender_concurrency: 1,
            effective_core_ids: Arc::new(vec![]),
        });
        let client = TradingClient::from_infrastructure(keypair, infrastructure, false);
        let wallet_client = SdkTradeClient {
            lanes: vec![SdkLane {
                swqos_type: SwqosType::Default,
                is_helius: false,
                client,
            }],
            sol_hop_cache: tokio::sync::Mutex::new(HashMap::new()),
            swqos_lanes: vec!["Default".into()],
            mev_protection: false,
            require_mev_submit: false,
            rpc_count: 1,
            broadcast_policy: BroadcastPolicy::Enabled,
        };
        let mut state = crate::http_contract_tests::state().await;
        state.persistence = Arc::new(
            TradePersistence::new(&TradePersistenceConfig {
                mode: TradePersistenceMode::Durable,
                database_url: "sqlite::memory:".into(),
                busy_timeout_ms: 2000,
            })
            .await
            .unwrap(),
        );
        let mut request = json!({"wallet_id":"fixture-wallet","side":"sell","pool":"meteora-damm-v2","mint":key(4).to_string(),"quote_mint":USDC_TOKEN_ACCOUNT.to_string(),"pool_id":key(1).to_string(),"sell_percent":10,"slippage_bps":1500,"tip_sol":0.0,"use_mev":false,"simulate":false,"execution_method":"sdk","idempotency_key":" sdk-original-runtime "});
        if let Some(native) = native {
            request["side"] = json!("buy");
            request.as_object_mut().unwrap().remove("sell_percent");
            request["quote_mint"] = json!(if native {
                WSOL_TOKEN_ACCOUNT
            } else {
                USDC_TOKEN_ACCOUNT
            }
            .to_string());
            request[if native {
                "amount_lamports"
            } else {
                "amount_quote_units"
            }] = json!(100);
        }
        let request = serde_json::from_value(request).unwrap();
        let f = admit_request_for_pubkey(state, request, wallet.to_string()).await;
        let ExecutionClaim::Acquired(owner) = f.claims().claim(&f.execution).await.unwrap() else {
            panic!("original temporary owner required")
        };
        let parent = f
            .state
            .trades
            .update(&f.record.request_id, TradeStatus::BuildingTx, None, None)
            .unwrap();
        f.state.persistence.update(&parent).await.unwrap();
        let pool = f.state.persistence.pool_for_tests();
        let original_request =
            sqlx::query_as("SELECT request_hash,request_json FROM trades WHERE request_id=?")
                .bind(&f.record.request_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        *trace.binding.lock().unwrap() = Some((pool.clone(), f.record.request_id.clone()));
        if mode == Mode::ReceiptWriteFailure {
            sqlx::raw_sql("CREATE TRIGGER deny_actual_sdk_receipt BEFORE UPDATE OF submission_stage ON ordinary_execution_claims WHEN NEW.submission_stage='submitted' BEGIN SELECT RAISE(ABORT,'synthetic actual SDK receipt failure'); END;").execute(&pool).await.unwrap();
        }
        let runtime = Arc::new(SharedReadRuntime::new(&[server.url.clone()]).unwrap());
        assert!(trace.calls.lock().unwrap().is_empty());
        Self {
            wallet: wallet_client,
            f,
            owner: Some(owner),
            journal: SqliteOrdinaryExecutionClaims::new(pool.clone()),
            pool,
            runtime,
            server,
            trace,
            original_request,
        }
    }
    async fn run(&mut self) -> JournaledTradeOutcome {
        self.wallet
            .execute_original_damm(
                &self.journal,
                &self.f.execution,
                self.owner.take().unwrap(),
                SdkExecutionContext {
                    blockhash: Blockhash::from_bytes([9; 32]),
                    wait_tx_confirmed: false,
                    mev_submit_verify_ms: 0,
                },
                self.runtime.clone(),
            )
            .await
    }
    async fn unchanged_request(&self) {
        let request: (String, String) =
            sqlx::query_as("SELECT request_hash,request_json FROM trades WHERE request_id=?")
                .bind(&self.f.record.request_id)
                .fetch_one(&self.pool)
                .await
                .unwrap();
        assert_eq!(request, self.original_request);
        assert_eq!(
            self.f.claims().claim(&self.f.execution).await.unwrap(),
            ExecutionClaim::AlreadyHeld
        );
    }
    async fn finish(self) {
        self.server.finish().await;
        self.pool.close().await;
    }
}

mod buy;
mod cases;

const FLAG: &str = "SOL_BOT_SDK_ORIGINAL_RUNTIME_CASE";
const CHILD: &str = "sdk_client::original_damm::tests::sdk_original_runtime_child";
const DONE: &str = "SDK_ORIGINAL_RUNTIME_DONE";
fn child(case: &str) {
    let process = IsolatedTestProcess::spawn(CHILD, FLAG, case);
    process.await_line(DONE);
    process.finish();
}
#[test]
fn sdk_original_runtime_real_preparation_signing_journal_and_rpc_submit() {
    child("good");
}
#[test]
fn sdk_original_runtime_invalid_raw_zero_output_or_preview_never_submit() {
    child("pre-sign");
}
#[test]
fn sdk_original_runtime_wrong_receipt_or_lost_write_preserves_original_packet() {
    child("post-sign");
}
#[test]
fn sdk_original_runtime_cancellation_during_probe_or_send_never_retries() {
    child("cancel");
}
#[tokio::test]
async fn sdk_original_runtime_child() {
    let Ok(case) = std::env::var(FLAG) else {
        return;
    };
    match case.as_str() {
        "good" => cases::good().await,
        "pre-sign" => cases::pre_sign().await,
        "post-sign" => cases::post_sign().await,
        "cancel" => cases::cancel().await,
        "buy-good" => buy::good().await,
        "buy-pre-sign" => buy::pre_sign().await,
        "buy-post-sign" => buy::post_sign().await,
        "buy-cancel" => buy::cancel().await,
        _ => panic!("unknown synthetic SDK runtime case"),
    }
    println!("{DONE}");
}
