use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{oneshot, Notify},
    task::{JoinHandle, JoinSet},
};
const MAX_BODY: usize = 65_536;
fn signature() -> String {
    bs58::encode([1u8; 64]).into_string()
}
fn request(id: u64, signature: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"getSignatureStatuses","params":[[signature],{"searchTransactionHistory":true}]})
}
fn response(id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"context":{"slot":123},"value":[{"slot":123,"confirmations":null,"status":{"Ok":null},"err":null,"confirmationStatus":"finalized"}]}})
}

pub(crate) trait DammPreviewOracle: Send + Sync {
    fn preview(
        &self,
        params: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send + '_>>;
}

#[derive(Clone, Copy)]
pub(crate) enum DammEvidenceKind {
    Accounts,
    Probe,
}
pub(crate) trait DammEvidenceOracle: Send + Sync {
    fn evidence(
        &self,
        kind: DammEvidenceKind,
        params: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send + '_>>;
}

#[derive(Clone)]
pub(crate) enum Reply {
    SolTransferPreparation {
        values: std::collections::HashMap<String, Value>,
        fail_method: Option<&'static str>,
        failure: Box<Reply>,
        normal: Box<Reply>,
    },
    DammEvidence {
        oracle: Arc<dyn DammEvidenceOracle>,
        other: Box<Reply>,
    },
    DammQuote {
        accounts: Value,
        probe: Value,
        account_mode: Box<Reply>,
        probe_mode: Box<Reply>,
    },
    DammPreview {
        oracle: Arc<dyn DammPreviewOracle>,
        other: Box<Reply>,
    },
    DammSimulation {
        result: Value,
        mode: Box<Reply>,
    },
    QuoteStatus {
        value: Value,
        mode: Box<Reply>,
    },
    WsolBroadcast {
        signature: String,
        mode: Box<Reply>,
    },
    WsolSimulation {
        value: Value,
        mode: Box<Reply>,
    },
    WsolQuote {
        values: std::collections::HashMap<String, Value>,
        fail_method: Option<&'static str>,
        failure: Box<Reply>,
        normal: Box<Reply>,
    },
    AggregatorSimulation {
        baseline: Box<Reply>,
        simulation: Box<Reply>,
        baseline_value: Value,
        simulation_value: Value,
    },
    Parameters {
        accounts: std::collections::HashMap<String, Value>,
        epoch: Option<u64>,
        other: Box<Reply>,
    },
    Success,
    ObservedSignature(String),
    AtLimit,
    Chunked(usize),
    DeclaredOversize,
    RateLimited(Option<&'static str>),
    Redirect(String),
    SlowHeaders,
    DelayedHeaders(u64),
    SlowBody,
    Disconnect,
    WrongId,
    InvalidJson,
    RpcError,
    ServerError,
    WrongAccountCount,
    FailAfter(usize),
    QuoteScan(Box<Reply>),
    QuoteEmpty,
    QuoteUnparsed,
    QuoteWrongMint,
    QuoteMixed,
    QuoteAtaFirstError,
    Preparation {
        scan: Box<Reply>,
        raw: Box<Reply>,
        scan_value: Value,
        raw_value: Value,
    },
}

fn quote_amount(units: &str) -> Value {
    json!({"amount":units,"decimals":6,"uiAmount":null,"uiAmountString":units})
}
fn quote_account(byte: u8, wallet: &str, mint: &str, units: &str) -> Value {
    json!({"pubkey":bs58::encode([byte;32]).into_string(),"account":{"owner":spl_token_interface::ID.to_string(),"lamports":1,"rentEpoch":0,"executable":false,"space":165,
        "data":{"program":"spl-token","space":165,"parsed":{"type":"account","info":{"owner":wallet,"mint":mint,"state":"initialized","tokenAmount":quote_amount(units)}}}}})
}

pub(crate) struct LocalServer {
    pub(crate) url: String,
    pub(crate) requests: Arc<Mutex<Vec<Value>>>,
    pub(crate) connections: Arc<AtomicUsize>,
    pub(crate) received: Arc<Notify>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl LocalServer {
    pub(crate) async fn new(reply: Reply) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        assert!(address.ip().is_loopback());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let received = Arc::new(Notify::new());
        let (stop, mut stopped) = oneshot::channel();
        let task = {
            let requests = requests.clone();
            let connections = connections.clone();
            let received = received.clone();
            tokio::spawn(async move {
                let mut tasks = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = &mut stopped => break,
                        accepted = listener.accept() => {
                            let (stream, peer) = accepted.unwrap();
                            assert!(peer.ip().is_loopback());
                            connections.fetch_add(1, Ordering::SeqCst);
                            let requests = requests.clone();
                            let received = received.clone();
                            let reply = reply.clone();
                            tasks.spawn(async move {
                                serve(stream, reply, requests, received).await;
                            });
                        }
                        Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                            result.unwrap();
                        }
                    }
                }
                tasks.abort_all();
                while let Some(result) = tasks.join_next().await {
                    if let Err(error) = result {
                        assert!(error.is_cancelled(), "{error}");
                    }
                }
            })
        };
        Self {
            url: format!("http://{address}/rpc?api-key=fixture-secret"),
            requests,
            connections,
            received,
            stop: Some(stop),
            task: Some(task),
        }
    }

    pub(crate) fn assert_requests(&self, ids: &[u64]) {
        assert_eq!(self.connections.load(Ordering::SeqCst), ids.len());
        assert_eq!(
            *self.requests.lock().unwrap(),
            ids.iter()
                .map(|id| request(*id, &signature()))
                .collect::<Vec<_>>()
        );
    }

    pub(crate) async fn finish(mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.task.take().unwrap().await.unwrap();
    }
}
impl Drop for LocalServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

// Deliberately raw HTTP fixtures exercise fragmented/chunked and oversized wire responses.
async fn read_request(stream: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0, "fixture client closed before its request");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 8192, "fixture request exceeded bounds");
        let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let header = std::str::from_utf8(&bytes[..end]).unwrap();
        assert!(header.starts_with("POST /rpc?api-key=fixture-secret HTTP/1.1\r\n"));
        let length: usize = header
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .unwrap()
            .1
            .trim()
            .parse()
            .unwrap();
        assert!(length <= 6144);
        if bytes.len() >= end + 4 + length {
            return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
        }
    }
}

async fn serve(
    mut stream: TcpStream,
    reply: Reply,
    requests: Arc<Mutex<Vec<Value>>>,
    received: Arc<Notify>,
) {
    let payload = read_request(&mut stream).await;
    let id = payload["id"].as_u64().unwrap();
    let kind = match payload["method"].as_str() {
        Some("getMultipleAccounts")
            if payload["params"][0]
                .as_array()
                .is_some_and(|a| matches!(a.len(), 8 | 9)) =>
        {
            Some(DammEvidenceKind::Accounts)
        }
        Some("simulateTransaction") if !payload["params"][1]["accounts"].is_null() => {
            Some(DammEvidenceKind::Probe)
        }
        _ => None,
    };
    let (reply, evidence) = match (reply, kind) {
        (Reply::DammEvidence { oracle, .. }, Some(kind)) => {
            let mut byte = [0u8; 1];
            let value = tokio::select! {
                value=oracle.evidence(kind,payload["params"].clone())=>value,
                read=stream.read(&mut byte)=>{
                    assert!(matches!(read,Ok(0)|Err(_)),"unexpected pipelining during blocked quote oracle");
                    return;
                }
            };
            (Reply::Success, Some(value))
        }
        (Reply::DammEvidence { other, .. }, _) => (*other, None),
        (other, _) => (other, None),
    };
    let is_runtime_evidence = evidence.is_some();
    let (reply, preview) = match reply {
        Reply::DammPreview { oracle, .. } if payload["method"] == "simulateTransaction" => {
            assert!(payload["params"][1]["accounts"].is_null());
            let value = oracle.preview(payload["params"].clone()).await;
            (Reply::Success, Some(value))
        }
        Reply::DammPreview { other, .. } => (*other, None),
        other => (other, None),
    };
    let is_runtime_preview = preview.is_some();
    let is_damm_quote = matches!(&reply, Reply::DammQuote { .. });
    let is_wsol_quote = matches!(&reply, Reply::WsolQuote { .. });
    let is_sol_transfer = matches!(&reply, Reply::SolTransferPreparation { .. });
    let is_wsol_simulation = matches!(&reply, Reply::WsolSimulation { .. });
    let is_damm_simulation = matches!(&reply, Reply::DammSimulation { .. });
    let is_wsol_broadcast = matches!(&reply, Reply::WsolBroadcast { .. });
    let is_quote_status = matches!(&reply, Reply::QuoteStatus { .. });
    let (reply, parameter_value) = match reply {
        Reply::SolTransferPreparation {
            values,
            fail_method,
            failure,
            normal,
        } => {
            let method = payload["method"].as_str().unwrap();
            assert!(matches!(
                method,
                "getBalance"
                    | "getLatestBlockhash"
                    | "getFeeForMessage"
                    | "getMinimumBalanceForRentExemption"
            ));
            let value = values
                .get(method)
                .expect("unscripted SOL transfer preparation method")
                .clone();
            (
                if fail_method == Some(method) {
                    *failure
                } else {
                    *normal
                },
                Some(value),
            )
        }
        Reply::DammQuote {
            accounts,
            probe,
            account_mode,
            probe_mode,
        } => match payload["method"].as_str() {
            Some("getMultipleAccounts") => (*account_mode, Some(accounts)),
            Some("simulateTransaction") => (*probe_mode, Some(probe)),
            _ => panic!("DAMM quote fixture cannot submit or query other methods"),
        },
        Reply::DammSimulation { result, mode } => {
            assert_eq!(payload["method"], "simulateTransaction");
            (*mode, Some(result))
        }
        Reply::QuoteStatus { value, mode } => {
            assert_eq!(payload["method"], "getSignatureStatuses");
            (*mode, Some(value))
        }
        Reply::WsolBroadcast { signature, mode } => {
            assert_eq!(payload["method"], "sendTransaction");
            (*mode, Some(json!(signature)))
        }
        Reply::WsolSimulation { value, mode } => {
            assert_eq!(payload["method"], "simulateTransaction");
            (*mode, Some(json!({"context":{"slot":42},"value":value})))
        }
        Reply::WsolQuote {
            values,
            fail_method,
            failure,
            normal,
        } => {
            let method = payload["method"].as_str().unwrap();
            let value = values
                .get(method)
                .expect("unscripted WSOL quote method")
                .clone();
            (
                if fail_method == Some(method) {
                    *failure
                } else {
                    *normal
                },
                Some(value),
            )
        }
        Reply::AggregatorSimulation {
            baseline,
            simulation,
            baseline_value,
            simulation_value,
        } => match payload["method"].as_str() {
            Some("getMultipleAccounts") => (*baseline, Some(baseline_value)),
            Some("simulateTransaction") => (*simulation, Some(simulation_value)),
            _ => (Reply::Success, None),
        },
        Reply::Parameters {
            accounts,
            epoch,
            other,
        } => {
            let value = match payload["method"].as_str() {
                Some("getAccountInfo") => Some(
                    json!({"context":{"slot":123},"value":accounts.get(payload["params"][0].as_str().unwrap()).expect("unscripted parameter account")}),
                ),
                Some("getMultipleAccounts")
                    if payload["params"][0]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|address| accounts.contains_key(address.as_str().unwrap())) =>
                {
                    Some(
                        json!({"context":{"slot":123},"value":payload["params"][0].as_array().unwrap().iter().map(|address| accounts.get(address.as_str().unwrap()).unwrap()).collect::<Vec<_>>()}),
                    )
                }
                Some("getEpochInfo") => {
                    let epoch = epoch.expect("unscripted parameter epoch");
                    Some(
                        json!({"epoch":epoch,"slotIndex":1,"slotsInEpoch":432000,"absoluteSlot":epoch*432000+1,"blockHeight":42,"transactionCount":100}),
                    )
                }
                Some("getProgramAccounts") => Some(json!([])),
                Some("getTokenAccountBalance") => {
                    Some(json!({"context":{"slot":123},"value":quote_amount("23")}))
                }
                _ => None,
            };
            (*other, value)
        }
        other => (other, None),
    };
    let parameter_value = parameter_value.or(preview).or(evidence);
    let is_blockhash = payload["method"] == "getLatestBlockhash";
    let is_preparation = parameter_value.is_none()
        && payload["method"] == "getMultipleAccounts"
        && payload["params"][1]["dataSlice"].is_null();
    let is_balances =
        parameter_value.is_none() && payload["method"] == "getMultipleAccounts" && !is_preparation;
    let is_scan = payload["method"] == "getTokenAccountsByOwner";
    let is_quote_scalar = matches!(
        payload["method"].as_str(),
        Some("getTokenSupply" | "getTokenAccountBalance" | "getBalance")
    );
    let body_limit = if is_sol_transfer {
        MAX_BODY
    } else if is_quote_status {
        MAX_BODY
    } else if is_wsol_simulation {
        524_288
    } else if is_wsol_quote {
        MAX_BODY
    } else if parameter_value.is_some() && payload["method"] == "getProgramAccounts" {
        1_048_576
    } else if is_scan || parameter_value.is_some() {
        524_288
    } else {
        MAX_BODY
    };
    let mut preparation_value = None;
    let reply = match reply {
        Reply::Preparation {
            scan,
            raw,
            scan_value,
            raw_value,
        } => {
            if is_scan {
                preparation_value = Some(scan_value);
                *scan
            } else if is_preparation {
                preparation_value = Some(raw_value);
                *raw
            } else {
                Reply::Success
            }
        }
        Reply::QuoteScan(inner) if is_scan => *inner,
        Reply::QuoteScan(_) => Reply::Success,
        Reply::QuoteAtaFirstError if is_scan => Reply::RpcError,
        Reply::QuoteAtaFirstError if payload["method"] == "getTokenAccountBalance" => {
            let first = {
                let history = requests.lock().unwrap();
                let scan = history
                    .iter()
                    .rev()
                    .find(|request| request["method"] == "getTokenAccountsByOwner")
                    .unwrap();
                let wallet = scan["params"][0]
                    .as_str()
                    .unwrap()
                    .parse::<solana_sdk::pubkey::Pubkey>()
                    .unwrap();
                let mint = scan["params"][1]["mint"]
                    .as_str()
                    .unwrap()
                    .parse::<solana_sdk::pubkey::Pubkey>()
                    .unwrap();
                sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &wallet,
                    &mint,
                    &spl_token_interface::ID,
                )
                .to_string()
            };
            if payload["params"][0] == first {
                Reply::RpcError
            } else {
                Reply::Success
            }
        }
        Reply::QuoteAtaFirstError => Reply::Success,
        other => other,
    };
    if is_runtime_evidence {
        assert_eq!(
            payload,
            json!({"jsonrpc":"2.0","id":id,"method":payload["method"],"params":payload["params"]})
        );
    } else if is_damm_quote {
        let config = &payload["params"][1];
        let keys = if payload["method"] == "getMultipleAccounts" {
            assert!(config["minContextSlot"].is_null() || config["minContextSlot"] == 123);
            assert_eq!(
                *config,
                json!({"encoding":"base64","commitment":"processed","dataSlice":null,"minContextSlot":config["minContextSlot"]})
            );
            &payload["params"][0]
        } else {
            assert_eq!(
                *config,
                json!({"sigVerify":false,"replaceRecentBlockhash":false,"commitment":"processed","encoding":"base64",
                "accounts":{"encoding":"base64","addresses":config["accounts"]["addresses"]},"minContextSlot":123,"innerInstructions":true})
            );
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                payload["params"][0].as_str().unwrap(),
            )
            .unwrap();
            assert!(bytes.len() <= 1232);
            let tx: solana_sdk::transaction::Transaction = bincode::deserialize(&bytes).unwrap();
            assert_eq!(tx.signatures, [solana_sdk::signature::Signature::default()]);
            assert_eq!(tx.message.header.num_required_signatures, 1);
            &config["accounts"]["addresses"]
        };
        let keys = keys.as_array().unwrap();
        assert!(matches!(keys.len(), 8 | 9));
        let addresses = keys
            .iter()
            .map(|key| key.as_str().unwrap())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(addresses.len(), keys.len());
        for address in addresses {
            assert_eq!(bs58::decode(address).into_vec().unwrap().len(), 32);
        }
    } else if is_runtime_preview {
        assert_eq!(
            payload,
            json!({"jsonrpc":"2.0","id":id,"method":"simulateTransaction","params":payload["params"]})
        );
    } else if is_damm_simulation {
        let config = &payload["params"][1];
        assert_eq!(
            *config,
            json!({"sigVerify":false,"replaceRecentBlockhash":false,
            "commitment":"processed","encoding":"base64","accounts":null,
            "minContextSlot":123,"innerInstructions":true})
        );
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            payload["params"][0].as_str().unwrap(),
        )
        .unwrap();
        assert!(bytes.len() <= 1232);
        let tx: solana_sdk::transaction::VersionedTransaction =
            bincode::deserialize(&bytes).unwrap();
        assert_eq!(tx.signatures, [solana_sdk::signature::Signature::default()]);
        assert!(matches!(
            tx.message,
            solana_sdk::message::VersionedMessage::Legacy(_)
        ));
        assert_eq!(tx.message.header().num_required_signatures, 1);
    } else if is_wsol_simulation {
        let config = &payload["params"][1];
        assert_eq!(config["sigVerify"], false);
        assert_eq!(config["replaceRecentBlockhash"], false);
        assert_eq!(config["innerInstructions"], false);
        assert_eq!(config["encoding"], "base64");
        assert!(config["accounts"].is_null() && config["minContextSlot"].is_null());
        assert!(matches!(
            config["commitment"].as_str(),
            Some("processed" | "confirmed" | "finalized")
        ));
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            payload["params"][0].as_str().unwrap(),
        )
        .unwrap();
        assert!(bytes.len() <= 1232);
        let tx: solana_sdk::transaction::Transaction = bincode::deserialize(&bytes).unwrap();
        assert!((1..=2).contains(&tx.message.header.num_required_signatures));
    } else if is_sol_transfer {
        match payload["method"].as_str().unwrap() {
            "getMinimumBalanceForRentExemption" => assert_eq!(payload["params"], json!([0])),
            "getLatestBlockhash" => assert!(matches!(
                payload["params"][0]["commitment"].as_str(),
                Some("processed" | "confirmed" | "finalized")
            )),
            "getBalance" | "getFeeForMessage" => {
                assert!(matches!(
                    payload["params"][1]["commitment"].as_str(),
                    Some("processed" | "confirmed" | "finalized")
                ));
                if payload["method"] == "getBalance" {
                    assert_eq!(
                        payload["params"][0],
                        solana_sdk::pubkey::Pubkey::new_from_array([1; 32]).to_string()
                    );
                } else {
                    let bytes = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        payload["params"][0].as_str().unwrap(),
                    )
                    .unwrap();
                    let source = solana_sdk::pubkey::Pubkey::new_from_array([1; 32]);
                    let destination = solana_sdk::pubkey::Pubkey::new_from_array([2; 32]);
                    let expected = solana_sdk::message::Message::new_with_blockhash(
                        &[solana_system_interface::instruction::transfer(
                            &source,
                            &destination,
                            1,
                        )],
                        Some(&source),
                        &solana_sdk::hash::Hash::new_from_array([6; 32]),
                    );
                    assert_eq!(bytes, expected.serialize());
                }
            }
            _ => panic!("SOL transfer preparation fixture cannot accept simulation or sending"),
        }
    } else if is_wsol_quote {
        match payload["method"].as_str().unwrap() {
            "getMinimumBalanceForRentExemption" => assert_eq!(payload["params"], json!([165])),
            "getAccountInfo" => {
                assert_eq!(payload["params"][1]["encoding"], "base64");
                assert_eq!(payload["params"][1]["commitment"], "processed");
            }
            "getLatestBlockhash" => assert!(matches!(
                payload["params"][0]["commitment"].as_str(),
                Some("processed" | "confirmed" | "finalized")
            )),
            "getBalance" | "getFeeForMessage" => assert!(matches!(
                payload["params"][1]["commitment"].as_str(),
                Some("processed" | "confirmed" | "finalized")
            )),
            _ => panic!("WSOL quote fixture cannot accept simulation or sending"),
        }
    } else if is_wsol_broadcast {
        assert_eq!(payload["method"], "sendTransaction");
        assert_eq!(payload["params"][1]["encoding"], "base64");
        assert_eq!(payload["params"][1]["skipPreflight"], false);
        assert!(matches!(
            payload["params"][1]["preflightCommitment"].as_str(),
            Some("processed" | "confirmed" | "finalized")
        ));
        assert!(payload["params"][1]["maxRetries"].is_null());
        assert!(payload["params"][1]["minContextSlot"].is_null());
    } else if is_quote_status {
        assert_eq!(payload, request(id, &signature()));
    } else if parameter_value.is_some() {
        if payload["method"] == "simulateTransaction" {
            assert_eq!(payload["params"][1]["sigVerify"], false);
            assert_eq!(payload["params"][1]["replaceRecentBlockhash"], false);
            assert_eq!(payload["params"][1]["innerInstructions"], true);
            assert_eq!(payload["params"][1]["commitment"], "processed");
            assert_eq!(payload["params"][1]["encoding"], "base64");
            assert_eq!(
                payload["params"][1]["accounts"]["addresses"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert!(payload["params"][1]["minContextSlot"].as_u64().unwrap() > 0);
        } else if payload["method"] == "getEpochInfo"
            || payload["method"] == "getTokenAccountBalance"
        {
            let config_index = if payload["method"] == "getEpochInfo" {
                0
            } else {
                1
            };
            assert!(matches!(
                payload["params"][config_index]["commitment"].as_str(),
                Some("processed" | "confirmed" | "finalized")
            ));
        } else {
            assert_eq!(payload["params"][1]["encoding"], "base64");
            assert!(payload["params"][1]["dataSlice"].is_null());
            assert!(payload["params"][1]["minContextSlot"].is_null());
        }
    } else if is_blockhash {
        assert_eq!(
            payload,
            json!({"jsonrpc":"2.0","id":id,"method":"getLatestBlockhash","params":[{"commitment":"processed"}]})
        );
    } else if is_preparation {
        let keys = payload["params"][0].as_array().unwrap();
        assert_eq!(keys.len(), 3);
        let minimum = payload["params"][1]["minContextSlot"].clone();
        assert!(minimum.is_null() || minimum.as_u64().is_some());
        assert_eq!(
            payload,
            json!({"jsonrpc":"2.0","id":id,"method":"getMultipleAccounts","params":[keys,{"encoding":"base64","dataSlice":null,"commitment":"processed","minContextSlot":minimum}]})
        );
    } else if is_balances {
        let wallets = payload["params"][0].as_array().unwrap();
        assert!(!wallets.is_empty() && wallets.len() <= 100);
        assert_eq!(
            payload,
            json!({"jsonrpc":"2.0","id":id,"method":"getMultipleAccounts","params":[wallets,{"commitment":"processed","encoding":"base64","dataSlice":{"offset":0,"length":0}}]})
        );
    } else if is_scan {
        let wallet = payload["params"][0].as_str().unwrap();
        let mint = payload["params"][1]["mint"].as_str().unwrap();
        assert_eq!(
            payload,
            json!({"jsonrpc":"2.0","id":id,"method":"getTokenAccountsByOwner","params":[wallet,{"mint":mint},{"encoding":"jsonParsed","dataSlice":null,"commitment":"processed","minContextSlot":null}]})
        );
    } else if is_quote_scalar {
        let method = payload["method"].as_str().unwrap();
        let address = payload["params"][0].as_str().unwrap();
        assert_eq!(bs58::decode(address).into_vec().unwrap().len(), 32);
        assert_eq!(
            payload,
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":[address,{"commitment":"processed"}]})
        );
    } else {
        let expected = match &reply {
            Reply::ObservedSignature(value) => value.clone(),
            _ => signature(),
        };
        assert_eq!(payload, request(id, &expected));
    }
    let balances = if is_balances {
        let mut values = payload["params"][0].as_array().unwrap().iter().map(|wallet| {
            let bytes = bs58::decode(wallet.as_str().unwrap()).into_vec().unwrap();
            assert_eq!(bytes.len(), 32);
            let byte = bytes[0];
            if byte == 0 { Value::Null }
            else {
                let lamports = if matches!(reply, Reply::FailAfter(_)) { 999 } else if byte == 1 { u64::MAX } else { u64::from(byte) };
                json!({"lamports":lamports,"owner":"11111111111111111111111111111111","data":["","base64"],"executable":false,"rentEpoch":0,"space":0})
            }
        }).collect::<Vec<_>>();
        if matches!(reply, Reply::WrongAccountCount) {
            values.pop();
        }
        Some(values)
    } else {
        None
    };
    let quote_value = if is_scan {
        let wallet = payload["params"][0].as_str().unwrap();
        let mint = payload["params"][1]["mint"].as_str().unwrap();
        let mut value = json!([
            quote_account(3, wallet, mint, "9007199254740993"),
            quote_account(4, wallet, mint, "7")
        ]);
        match reply {
            Reply::QuoteEmpty => value = json!([]),
            Reply::QuoteUnparsed => {
                value[0]["account"]["data"] = json!(["", "base64"]);
                value.as_array_mut().unwrap().pop();
            }
            Reply::QuoteMixed => value[1]["account"]["data"] = json!(["", "base64"]),
            Reply::QuoteWrongMint => {
                value[0]["account"]["data"]["parsed"]["info"]["mint"] =
                    json!(bs58::encode([99u8; 32]).into_string())
            }
            _ => {}
        }
        Some(value)
    } else {
        match payload["method"].as_str() {
            Some("getTokenSupply") => Some(quote_amount("10000000000000000000")),
            Some("getTokenAccountBalance") => Some(quote_amount("23")),
            Some("getBalance") => Some(json!(123456789)),
            _ => None,
        }
    };
    let request_count = {
        let mut requests = requests.lock().unwrap();
        requests.push(payload);
        requests.len()
    };
    received.notify_one();
    if matches!(reply, Reply::Disconnect) {
        return;
    }
    if matches!(reply, Reply::SlowHeaders) {
        std::future::pending::<()>().await;
    }
    if let Reply::DelayedHeaders(ms) = reply {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }

    let response_id = if matches!(reply, Reply::WrongId) {
        id + 1
    } else {
        id
    };
    let mut body = serde_json::to_vec(&if let Some(value)=parameter_value {
        json!({"jsonrpc":"2.0","id":response_id,"result":value})
    } else if let Some(value)=preparation_value {
        json!({"jsonrpc":"2.0","id":response_id,"result":{"context":{"slot":123},"value":value}})
    } else if is_blockhash {
        json!({"jsonrpc":"2.0","id":response_id,"result":{"context":{"slot":123},"value":{"blockhash":bs58::encode([7u8;32]).into_string(),"lastValidBlockHeight":456}}})
    } else if let Some(balances) = balances {
        json!({"jsonrpc":"2.0","id":response_id,"result":{"context":{"slot":123},"value":balances}})
    } else if let Some(value) = quote_value {
        json!({"jsonrpc":"2.0","id":response_id,"result":{"context":{"slot":123},"value":value}})
    } else { response(response_id) }).unwrap();
    if matches!(reply, Reply::AtLimit) {
        body.resize(body_limit, b' ');
    }
    if let Reply::Chunked(size) = reply {
        body.resize(size, b' ');
        if stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .is_err()
        {
            return;
        }
        for chunk in body.chunks(4096) {
            if stream
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await
                .is_err()
                || stream.write_all(chunk).await.is_err()
                || stream.write_all(b"\r\n").await.is_err()
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        let _ = stream.write_all(b"0\r\n\r\n").await;
        return;
    }
    let mut status = "200 OK";
    let mut extra = String::new();
    match &reply {
        Reply::RateLimited(retry) => {
            status = "429 Too Many Requests";
            if let Some(value) = retry {
                extra = format!("Retry-After: {value}\r\n");
            }
            body = b"fixture-secret provider diagnostic".to_vec();
        }
        Reply::Redirect(location) => {
            status = "302 Found";
            extra = format!("Location: {location}\r\n");
            body.clear();
        }
        Reply::InvalidJson => body = b"fixture-secret invalid json".to_vec(),
        Reply::RpcError => body = serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32005,"message":"fixture-secret"}}),
        )
        .unwrap(),
        Reply::ServerError => {
            status = "503 Service Unavailable";
            body = b"fixture-secret".to_vec();
        }
        Reply::FailAfter(limit) if request_count > *limit => {
            status = "503 Service Unavailable";
            body = b"fixture-secret".to_vec();
        }
        _ => {}
    }
    let length = if matches!(reply, Reply::DeclaredOversize) {
        body_limit + 1
    } else {
        body.len()
    };
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n{extra}Connection: close\r\n\r\n");
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    if matches!(reply, Reply::DeclaredOversize) {
        return;
    }
    if matches!(reply, Reply::SlowBody) {
        let _ = stream.write_all(&body[..5]).await;
        std::future::pending::<()>().await;
    }
    let _ = stream.write_all(&body).await;
}
