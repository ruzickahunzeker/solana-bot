//! Independent test-only recipe, signer and memory sender; no shared build engine.
use super::Control;
use crate::{
    adapters::sdk::damm_sell_message::validate_signed_candidate,
    application::{
        ordinary_sdk_submission::SdkSigningPermit,
        ports::ordinary_sdk_submission::OrdinarySdkCandidateBackend,
        sdk_preparation::validate_origin, trade_admission::AdmittedTrade,
    },
    domain::{
        sdk_candidate::{DammCandidateDetails, DammCandidatePool, SdkCandidateMetadata},
        sdk_prepared_record::SdkPreparedRecord,
        submission::SignedSubmission,
        trade::{ExecutionMethod, Pool, TradeAmount, WSOL_MINT},
    },
};
use anyhow::{ensure, Result};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{Message, VersionedMessage},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use sqlx::{Row, SqlitePool};
use std::{
    str::FromStr,
    sync::{atomic::Ordering, Arc},
};

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Price,
    Limit,
    EnsureOutput,
    Swap,
    CloseInput,
}

pub(super) struct TapeCandidate {
    source: DammCandidateDetails,
    steps: Box<[Step]>,
    record: SdkPreparedRecord,
}

impl TapeCandidate {
    pub(super) fn build(execution: &AdmittedTrade) -> Result<Self> {
        let command = execution.command();
        let wallet = Pubkey::from_str(execution.reservation().wallet_pubkey())?;
        let quote = Pubkey::from_str(WSOL_MINT)?;
        let key = |value| Pubkey::new_from_array([value; 32]);
        ensure!(
            command.route.pool == Pool::MeteoraDammV2
                && command.execution.method == ExecutionMethod::Sdk
                && !command.aggregator_sol
                && command.route.sol_hop_pool.is_none()
                && command.route.sol_hop_pool_id.is_none()
                && command.route.mint.to_bytes() == key(2).to_bytes()
                && command.route.quote_mint.to_bytes() == quote.to_bytes()
                && command.route.pool_id.map(|v| v.to_bytes()) == Some(key(3).to_bytes())
                && matches!(command.amount, TradeAmount::SellPercent { percent: 100 })
                && !command.execution.mode.simulate()
                && command.execution.use_mev == Some(false)
                && command.execution.mev_provider.is_none(),
            "independent fixture backend supports only its explicit default sell scenario"
        );
        let program = spl_token_interface::ID;
        let input = spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&wallet, &key(2), &program);
        let source = DammCandidateDetails {
            wallet: wallet.to_bytes(),
            mint: key(2).to_bytes(),
            quote_mint: quote.to_bytes(),
            pool_hint: Some(key(3).to_bytes()),
            pool: DammCandidatePool {
                pool: key(3).to_bytes(),
                token_a_vault: key(4).to_bytes(),
                token_b_vault: key(5).to_bytes(),
                token_a_mint: key(2).to_bytes(),
                token_b_mint: quote.to_bytes(),
                token_a_program: program.to_bytes(),
                token_b_program: program.to_bytes(),
                referral: None,
                rate_limiter_sysvar: false,
            },
            balance_units: 10_000,
            balance_slot: 123,
            input_account: input.to_bytes(),
            input_program: program.to_bytes(),
            input_decimals: 6,
            input_units: 10_000,
            sell_percent: 100,
            minimum_output_units: 777,
            slippage_bps: command.execution.slippage_bps,
            tip_sol_bits: command.execution.tip_sol.to_bits(),
            relay: "default".into(),
            tip_destination: None,
            recent_blockhash: [9; 32],
            simulate: false,
            use_mev: Some(false),
            requested_mev_provider: None,
            wait_tx_confirmed: false,
            mev_submit_verify_ms: 250,
        };
        let steps = vec![
            Step::Price,
            Step::Limit,
            Step::EnsureOutput,
            Step::Swap,
            Step::CloseInput,
        ]
        .into_boxed_slice();
        let message = compile(&source, &steps)?;
        let record = SdkCandidateMetadata::damm_sell(source.clone(), &message)?.into();
        Ok(Self {
            source,
            steps,
            record,
        })
    }

    pub(super) fn record(&self) -> &SdkPreparedRecord {
        &self.record
    }

    pub(super) fn message(&self) -> Result<Vec<u8>> {
        ensure!(
            self.record.sell()?.details() == &self.source,
            "independent recipe metadata changed"
        );
        let message = compile(&self.source, &self.steps)?;
        self.record.validate(&message)?;
        Ok(message)
    }
}

fn compile(source: &DammCandidateDetails, steps: &[Step]) -> Result<Vec<u8>> {
    ensure!(
        steps
            == [
                Step::Price,
                Step::Limit,
                Step::EnsureOutput,
                Step::Swap,
                Step::CloseInput
            ],
        "independent recipe shape changed"
    );
    let key = Pubkey::new_from_array;
    let wallet = key(source.wallet);
    let quote = key(source.quote_mint);
    let input = key(source.input_account);
    let program = spl_token_interface::ID;
    let output = spl_associated_token_account_interface::address::get_associated_token_address_with_program_id(&wallet, &quote, &program);
    let dex = Pubkey::from_str("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG")?;
    let authority = Pubkey::from_str("HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC")?;
    let event = Pubkey::find_program_address(&[b"__event_authority"], &dex).0;
    let budget = Pubkey::from_str("ComputeBudget111111111111111111111111111111")?;
    let p = &source.pool;
    let mut instructions = Vec::new();
    for step in steps {
        instructions.push(match step {
            Step::Price => Instruction::new_with_bytes(budget, &[vec![3], 1_000u64.to_le_bytes().to_vec()].concat(), vec![]),
            Step::Limit => Instruction::new_with_bytes(budget, &[vec![2], 300_000u32.to_le_bytes().to_vec()].concat(), vec![]),
            Step::EnsureOutput => spl_associated_token_account_interface::instruction::create_associated_token_account_idempotent(&wallet, &wallet, &quote, &program),
            Step::Swap => {
                let mut data = vec![65,75,63,76,235,91,91,136];
                data.extend_from_slice(&source.input_units.to_le_bytes());
                data.extend_from_slice(&source.minimum_output_units.to_le_bytes());
                data.push(0);
                Instruction::new_with_bytes(dex, &data, vec![
                    AccountMeta::new_readonly(authority,false), AccountMeta::new(key(p.pool),false),
                    AccountMeta::new(input,false), AccountMeta::new(output,false),
                    AccountMeta::new(key(p.token_a_vault),false), AccountMeta::new(key(p.token_b_vault),false),
                    AccountMeta::new_readonly(key(p.token_a_mint),false), AccountMeta::new_readonly(key(p.token_b_mint),false),
                    AccountMeta::new_readonly(wallet,true), AccountMeta::new_readonly(key(p.token_a_program),false),
                    AccountMeta::new_readonly(key(p.token_b_program),false), AccountMeta::new_readonly(dex,false),
                    AccountMeta::new_readonly(event,false), AccountMeta::new_readonly(dex,false),
                ])
            }
            Step::CloseInput => spl_token_interface::instruction::close_account(&program, &input, &wallet, &wallet, &[&wallet])?,
        });
    }
    Ok(VersionedMessage::Legacy(Message::new_with_blockhash(
        &instructions,
        Some(&wallet),
        &Hash::new_from_array(source.recent_blockhash),
    ))
    .serialize())
}

pub(super) struct TapeBackend {
    pub(super) signer: Arc<Keypair>,
    pub(super) control: Arc<Control>,
    pub(super) pool: SqlitePool,
    pub(super) request_id: String,
}

pub(super) struct TapeSigned {
    candidate: TapeCandidate,
    submission: SignedSubmission,
}

impl OrdinarySdkCandidateBackend for TapeBackend {
    type Candidate = TapeCandidate;
    type Signed = TapeSigned;
    fn ensure_live_allowed(&self) -> Result<()> {
        let count = self.control.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.control.event(&format!("live-{count}"))
    }
    fn metadata<'a>(&self, candidate: &'a TapeCandidate) -> &'a SdkPreparedRecord {
        candidate.record()
    }
    fn unsigned_message(&self, candidate: &TapeCandidate) -> Result<Vec<u8>> {
        self.control.event("unsigned")?;
        candidate.message()
    }
    fn authorize(&self, execution: &AdmittedTrade, candidate: &TapeCandidate) -> Result<()> {
        let count = self.control.authorize.fetch_add(1, Ordering::SeqCst) + 1;
        self.control.event(&format!("authorize-{count}"))?;
        validate_origin(execution, candidate.record(), &candidate.message()?)
    }
    fn sign(
        &self,
        mut candidate: TapeCandidate,
        permit: SdkSigningPermit<'_>,
    ) -> Result<TapeSigned> {
        if self.control.changed("tape-recipe") {
            candidate.source.input_units += 1;
        }
        if self.control.changed("tape-permit") {
            candidate.source.balance_units += 1;
            candidate.source.input_units += 1;
            let changed = compile(&candidate.source, &candidate.steps)?;
            candidate.record =
                SdkCandidateMetadata::damm_sell(candidate.source.clone(), &changed)?.into();
        }
        let message = candidate.message()?;
        permit.validate(candidate.record(), &message)?;
        self.control.signs.fetch_add(1, Ordering::SeqCst);
        self.control.event("sign")?;
        self.control.event("acquire-signer")?;
        self.control.keys.fetch_add(1, Ordering::SeqCst);
        ensure!(
            self.signer.pubkey().to_bytes() == candidate.source.wallet,
            "independent signer differs from original wallet"
        );
        let signature = self.signer.sign_message(&message);
        let transaction = VersionedTransaction {
            signatures: vec![signature],
            message: bincode::deserialize(&message)?,
        };
        let mut bytes = bincode::serialize(&transaction)?;
        if self.control.changed("tape-signed") {
            bytes[0] ^= 1;
        }
        Ok(TapeSigned {
            candidate,
            submission: SignedSubmission {
                signature: signature.to_string(),
                bytes,
            },
        })
    }
    fn signed_submission<'a>(&self, signed: &'a TapeSigned) -> &'a SignedSubmission {
        &signed.submission
    }
    fn verify_signed(
        &self,
        signed: &TapeSigned,
        metadata: &SdkPreparedRecord,
        message: &[u8],
    ) -> Result<()> {
        self.control.event("verify-signed")?;
        validate_signed_candidate(metadata, message, &signed.submission)
    }
    fn authorize_broadcast(&self, signed: &TapeSigned) -> Result<()> {
        let count = self
            .control
            .broadcast_authorize
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        self.control
            .event(&format!("authorize-broadcast-{count}"))?;
        validate_signed_candidate(
            signed.candidate.record(),
            &signed.candidate.message()?,
            &signed.submission,
        )
    }
    async fn broadcast(&self, artifact: &TapeSigned, signed: &SignedSubmission) -> Result<String> {
        ensure!(
            artifact.submission.signature == signed.signature
                && artifact.submission.bytes == signed.bytes,
            "independent sender changed original bytes"
        );
        self.control.sends.fetch_add(1, Ordering::SeqCst);
        self.control.gate("broadcast").await?;
        let row = sqlx::query("SELECT submission_stage,signed_signature,signed_transaction FROM ordinary_execution_claims WHERE request_id=?")
            .bind(&self.request_id).fetch_one(&self.pool).await?;
        ensure!(
            row.try_get::<&str, _>("submission_stage")? == "broadcasting"
                && row.try_get::<&str, _>("signed_signature")? == signed.signature
                && row.try_get::<Vec<u8>, _>("signed_transaction")? == signed.bytes,
            "independent sender did not receive original persisted bytes"
        );
        match self.control.response.load(Ordering::SeqCst) {
            0 => Ok(signed.signature.clone()),
            1 => anyhow::bail!(
                "independent fixture sender unavailable at https://rpc.invalid/private-token"
            ),
            2 => Ok("different-signature".into()),
            _ => anyhow::bail!("unsupported independent fixture response"),
        }
    }
}
