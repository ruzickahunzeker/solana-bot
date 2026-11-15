use super::*;
use anyhow::bail;
use sol_trade_sdk::trading::middleware::traits::InstructionMiddleware;
use solana_sdk::signature::Signature;
use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

enum Stage {
    Pending,
    Protocol,
    Frozen(DammSellCandidate),
    Rejected,
}

#[derive(Clone)]
struct Guard {
    profile: Arc<DammSellMessageProfile>,
    stage: Arc<Mutex<Stage>>,
    cancelled: Arc<AtomicBool>,
}

pub(super) struct Invocation(Guard);

impl Invocation {
    pub(super) fn new(profile: Arc<DammSellMessageProfile>) -> Self {
        Self(Guard {
            profile,
            stage: Arc::new(Mutex::new(Stage::Pending)),
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }
    pub(super) fn middleware(&self) -> MiddlewareManager {
        MiddlewareManager::new().add_middleware(Box::new(self.0.clone()))
    }
    pub(super) fn finish(
        &self,
        result: Result<SdkLaneReport, SdkInvocationFailure>,
    ) -> Result<SdkLaneReport, SdkInvocationFailure> {
        let report = result?;
        // An SDK rejection before building has no candidate to attest and remains a rejection.
        if !report.success && report.signatures.is_empty() {
            return Ok(report);
        }
        let verified = (|| -> Result<()> {
            let stage = self
                .0
                .stage
                .lock()
                .map_err(|_| anyhow::anyhow!("DAMM message guard unavailable"))?;
            let Stage::Frozen(candidate) = &*stage else {
                bail!("DAMM SDK returned signatures without a verified final candidate");
            };
            verify_report(
                self.0.profile.context.wallet,
                &candidate.message_bytes(),
                &report.signatures,
            )
        })();
        verified.map_err(|error| SdkInvocationFailure {
            signatures: report.signatures.clone(),
            message: error.to_string(),
        })?;
        Ok(report)
    }
}

impl Drop for Invocation {
    fn drop(&mut self) {
        self.0.cancelled.store(true, Ordering::Release);
    }
}

fn verify_report(wallet: Pubkey, message: &[u8], signatures: &[String]) -> Result<()> {
    ensure!(
        signatures.len() == 1,
        "DAMM SDK signature count differs from the single candidate"
    );
    let signature =
        Signature::from_str(&signatures[0]).context("DAMM SDK returned an invalid signature")?;
    ensure!(
        signature != Signature::default() && signature.verify(wallet.as_ref(), message),
        "DAMM SDK signature does not bind the frozen final message"
    );
    Ok(())
}

impl Guard {
    fn check(&self, name: &str, is_buy: bool) -> Result<()> {
        ensure!(
            !self.cancelled.load(Ordering::Acquire),
            "DAMM message invocation was cancelled or completed"
        );
        ensure!(
            name == "MeteoraDammV2" && !is_buy,
            "DAMM message callback direction or protocol changed"
        );
        self.profile.recheck()
    }

    fn full_candidate(&self, actual: &[Instruction]) -> Result<DammSellCandidate> {
        let profile = &self.profile;
        let wants_tip = !profile.intent.simulate
            && profile.context.provider != SwqosType::Default
            && profile.intent.tip_sol > 0.0;
        let destination = if wants_tip {
            let tip = actual.first().context("DAMM final relay tip is missing")?;
            let destination = tip
                .accounts
                .get(1)
                .context("DAMM final relay tip destination is missing")?
                .pubkey;
            Some(destination)
        } else {
            None
        };
        let candidate = DammSellCandidate::build(profile.clone(), destination)?;
        candidate.validate_full(actual)?;
        Ok(candidate)
    }
}

impl InstructionMiddleware for Guard {
    fn name(&self) -> &'static str {
        "ordinary-damm-sell-candidate"
    }
    fn process_protocol_instructions(
        &self,
        actual: Vec<Instruction>,
        name: &str,
        buy: bool,
    ) -> Result<Vec<Instruction>> {
        let mut stage = self
            .stage
            .lock()
            .map_err(|_| anyhow::anyhow!("DAMM message guard unavailable"))?;
        let checked = (|| -> Result<()> {
            self.check(name, buy)?;
            ensure!(
                matches!(*stage, Stage::Pending),
                "DAMM protocol callback is not a first invocation"
            );
            ensure!(
                actual == self.profile.protocol,
                "DAMM SDK protocol instructions differ from the frozen profile"
            );
            Ok(())
        })();
        if let Err(error) = checked {
            *stage = Stage::Rejected;
            return Err(error);
        }
        *stage = Stage::Protocol;
        Ok(actual)
    }
    fn process_full_instructions(
        &self,
        actual: Vec<Instruction>,
        name: &str,
        buy: bool,
    ) -> Result<Vec<Instruction>> {
        let mut stage = self
            .stage
            .lock()
            .map_err(|_| anyhow::anyhow!("DAMM message guard unavailable"))?;
        let checked = (|| -> Result<DammSellCandidate> {
            self.check(name, buy)?;
            ensure!(
                matches!(*stage, Stage::Protocol),
                "DAMM full callback is missing or repeated protocol preparation"
            );
            self.full_candidate(&actual)
        })();
        match checked {
            Ok(message) => *stage = Stage::Frozen(message),
            Err(error) => {
                *stage = Stage::Rejected;
                return Err(error);
            }
        }
        Ok(actual)
    }
    fn clone_box(&self) -> Box<dyn InstructionMiddleware> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests;
