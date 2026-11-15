use super::*;
use crate::adapters::rpc::damm_sell_simulation::simulate_unsigned;
use solana_client::nonblocking::rpc_client::RpcClient;

fn failure(error: anyhow::Error) -> SdkInvocationFailure {
    SdkInvocationFailure {
        signatures: vec![],
        message: error.to_string(),
    }
}

impl DammSellCandidate {
    pub(crate) async fn simulate(&self, rpc: &RpcClient) -> Result<SdkLaneReport> {
        ensure!(
            self.profile.intent.simulate,
            "DAMM simulation requires its own unsigned profile"
        );
        self.profile.recheck()?;
        let result = simulate_unsigned(rpc, self.transaction()).await?;
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

    pub(crate) async fn execute<F, Fut>(
        self: &Arc<Self>,
        rpc: &RpcClient,
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
        rpc: &RpcClient,
        submit: F,
    ) -> Result<SdkLaneReport, SdkInvocationFailure>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<SdkLaneReport, SdkInvocationFailure>> + Send,
    {
        let checked = (|| -> Result<()> {
            ensure!(
                !self.intent.simulate
                    && preview.intent.simulate
                    && self.context == preview.context
                    && Arc::ptr_eq(&self.holding, &preview.holding)
                    && self.protocol == preview.protocol,
                "DAMM preview is not bound to this final candidate and balance evidence"
            );
            let mut intent = self.intent.clone();
            intent.simulate = true;
            preview.validate_intent(&intent)
        })();
        checked.map_err(failure)?;
        let result = preview
            .prepare_simulation()
            .map_err(failure)?
            .simulate(rpc)
            .await
            .map_err(failure)?;
        if !result.success {
            return Err(SdkInvocationFailure {
                signatures: vec![],
                message: format!(
                    "DAMM v2 final transaction simulation failed: {}",
                    result.error.unwrap_or_else(|| "simulation rejected".into())
                ),
            });
        }
        self.recheck().map_err(failure)?;
        submit().await
    }
}

#[cfg(test)]
mod tests;
