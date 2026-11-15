use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::sync::Notify;

pub(crate) type Trace = Arc<Mutex<Vec<String>>>;

pub(crate) struct Metadata {
    pub wallets: Vec<WalletMetadata>,
    pub error: Option<&'static str>,
    pub trace: Trace,
}

impl WalletMetadataSource for Metadata {
    async fn wallet(&self, id: &str) -> anyhow::Result<WalletMetadata> {
        self.trace.lock().unwrap().push(format!("get:{id}"));
        if let Some(error) = self.error {
            anyhow::bail!(error);
        }
        self.wallets
            .iter()
            .find(|wallet| wallet.wallet_id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("wallet not found"))
    }
    async fn wallets(&self) -> anyhow::Result<Vec<WalletMetadata>> {
        self.trace.lock().unwrap().push("list".into());
        if let Some(error) = self.error {
            anyhow::bail!(error);
        }
        Ok(self.wallets.clone())
    }
}

pub(crate) struct Parser(pub Trace);
impl TradeAddressParser for Parser {
    fn parse(&self, value: &str) -> anyhow::Result<Address> {
        self.0.lock().unwrap().push(format!("parse:{value}"));
        let byte = match value {
            "pub-1" => 1,
            "pub-2" => 2,
            "pub-3" => 3,
            "pub-4" => 4,
            "mint" => 9,
            _ => anyhow::bail!("synthetic address error"),
        };
        Ok(Address::from_bytes([byte; 32]))
    }
}

#[derive(Default)]
pub(crate) struct Gate {
    pub entered: Notify,
    pub dropped: AtomicBool,
}

struct PendingRead(Arc<Gate>);
impl Drop for PendingRead {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub(crate) struct Runtime {
    pub trace: Trace,
    pub loaded: bool,
    pub quote_error: Option<&'static str>,
    pub native_error: Option<&'static str>,
    pub balances: Vec<u64>,
    pub inputs: Arc<Mutex<Vec<Vec<Address>>>>,
    pub gate: Option<Arc<Gate>>,
}

impl Runtime {
    pub(crate) fn new(trace: Trace) -> Self {
        Self {
            trace,
            loaded: true,
            quote_error: None,
            native_error: None,
            balances: vec![11, 22, 33],
            inputs: Arc::default(),
            gate: None,
        }
    }
    async fn read_gate(&self) {
        if let Some(gate) = &self.gate {
            let _pending = PendingRead(gate.clone());
            gate.entered.notify_one();
            std::future::pending::<()>().await;
        }
    }
}

impl WalletBalanceRuntime for Runtime {
    type Reader = Self;
    fn reader(&self, id: &str) -> Option<Self::Reader> {
        self.trace.lock().unwrap().push(format!("reader:{id}"));
        self.loaded.then(|| self.clone())
    }
    fn default_reader(&self) -> Self::Reader {
        self.trace.lock().unwrap().push("default".into());
        self.clone()
    }
}

impl WalletReadPort for Runtime {
    async fn quote_balance(&self, mint: Address) -> anyhow::Result<QuoteBalance> {
        self.trace.lock().unwrap().push(format!("quote:{mint}"));
        self.read_gate().await;
        if let Some(error) = self.quote_error {
            anyhow::bail!(error);
        }
        Ok(QuoteBalance {
            decimals: 6,
            balance_units: u64::MAX,
            balance_lamports: 42,
        })
    }
    async fn wallet_balances(&self, addresses: &[Address]) -> anyhow::Result<Vec<u64>> {
        self.trace
            .lock()
            .unwrap()
            .push(format!("batch:{}", addresses.len()));
        self.inputs.lock().unwrap().push(addresses.to_vec());
        self.read_gate().await;
        if let Some(error) = self.native_error {
            anyhow::bail!(error);
        }
        Ok(self.balances.clone())
    }
}
