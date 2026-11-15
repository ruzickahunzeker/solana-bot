use super::*;
use anyhow::{bail, Context};
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
    Frozen(DammBuyCandidate),
    Rejected,
}

#[derive(Clone)]
struct Guard {
    profile: Arc<DammBuyMessage>,
    stage: Arc<Mutex<Stage>>,
    cancelled: Arc<AtomicBool>,
}

pub(super) struct Invocation(Guard);

impl Invocation {
    pub(super) fn new(profile: Arc<DammBuyMessage>) -> Self {
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
        if !report.success && report.signatures.is_empty() {
            return Ok(report);
        }
        let checked = (|| -> Result<()> {
            let stage = self
                .0
                .stage
                .lock()
                .map_err(|_| anyhow::anyhow!("DAMM buy message guard unavailable"))?;
            let Stage::Frozen(candidate) = &*stage else {
                bail!("DAMM buy SDK returned signatures without a verified final candidate");
            };
            ensure!(
                report.signatures.len() == 1,
                "DAMM buy SDK signature count changed"
            );
            let signature = Signature::from_str(&report.signatures[0])
                .context("DAMM buy SDK returned an invalid signature")?;
            ensure!(
                signature != Signature::default()
                    && signature.verify(
                        self.0.profile.quote.context.wallet.as_ref(),
                        &candidate.transaction.message.serialize()
                    ),
                "DAMM buy SDK signature does not bind the frozen final message"
            );
            Ok(())
        })();
        checked.map_err(|error| SdkInvocationFailure {
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

impl Guard {
    fn check(&self, name: &str, buy: bool) -> Result<()> {
        ensure!(
            !self.cancelled.load(Ordering::Acquire),
            "DAMM buy invocation was cancelled or completed"
        );
        ensure!(
            name == "MeteoraDammV2" && buy,
            "DAMM buy callback direction or protocol changed"
        );
        self.profile.recheck()
    }
    fn candidate(&self, actual: &[Instruction]) -> Result<DammBuyCandidate> {
        let quote = &self.profile.quote;
        let wants_tip = !quote.intent.simulate
            && quote.context.provider != SwqosType::Default
            && quote.intent.tip_sol > 0.0;
        let destination = if wants_tip {
            Some(
                actual
                    .first()
                    .and_then(|i| i.accounts.get(1))
                    .context("DAMM buy relay tip destination is missing")?
                    .pubkey,
            )
        } else {
            None
        };
        let candidate = self.profile.prepare(destination)?;
        candidate.validate_full(actual)?;
        Ok(candidate)
    }
}

impl InstructionMiddleware for Guard {
    fn name(&self) -> &'static str {
        "ordinary-damm-buy-candidate"
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
            .map_err(|_| anyhow::anyhow!("DAMM buy message guard unavailable"))?;
        let checked = (|| -> Result<()> {
            self.check(name, buy)?;
            ensure!(
                matches!(*stage, Stage::Pending),
                "DAMM buy protocol callback is not first"
            );
            ensure!(
                actual == self.profile.protocol,
                "DAMM buy protocol instructions changed"
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
            .map_err(|_| anyhow::anyhow!("DAMM buy message guard unavailable"))?;
        let checked = (|| -> Result<DammBuyCandidate> {
            self.check(name, buy)?;
            ensure!(
                matches!(*stage, Stage::Protocol),
                "DAMM buy full callback is out of order or repeated"
            );
            self.candidate(&actual)
        })();
        match checked {
            Ok(candidate) => *stage = Stage::Frozen(candidate),
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
