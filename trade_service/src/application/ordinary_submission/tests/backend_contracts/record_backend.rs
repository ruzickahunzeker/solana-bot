use super::*;
use serde::{Deserialize, Serialize};

pub(super) struct RecordBackend(Arc<Probe>);

#[derive(Serialize, Deserialize)]
pub(super) struct RecordPlan {
    wallet: String,
    payer: String,
    asset_in: String,
    asset_out: String,
    debit: u64,
    estimated_credit: u64,
    credit_floor: u64,
    slippage_bps: u16,
    deadline: u64,
    path: Vec<(String, String, String)>,
}

impl FakeConstruction for RecordBackend {
    const SIGNATURE: &'static str = "non-executable-record-receipt";
    fn new(probe: Arc<Probe>) -> Self {
        Self(probe)
    }
    fn inspect(plan: &RecordPlan) -> Result<IntentView> {
        let bytes = serde_json::to_vec(plan)?;
        let value: RecordPlan = serde_json::from_slice(&bytes)?;
        Ok(IntentView {
            wallet_id: value.wallet,
            payer: value.payer,
            input: value.asset_in,
            output: value.asset_out,
            amount: value.debit,
            expected: value.estimated_credit,
            minimum: value.credit_floor,
            slippage: value.slippage_bps,
            expires: value.deadline,
            legs: value.path,
        })
    }
}

impl QuoteSubmissionBackend for RecordBackend {
    type Prepared = RecordPlan;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.0.event("policy");
        Ok(())
    }
    async fn prepare(&self, bound: &BoundQuote) -> Result<(RecordPlan, QuotePreflight)> {
        self.0.event("prepare");
        let quote = &bound.quote;
        ensure!(
            quote.input_mint == WSOL_MINT && quote.amount_lamports > 0,
            "record builder only supports positive SOL input"
        );
        ensure!(
            quote.minimum_output_units > 0
                && quote.minimum_output_units <= quote.expected_output_units,
            "record builder requires a positive output floor"
        );
        ensure!(
            quote.route.len() == 1 && quote.route[0].label == "fake-amm",
            "record builder does not support this protocol or route"
        );
        let plan = RecordPlan {
            wallet: bound.wallet_id.clone(),
            payer: bound.wallet_pubkey.clone(),
            asset_in: quote.input_mint.clone(),
            asset_out: quote.output_mint.clone(),
            debit: quote.amount_lamports,
            estimated_credit: quote.expected_output_units,
            credit_floor: quote.minimum_output_units,
            slippage_bps: bound.quote_slippage_bps,
            deadline: bound.expires_at_ms,
            path: quote
                .route
                .iter()
                .map(|leg| {
                    (
                        leg.label.clone(),
                        leg.input_mint.clone(),
                        leg.output_mint.clone(),
                    )
                })
                .collect(),
        };
        self.0.at("constructed").await;
        Ok((plan, self.0.report.clone()))
    }
    fn sign(&self, plan: RecordPlan) -> Result<SignedSubmission> {
        self.0.event("sign");
        ensure!(
            self.0.fault != Fault::Sign,
            "synthetic record signer unavailable"
        );
        let mut bytes = b"RECORD-NOT-A-TRANSACTION:".to_vec();
        bytes.extend(serde_json::to_vec(&plan)?);
        Ok(SignedSubmission {
            signature: Self::SIGNATURE.into(),
            bytes,
        })
    }
    async fn broadcast(&self, signed: &SignedSubmission) -> Result<String> {
        self.0.assert_persisted(signed).await?;
        self.0.at("broadcast").await;
        match self.0.fault {
            Fault::Send => {
                anyhow::bail!("synthetic record send failed https://record.invalid/private-token")
            }
            Fault::Receipt => Ok("different-record-receipt".into()),
            _ => Ok(Self::SIGNATURE.into()),
        }
    }
}

impl OrdinaryQuoteBackend for RecordBackend {
    fn prepared_message(&self, plan: &RecordPlan) -> Result<Vec<u8>> {
        self.0.event("inspect_unsigned");
        Ok(serde_json::to_vec(plan)?)
    }
    fn signed_message(&self, signed: &SignedSubmission) -> Result<Vec<u8>> {
        self.0.event("inspect_signed");
        let bytes = signed
            .bytes
            .strip_prefix(b"RECORD-NOT-A-TRANSACTION:")
            .ok_or_else(|| anyhow::anyhow!("record receipt envelope changed"))?;
        let _: RecordPlan = serde_json::from_slice(bytes)?;
        Ok(if self.0.fault == Fault::SignedMessage {
            vec![0]
        } else {
            bytes.to_vec()
        })
    }
}

impl SignatureStatusReader for RecordBackend {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        self.0.event("lookup_signature");
        ensure!(
            signature == Self::SIGNATURE,
            "record lookup must retain original signature"
        );
        Ok(None)
    }
}
