use super::*;
use serde::{Deserialize, Serialize};

pub(super) struct TapeBackend(Arc<Probe>);

#[derive(Serialize, Deserialize)]
pub(super) enum Step {
    Owner(String, String),
    Spend(String, u64),
    Swap(String, String, String),
    Receive(String, u64, u64),
    Validity(u16, u64),
}

pub(super) struct TapePlan(Box<[Step]>);

impl FakeConstruction for TapeBackend {
    const SIGNATURE: &'static str = "non-executable-tape-receipt";
    fn new(probe: Arc<Probe>) -> Self {
        Self(probe)
    }
    fn inspect(plan: &TapePlan) -> Result<IntentView> {
        let steps: Vec<Step> = bincode::deserialize(&bincode::serialize(&plan.0)?)?;
        let [Step::Owner(id, payer), Step::Spend(input, amount), Step::Swap(label, from, to), Step::Receive(output, expected, minimum), Step::Validity(slippage, expires)] =
            steps.as_slice()
        else {
            anyhow::bail!("tape operation shape mismatch")
        };
        Ok(IntentView {
            wallet_id: id.clone(),
            payer: payer.clone(),
            input: input.clone(),
            output: output.clone(),
            amount: *amount,
            expected: *expected,
            minimum: *minimum,
            slippage: *slippage,
            expires: *expires,
            legs: vec![(label.clone(), from.clone(), to.clone())],
        })
    }
}

impl QuoteSubmissionBackend for TapeBackend {
    type Prepared = TapePlan;
    fn ensure_live_allowed(&self) -> Result<()> {
        self.0.event("policy");
        Ok(())
    }
    async fn prepare(&self, bound: &BoundQuote) -> Result<(TapePlan, QuotePreflight)> {
        self.0.event("prepare");
        let [leg] = bound.quote.route.as_slice() else {
            anyhow::bail!("tape requires one direct pool")
        };
        ensure!(leg.label == "fake-amm", "tape protocol is unsupported");
        ensure!(
            bound.quote.amount_lamports != 0 && bound.quote.input_mint == WSOL_MINT,
            "tape cannot spend this asset or amount"
        );
        ensure!(
            bound.quote.minimum_output_units != 0
                && bound.quote.expected_output_units >= bound.quote.minimum_output_units,
            "tape output constraint invalid"
        );
        let steps = vec![
            Step::Owner(bound.wallet_id.clone(), bound.wallet_pubkey.clone()),
            Step::Spend(bound.quote.input_mint.clone(), bound.quote.amount_lamports),
            Step::Swap(
                leg.label.clone(),
                leg.input_mint.clone(),
                leg.output_mint.clone(),
            ),
            Step::Receive(
                bound.quote.output_mint.clone(),
                bound.quote.expected_output_units,
                bound.quote.minimum_output_units,
            ),
            Step::Validity(bound.quote_slippage_bps, bound.expires_at_ms),
        ];
        self.0.at("constructed").await;
        Ok((TapePlan(steps.into_boxed_slice()), self.0.report.clone()))
    }
    fn sign(&self, plan: TapePlan) -> Result<SignedSubmission> {
        self.0.event("sign");
        ensure!(
            self.0.fault != Fault::Sign,
            "synthetic tape signer unavailable"
        );
        let payload = bincode::serialize(&plan.0)?;
        let mut bytes = vec![0xff, 0x54, 0x41, 0x50, 0x45];
        bytes.extend(payload);
        Ok(SignedSubmission {
            signature: Self::SIGNATURE.into(),
            bytes,
        })
    }
    async fn broadcast(&self, signed: &SignedSubmission) -> Result<String> {
        self.0.assert_persisted(signed).await?;
        self.0.at("broadcast").await;
        if self.0.fault == Fault::Send {
            anyhow::bail!("synthetic tape transport failed https://tape.invalid/private-token");
        }
        Ok(if self.0.fault == Fault::Receipt {
            "different-tape-receipt".into()
        } else {
            Self::SIGNATURE.into()
        })
    }
}

impl OrdinaryQuoteBackend for TapeBackend {
    fn prepared_message(&self, plan: &TapePlan) -> Result<Vec<u8>> {
        self.0.event("inspect_unsigned");
        Ok(bincode::serialize(&plan.0)?)
    }
    fn signed_message(&self, signed: &SignedSubmission) -> Result<Vec<u8>> {
        self.0.event("inspect_signed");
        ensure!(
            signed.bytes.starts_with(&[0xff, 0x54, 0x41, 0x50, 0x45]),
            "tape receipt tag invalid"
        );
        let payload = &signed.bytes[5..];
        let _: Vec<Step> = bincode::deserialize(payload)?;
        let mut result = payload.to_vec();
        if self.0.fault == Fault::SignedMessage {
            result.push(0);
        }
        Ok(result)
    }
}

impl SignatureStatusReader for TapeBackend {
    async fn lookup_signature(&self, signature: &str) -> Result<Option<ChainObservation>> {
        self.0.event("lookup_signature");
        ensure!(
            signature == Self::SIGNATURE,
            "tape status request switched signature"
        );
        Ok(None)
    }
}
