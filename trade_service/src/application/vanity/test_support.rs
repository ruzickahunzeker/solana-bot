use super::*;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedView {
    pub owner_type: String,
    pub owner_id: String,
    pub label: String,
    pub match_side: String,
    pub pattern: String,
    pub max_attempts: u64,
}

pub(crate) struct State {
    pub job: VanityJob,
    pub events: Vec<String>,
    pub prepared: Vec<PreparedView>,
    pub failure: Option<&'static str>,
}

#[derive(Clone)]
pub(crate) struct Worker {
    pub state: Arc<Mutex<State>>,
    pub create_gate: Option<Arc<Notify>>,
}

impl Worker {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                job: VanityJob {
                    job_id: "public-job".into(),
                    owner_type: "telegram".into(),
                    owner_id: "42".into(),
                    label: "fixture".into(),
                    match_side: "prefix".into(),
                    pattern: "9999".into(),
                    status: "queued".into(),
                    attempts: 0,
                    max_attempts: 30_000_000,
                    wallet_id: None,
                    pubkey: None,
                    error: None,
                },
                events: vec![],
                prepared: vec![],
                failure: None,
            })),
            create_gate: None,
        }
    }
    pub(crate) fn events(&self) -> Vec<String> {
        self.state.lock().unwrap().events.clone()
    }
}

impl VanityWorkerPort for Worker {
    async fn create(&self, command: VanityCreateCommand) -> Result<VanityJob> {
        let job = {
            let mut state = self.state.lock().unwrap();
            state.events.push("create".into());
            if state.failure == Some("create") {
                bail!("synthetic create failure");
            }
            let prepared = PreparedView {
                owner_type: command.owner_type().into(),
                owner_id: command.owner_id().into(),
                label: command.label().into(),
                match_side: command.match_side().into(),
                pattern: command.pattern().into(),
                max_attempts: command.max_attempts(),
            };
            state.job.owner_type = prepared.owner_type.clone();
            state.job.owner_id = prepared.owner_id.clone();
            state.job.label = prepared.label.clone();
            state.job.match_side = prepared.match_side.clone();
            state.job.pattern = prepared.pattern.clone();
            state.job.max_attempts = prepared.max_attempts;
            state.prepared.push(prepared);
            state.job.clone()
        };
        if let Some(gate) = &self.create_gate {
            gate.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(job)
    }
    async fn get(&self, id: &str) -> Result<VanityJob> {
        let mut state = self.state.lock().unwrap();
        state.events.push(format!("get:{id}"));
        if state.failure == Some("get") {
            bail!("synthetic get failure");
        }
        Ok(state.job.clone())
    }
    async fn cancel(&self, id: &str, owner_type: &str, owner_id: &str) -> Result<VanityJob> {
        let mut state = self.state.lock().unwrap();
        state
            .events
            .push(format!("cancel:{id}:{owner_type}:{owner_id}"));
        if state.failure == Some("cancel") {
            bail!("synthetic cancel failure");
        }
        if cancellation_allowed(&state.job, owner_type, owner_id)? {
            state.job.status = "cancel_requested".into();
        }
        Ok(state.job.clone())
    }
}

pub(crate) fn input() -> VanityCreateInput {
    VanityCreateInput {
        owner_type: "telegram".into(),
        owner_id: "42".into(),
        label: " new vanity ".into(),
        match_side: VanityMatchSide::Prefix,
        pattern: "9999".into(),
        max_attempts: None,
    }
}
