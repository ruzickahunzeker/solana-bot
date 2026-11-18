use super::*;
use crate::adapters::rpc::damm_sell_simulation::shared::{
    simulate_unsigned_at_slot, DammSimulator,
};

fn failure(error: anyhow::Error) -> SdkInvocationFailure {
    SdkInvocationFailure {
        signatures: vec![],
        message: error.to_string(),
    }
}

impl DammSellCandidate {
    pub(crate) async fn simulate(&self, rpc: &impl DammSimulator) -> Result<SdkLaneReport> {
        ensure!(
            self.profile.intent.simulate,
            "DAMM simulation requires its own unsigned profile"
        );
        self.profile.recheck()?;
        let result = simulate_unsigned_at_slot(
            rpc,
            self.transaction(),
            self.profile.quote.simulation_slot()?,
        )
        .await?;
        self.profile.recheck()?;
        Ok(SdkLaneReport {
            success: result.error.is_none(),
            signatures: vec![],
            error: result.error,
        })
    }
}

impl DammSellMessageProfile {
    pub(crate) fn prepare_simulation(self: &Arc<Self>) -> Result<DammSellCandidate> {
        ensure!(
            self.intent.simulate,
            "DAMM simulation requires its own unsigned profile"
        );
        DammSellCandidate::build(self.clone(), None)
    }

    pub(crate) fn prepare_live(self: &Arc<Self>, tip: Option<Pubkey>) -> Result<DammSellCandidate> {
        ensure!(
            !self.intent.simulate,
            "DAMM live candidate requires its original live profile"
        );
        DammSellCandidate::build(self.clone(), tip)
    }

    pub(crate) async fn execute<F, Fut>(
        self: &Arc<Self>,
        rpc: &impl DammSimulator,
        invoke: F,
    ) -> Result<SdkLaneReport, SdkInvocationFailure>
    where
        F: FnOnce(MiddlewareManager) -> Fut + Send,
        Fut: Future<Output = Result<SdkLaneReport, SdkInvocationFailure>> + Send,
    {
        if self.intent.simulate {
            return self
                .prepare_simulation()
                .map_err(failure)?
                .simulate(rpc)
                .await
                .map_err(failure);
        }
        self.invoke(invoke).await
    }

    pub(crate) async fn after_unsigned_preview<F, Fut>(
        self: &Arc<Self>,
        preview: &Arc<Self>,
        rpc: &impl DammSimulator,
        submit: F,
    ) -> Result<SdkLaneReport, SdkInvocationFailure>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<SdkLaneReport, SdkInvocationFailure>> + Send,
    {
        self.validate_unsigned_preview(preview, rpc)
            .await
            .map_err(failure)?;
        submit().await
    }

    pub(crate) async fn validate_unsigned_preview(
        self: &Arc<Self>,
        preview: &Arc<Self>,
        rpc: &impl DammSimulator,
    ) -> Result<()> {
        let checked = (|| -> Result<()> {
            ensure!(
                !self.intent.simulate
                    && preview.intent.simulate
                    && self.context == preview.context
                    && Arc::ptr_eq(&self.holding, &preview.holding)
                    && Arc::ptr_eq(&self.quote, &preview.quote)
                    && self.protocol == preview.protocol,
                "DAMM preview is not bound to this final candidate and balance evidence"
            );
            let mut intent = self.intent.clone();
            intent.simulate = true;
            preview.validate_intent(&intent)
        })();
        checked?;
        let result = preview.prepare_simulation()?.simulate(rpc).await?;
        if !result.success {
            anyhow::bail!(
                "DAMM v2 final transaction simulation failed: {}",
                result.error.unwrap_or_else(|| "simulation rejected".into())
            );
        }
        self.recheck()
    }
}

#[cfg(test)]
mod tests;
