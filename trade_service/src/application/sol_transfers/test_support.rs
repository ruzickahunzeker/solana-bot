//! Keyless metadata, real temporary journal, and synthetic execution only.
use super::*;
use crate::{
    adapters::{
        persistence::sol_transfer_journal::SqliteSolTransferJournal,
        sdk::address_parser::SdkAddressParser,
    },
    application::ports::sol_transfer_execution::{
        SolTransferArtifact, SolTransferExecutionWallet, SolTransferRuntime, TransferWalletMetadata,
    },
    domain::{
        sol_transfer::{SolTransferAmount, SolTransferSnapshot},
        submission::SignedSubmission,
        trade::Address,
    },
    wallet_repository::WalletRepository,
};
use anyhow::{bail, Result};
use sqlx::SqlitePool;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

pub(crate) const KEY: &str = "fixture-transfer-key";
pub(crate) const SIG: &str = "synthetic-signature";
#[derive(Clone, Default)]
pub(crate) struct Faults {
    pub lookup: bool,
    pub no_journal: bool,
    pub prepare: bool,
    pub sign: bool,
    pub broadcast: bool,
    pub different_receipt: bool,
    pub unconfirmed: bool,
    pub bad_snapshot: bool,
    pub changed_signed_snapshot: bool,
    pub replace_wallet: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Reserved,
    Prepare,
    RecordedSigned,
    Broadcast,
    Confirm,
}
pub(crate) struct Trace {
    events: Mutex<Vec<&'static str>>,
    pub faults: Mutex<Faults>,
    pub wallets: Mutex<BTreeMap<String, Address>>,
    gate: Mutex<Option<(Stage, Arc<Notify>, Arc<Notify>)>>,
}
impl Trace {
    pub fn push(&self, event: &'static str) {
        self.events.lock().unwrap().push(event);
    }
    pub fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
    pub fn count(&self, event: &str) -> usize {
        self.events()
            .iter()
            .filter(|&&value| value == event)
            .count()
    }
    pub fn faults(&self) -> Faults {
        self.faults.lock().unwrap().clone()
    }
    pub fn gate(&self, stage: Stage) -> (Arc<Notify>, Arc<Notify>) {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        *self.gate.lock().unwrap() = Some((stage, entered.clone(), release.clone()));
        (entered, release)
    }
    async fn wait(&self, stage: Stage) {
        let gate = self.gate.lock().unwrap().clone();
        if let Some((selected, entered, release)) = gate {
            if selected == stage {
                entered.notify_one();
                release.notified().await;
            }
        }
    }
}
pub(crate) struct Runtime {
    pub repo: Arc<WalletRepository>,
    pub pool: SqlitePool,
    pub trace: Arc<Trace>,
}
pub(crate) struct Wallet {
    pool: SqlitePool,
    trace: Arc<Trace>,
    address: Address,
}
pub(crate) struct Journal {
    inner: SqliteSolTransferJournal,
    trace: Arc<Trace>,
}
pub(crate) struct Artifact {
    snapshot: SolTransferSnapshot,
    submission: SignedSubmission,
}
impl SolTransferArtifact for Artifact {
    fn submission(&self) -> &SignedSubmission {
        &self.submission
    }
    fn snapshot(&self) -> &SolTransferSnapshot {
        &self.snapshot
    }
}
impl SolTransferRuntime for Runtime {
    type Wallet = Wallet;
    type Journal = Journal;
    async fn metadata(&self, id: &str) -> Result<TransferWalletMetadata> {
        self.trace.push("metadata");
        let row = self.repo.get(id).await?;
        Ok(TransferWalletMetadata {
            owner_type: row.owner_type,
            owner_id: row.owner_id,
            pubkey: row.pubkey,
            enabled: row.enabled,
        })
    }
    fn journal(&self) -> Option<Journal> {
        self.trace.push("journal");
        if self.trace.faults().no_journal {
            return None;
        }
        self.repo.sol_transfer_journal().map(|inner| Journal {
            inner,
            trace: self.trace.clone(),
        })
    }
    fn resolve(&self, id: &str) -> Option<Wallet> {
        self.trace.push("resolve");
        self.trace
            .wallets
            .lock()
            .unwrap()
            .get(id)
            .map(|address| Wallet {
                pool: self.pool.clone(),
                trace: self.trace.clone(),
                address: *address,
            })
    }
}
impl Drop for Wallet {
    fn drop(&mut self) {
        self.trace.push("drop");
    }
}
impl SolTransferExecutionWallet for Wallet {
    type Prepared = SolTransferSnapshot;
    type Signed = Artifact;
    fn pubkey(&self) -> Address {
        self.trace.push("pubkey");
        self.address
    }
    async fn prepare(
        &self,
        destination: Address,
        amount: SolTransferAmount,
    ) -> Result<Self::Prepared> {
        self.trace.push("prepare");
        let f = self.trace.faults();
        if f.prepare {
            bail!("synthetic prepare failure https://rpc.example.invalid/private");
        }
        if f.replace_wallet {
            self.trace
                .wallets
                .lock()
                .unwrap()
                .insert("source".into(), Address::from_bytes([9; 32]));
        }
        self.trace.wait(Stage::Prepare).await;
        let (fixed, percent) = amount.legacy_params();
        let (amount_lamports, reserve_lamports) =
            sol_transfer_policy::calculate_sol_transfer_amount(
                2_000_000, 5000, 890_880, fixed, percent,
            )?;
        Ok(SolTransferSnapshot {
            source: self.address,
            destination: if f.bad_snapshot {
                Address::from_bytes([9; 32])
            } else {
                destination
            },
            amount_lamports,
            fee_lamports: 5000,
            balance_before_lamports: 2_000_000,
            reserve_lamports,
        })
    }
    fn prepared_snapshot<'a>(&self, prepared: &'a Self::Prepared) -> &'a SolTransferSnapshot {
        prepared
    }
    fn sign(&self, mut snapshot: Self::Prepared) -> Result<Artifact> {
        self.trace.push("sign");
        let f = self.trace.faults();
        if f.sign {
            bail!("synthetic signer unavailable");
        }
        if f.changed_signed_snapshot {
            snapshot.amount_lamports += 1;
        }
        Ok(Artifact {
            snapshot,
            submission: SignedSubmission {
                signature: SIG.into(),
                bytes: vec![1, 2, 3],
            },
        })
    }
    async fn broadcast(&self, signed: &Artifact) -> Result<String> {
        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM wallet_sol_transfer_attempts WHERE state='signed' AND signature=? AND signed_transaction=?")
            .bind(&signed.submission.signature).bind(&signed.submission.bytes).fetch_one(&self.pool).await?;
        assert_eq!(
            count, 1,
            "send is only allowed after both signed bytes and signature are stored"
        );
        self.trace.push("broadcast");
        self.trace.wait(Stage::Broadcast).await;
        let f = self.trace.faults();
        if f.broadcast {
            bail!("synthetic send timeout https://rpc.example.invalid/private");
        }
        Ok(if f.different_receipt {
            "different-signature".into()
        } else {
            signed.submission.signature.clone()
        })
    }
    async fn confirm(&self, _: &Artifact) -> bool {
        self.trace.push("confirm");
        self.trace.wait(Stage::Confirm).await;
        !self.trace.faults().unconfirmed
    }
}
impl SolTransferJournal for Journal {
    async fn lookup(&self, key: &str) -> Result<Option<SolTransferJournalEntry>> {
        self.trace.push("lookup");
        if self.trace.faults().lookup {
            bail!("synthetic final read failure");
        }
        self.inner.lookup(key).await
    }
    async fn reserve(&self, intent: &SolTransferIntent) -> Result<SolTransferReservation> {
        self.trace.push("reserve");
        let result = self.inner.reserve(intent).await?;
        if matches!(&result, SolTransferReservation::Created(_)) {
            self.trace.wait(Stage::Reserved).await;
        }
        Ok(result)
    }
    async fn record_signed(
        &self,
        id: &str,
        snapshot: &SolTransferSnapshot,
        signed: &SignedSubmission,
    ) -> Result<()> {
        self.trace.push("record_signed");
        self.inner.record_signed(id, snapshot, signed).await?;
        self.trace.wait(Stage::RecordedSigned).await;
        Ok(())
    }
    async fn fail_before_sign(&self, id: &str, error: &str) -> Result<()> {
        self.trace.push("fail_before_sign");
        self.inner.fail_before_sign(id, error).await
    }
    async fn record_broadcast(&self, id: &str, state: State, error: Option<&str>) -> Result<()> {
        self.trace.push("record_broadcast");
        self.inner.record_broadcast(id, state, error).await
    }
    async fn record_confirmed(&self, id: &str) -> Result<()> {
        self.trace.push("record_confirmed");
        self.inner.record_confirmed(id).await
    }
}

pub(crate) struct Harness {
    pub service: Arc<dyn SolTransfers>,
    pub repo: Arc<WalletRepository>,
    pub pool: SqlitePool,
    pub trace: Arc<Trace>,
    destination: String,
}
impl Harness {
    pub fn input(&self) -> SolTransferInput<'_> {
        SolTransferInput {
            owner_type: "telegram",
            owner_id: "42",
            to_wallet_id: None,
            to_address: Some(&self.destination),
            amount: TransferAmountInput {
                mode: sol_transfer_policy::TransferAmountMode::Fixed,
                fixed_lamports: Some(1_000_000),
                percent: None,
            },
            idempotency_key: KEY,
        }
    }
    pub fn policy(&self) -> SolTransferPolicy {
        SolTransferPolicy {
            broadcast: BroadcastPolicy::Enabled,
            allow_system_wallet: false,
        }
    }
    pub async fn submit(&self) -> Result<SolTransferOutcome, SolTransferError> {
        self.service
            .submit("source", self.input(), self.policy())
            .await
    }
    pub fn journal(&self) -> SqliteSolTransferJournal {
        self.repo.sol_transfer_journal().unwrap()
    }
}
pub(crate) async fn harness() -> Harness {
    let (repo, pool) = WalletRepository::empty_for_transfer_tests().await.unwrap();
    harness_with_repository(Arc::new(repo), pool).await
}
pub(crate) async fn harness_with_repository(
    repo: Arc<WalletRepository>,
    pool: SqlitePool,
) -> Harness {
    for (id, owner, byte) in [
        ("source", "42", 1),
        ("target", "42", 2),
        ("foreign", "7", 3),
    ] {
        sqlx::query("INSERT OR IGNORE INTO wallets(wallet_id,owner_type,owner_id,label,pubkey,encrypted_private_key,enabled) VALUES(?,'telegram',?,?,?,'not-a-valid-wallet',1)")
            .bind(id).bind(owner).bind(id).bind(Address::from_bytes([byte;32]).to_string()).execute(&pool).await.unwrap();
    }
    let trace = Arc::new(Trace {
        events: Mutex::default(),
        faults: Mutex::default(),
        wallets: Mutex::new(BTreeMap::from([(
            "source".into(),
            Address::from_bytes([1; 32]),
        )])),
        gate: Mutex::default(),
    });
    let service = Arc::new(SolTransferService::new(
        Runtime {
            repo: repo.clone(),
            pool: pool.clone(),
            trace: trace.clone(),
        },
        SdkAddressParser,
    ));
    Harness {
        service,
        repo,
        pool,
        trace,
        destination: Address::from_bytes([2; 32]).to_string(),
    }
}
