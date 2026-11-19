use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::sync::Notify;

pub(crate) const EXPORT_MARKER: &str = "explicit-export-not-a-key";

pub(crate) struct Material(Arc<AtomicUsize>);
impl Drop for Material {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

pub(crate) struct State {
    pub wallet: ManagedWallet,
    pub events: Vec<String>,
    pub inputs: Vec<(String, String, String, Option<String>)>,
    pub import_created: bool,
    pub failures: Vec<&'static str>,
}

#[derive(Clone)]
pub(crate) struct Backend {
    pub state: Arc<Mutex<State>>,
    pub drops: Arc<AtomicUsize>,
    pub install_gate: Option<Arc<Notify>>,
    pub mutation: Arc<tokio::sync::Mutex<()>>,
}

impl Backend {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                wallet: ManagedWallet {
                    wallet_id: "managed-wallet".into(),
                    owner_type: "telegram".into(),
                    owner_id: "42".into(),
                    label: "public label".into(),
                    pubkey: crate::domain::trade::Address::from_bytes([1; 32]).to_string(),
                    is_default: false,
                    enabled: true,
                },
                events: vec![],
                inputs: vec![],
                import_created: false,
                failures: vec![],
            })),
            drops: Arc::default(),
            install_gate: None,
            mutation: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    fn event(&self, op: &'static str, detail: String) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap();
        state.events.push(detail);
        if state.failures.contains(&op) {
            anyhow::bail!("synthetic {op} failure");
        }
        Ok(())
    }
    pub(crate) fn events(&self) -> Vec<String> {
        self.state.lock().unwrap().events.clone()
    }
    pub(crate) fn fail(&self, op: &'static str) {
        self.state.lock().unwrap().failures.push(op);
    }
    fn input(&self, input: &CreateWalletInput, secret: Option<&str>) {
        self.state.lock().unwrap().inputs.push((
            input.owner_type.clone(),
            input.owner_id.clone(),
            input.label.clone(),
            secret.map(str::to_owned),
        ));
    }
}

impl WalletRepositoryPort for Backend {
    type Material = Material;
    async fn list(&self) -> anyhow::Result<Vec<ManagedWallet>> {
        self.event("list", "list".into())?;
        Ok(vec![self.state.lock().unwrap().wallet.clone()])
    }
    async fn create(&self, input: &CreateWalletInput) -> anyhow::Result<(ManagedWallet, Material)> {
        self.event("create", "create".into())?;
        self.input(input, None);
        let mut state = self.state.lock().unwrap();
        state.wallet.label = input.label.clone();
        state.wallet.owner_type = input.owner_type.clone();
        state.wallet.owner_id = input.owner_id.clone();
        Ok((state.wallet.clone(), Material(self.drops.clone())))
    }
    async fn import(
        &self,
        input: &CreateWalletInput,
        secret: &str,
    ) -> anyhow::Result<(ManagedWallet, Material, bool)> {
        self.event("import", "import".into())?;
        self.input(input, Some(secret));
        let state = self.state.lock().unwrap();
        Ok((
            state.wallet.clone(),
            Material(self.drops.clone()),
            state.import_created,
        ))
    }
    async fn rename(&self, id: &str, label: &str) -> anyhow::Result<ManagedWallet> {
        self.event("rename", format!("rename:{id}:{label}"))?;
        let mut state = self.state.lock().unwrap();
        state.wallet.label = label.into();
        Ok(state.wallet.clone())
    }
    async fn disable(&self, id: &str) -> anyhow::Result<ManagedWallet> {
        self.event("disable", format!("disable:{id}"))?;
        let mut state = self.state.lock().unwrap();
        if state.wallet.is_default {
            anyhow::bail!("default wallet cannot be deleted");
        }
        state.wallet.enabled = false;
        Ok(state.wallet.clone())
    }
    async fn export(&self, id: &str) -> anyhow::Result<String> {
        self.event("export", format!("export:{id}"))?;
        Ok(EXPORT_MARKER.into())
    }
}

impl WalletRuntimePort<Material> for Backend {
    type Mutation<'a> = tokio::sync::MutexGuard<'a, ()>;
    async fn begin_mutation(&self) -> Self::Mutation<'_> {
        self.mutation.lock().await
    }
    fn loaded(&self) -> LoadedWallets {
        self.state.lock().unwrap().events.push("loaded".into());
        LoadedWallets {
            default_wallet_id: " runtime-default ".into(),
            wallets: vec![WalletInfo {
                wallet_id: "runtime-wallet".into(),
                pubkey: crate::domain::trade::Address::from_bytes([2; 32]).to_string(),
            }],
        }
    }
    async fn install(
        &self,
        _mutation: &mut Self::Mutation<'_>,
        id: &str,
        material: Material,
    ) -> anyhow::Result<()> {
        let _material = material;
        {
            self.state
                .lock()
                .unwrap()
                .events
                .push(format!("install:{id}"));
        }
        if let Some(gate) = &self.install_gate {
            gate.notify_one();
            std::future::pending::<()>().await;
        }
        if self.state.lock().unwrap().failures.contains(&"install") {
            anyhow::bail!("synthetic install failure");
        }
        Ok(())
    }
    fn remove(&self, _mutation: &mut Self::Mutation<'_>, id: &str) -> anyhow::Result<()> {
        self.event("remove", format!("remove:{id}"))
    }
}

pub(crate) fn input(secret: Option<&str>) -> CreateWalletInput {
    CreateWalletInput {
        owner_type: " telegram ".into(),
        owner_id: " 42 ".into(),
        label: " new label ".into(),
        private_key_base58: secret.map(str::to_owned),
    }
}
