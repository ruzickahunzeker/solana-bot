use super::*;
use crate::adapters::{
    rpc::damm_sell_simulation::simulate_unsigned_at_slot,
    sdk::{
        damm_instructions::{compile_candidate, full_instructions, swap_instruction, tip_lamports},
        ordinary_lanes::{SdkInvocationFailure, SdkLaneReport},
    },
};
use sol_trade_sdk::{
    common::fast_fn::create_associated_token_account_idempotent_fast, trading::MiddlewareManager,
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{instruction::Instruction, transaction::VersionedTransaction};
use std::{future::Future, sync::Arc};

mod guard;
mod metadata;
mod signed;
use crate::domain::sdk_prepared_record::SdkPreparedRecord;
pub(crate) use metadata::validate_record as validate_buy_record;
pub(crate) use signed::DammBuySignedCandidate;

/// Frozen buy construction. No signing authority, persisted owner or broadcast capability.
pub(crate) struct DammBuyMessage {
    quote: Arc<DammBuyQuote>,
    request: TradeBuyParams,
    protocol: Vec<Instruction>,
}

pub(crate) struct DammBuyCandidate {
    profile: Arc<DammBuyMessage>,
    instructions: Vec<Instruction>,
    transaction: VersionedTransaction,
    record: SdkPreparedRecord,
}

impl DammBuyMessage {
    pub(crate) fn new(
        quote: Arc<DammBuyQuote>,
        context: DammBuyContext,
        intent: &ExecuteTrade,
        request: &TradeBuyParams,
    ) -> Result<Arc<Self>> {
        quote.validate(context, intent, request)?;
        let protocol = protocol(&quote, request);
        let profile = Arc::new(Self {
            quote,
            request: request.clone(),
            protocol,
        });
        profile.recheck()?;
        Ok(profile)
    }

    pub(crate) fn validate(
        &self,
        context: DammBuyContext,
        intent: &ExecuteTrade,
        request: &TradeBuyParams,
    ) -> Result<()> {
        self.quote.validate(context, intent, request)?;
        ensure!(
            self.protocol == protocol(&self.quote, request),
            "DAMM buy protocol instructions differ from the frozen message"
        );
        self.recheck()
    }

    pub(crate) fn validate_request(
        &self,
        context: DammBuyContext,
        request: &TradeBuyParams,
    ) -> Result<()> {
        self.validate(context, &self.quote.intent, request)
    }

    fn recheck(&self) -> Result<()> {
        self.recheck_inner().map_err(|error| {
            reject_balance("DAMM buy final message rejected").context(error.to_string())
        })
    }

    fn recheck_inner(&self) -> Result<()> {
        self.quote
            .recheck_request(self.quote.context, &self.request)?;
        let tip =
            if !self.quote.intent.simulate && self.quote.context.provider != SwqosType::Default {
                tip_lamports(self.quote.intent.tip_sol)
            } else {
                0
            };
        // The discovery packet already paid its same-bank base fee, input and actual rent.
        // The fixed envelope adds ceil(300_000 * 1_000 / 1_000_000) and the selected relay tip.
        let additional = tip
            .checked_add(300)
            .ok_or_else(|| reject_balance("DAMM buy final native fee budget overflow"))?;
        self.quote
            .evidence
            .check_additional_native_budget(&self.quote.plan, additional)
    }

    fn prepare(self: &Arc<Self>, destination: Option<Pubkey>) -> Result<DammBuyCandidate> {
        self.recheck()?;
        let quote = &self.quote;
        let instructions = full_instructions(
            quote.context.wallet,
            quote.context.provider,
            quote.intent.simulate,
            quote.intent.tip_sol,
            destination,
            &self.protocol,
        )?;
        let transaction = compile_candidate(
            quote.context.wallet,
            &instructions,
            quote.intent.recent_blockhash,
        )?;
        let record = metadata::capture(self, destination, &transaction.message.serialize())?;
        metadata::validate_record(&record, &transaction.message.serialize())?;
        Ok(DammBuyCandidate {
            profile: self.clone(),
            instructions,
            transaction,
            record: SdkPreparedRecord::DammBuy(record),
        })
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
        self.recheck().map_err(failure)?;
        if self.quote.intent.simulate {
            let candidate = self.prepare(None).map_err(failure)?;
            let result = simulate_unsigned_at_slot(
                rpc,
                &candidate.transaction,
                self.quote.evidence.context().1,
            )
            .await
            .map_err(failure)?;
            self.recheck().map_err(failure)?;
            return Ok(SdkLaneReport {
                success: result.error.is_none(),
                signatures: vec![],
                error: result.error,
            });
        }
        let invocation = guard::Invocation::new(self.clone());
        let result = invoke(invocation.middleware()).await;
        invocation.finish(result)
    }
}

impl DammBuyCandidate {
    #[cfg(test)]
    pub(crate) fn expire_for_test(&mut self) {
        Arc::get_mut(&mut Arc::get_mut(&mut self.profile).unwrap().quote)
            .unwrap()
            .evidence
            .expire_for_tests();
    }
    pub(crate) fn metadata(&self) -> &SdkPreparedRecord {
        &self.record
    }

    pub(crate) fn message_bytes(&self) -> Vec<u8> {
        self.transaction.message.serialize()
    }

    pub(crate) async fn validate_unsigned_live(&self, rpc: &RpcClient) -> Result<()> {
        ensure!(
            !self.profile.quote.intent.simulate,
            "DAMM buy live preview requires its original live profile"
        );
        self.validate_full(&self.instructions)?;
        let result = simulate_unsigned_at_slot(
            rpc,
            &self.transaction,
            self.profile.quote.evidence.context().1,
        )
        .await?;
        self.validate_full(&self.instructions)?;
        ensure!(
            result.error.is_none(),
            "DAMM v2 final buy transaction simulation failed: {}",
            result.error.unwrap_or_default()
        );
        Ok(())
    }

    fn validate_full(&self, actual: &[Instruction]) -> Result<()> {
        self.profile.recheck()?;
        ensure!(
            actual == self.instructions,
            "DAMM buy final instructions, permissions or fee budget changed"
        );
        ensure!(
            compile_candidate(
                self.profile.quote.context.wallet,
                actual,
                self.profile.quote.intent.recent_blockhash
            )?
            .message
                == self.transaction.message,
            "DAMM buy compiled message changed"
        );
        let message = self.transaction.message.serialize();
        metadata::validate_record(self.record.buy()?, &message)?;
        ensure!(
            metadata::capture(
                &self.profile,
                self.record
                    .buy()?
                    .details()
                    .tip_destination
                    .map(Pubkey::new_from_array),
                &message
            )? == *self.record.buy()?,
            "DAMM buy record differs from its original live evidence"
        );
        Ok(())
    }
}

impl DammBuyMessage {
    pub(crate) fn prepare_live(self: &Arc<Self>, tip: Option<Pubkey>) -> Result<DammBuyCandidate> {
        ensure!(
            !self.quote.intent.simulate,
            "DAMM buy live candidate requires its original live profile"
        );
        self.prepare(tip)
    }
}

fn protocol(quote: &DammBuyQuote, request: &TradeBuyParams) -> Vec<Instruction> {
    protocol_for(&quote.plan, quote.minimum, request.create_mint_ata)
}

fn protocol_for(
    plan: &PreparedDammBuyProbe,
    minimum: u64,
    create_output: bool,
) -> Vec<Instruction> {
    let a = &plan.accounts;
    let mut instructions = match plan.funding {
        Funding::NativeSol => sol_trade_sdk::trading::common::handle_wsol(&a.wallet, a.amount)
            .into_iter()
            .collect(),
        Funding::ExistingUsdc => create_associated_token_account_idempotent_fast(
            &a.wallet,
            &a.wallet,
            &a.input_mint,
            &spl_token_interface::ID,
        ),
    };
    if create_output {
        let program = if a.output_mint == a.pool.token_a_mint {
            a.pool.token_a_program
        } else {
            a.pool.token_b_program
        };
        instructions.extend(create_associated_token_account_idempotent_fast(
            &a.wallet,
            &a.wallet,
            &a.output_mint,
            &program,
        ));
    }
    instructions.push(swap_instruction(
        a.wallet,
        &a.pool,
        a.input_ata,
        a.output_ata,
        a.amount,
        minimum,
    ));
    instructions
}

fn failure(error: anyhow::Error) -> SdkInvocationFailure {
    SdkInvocationFailure {
        signatures: vec![],
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) async fn buy_record_fixture() -> (
    crate::domain::sdk_buy_candidate::DammBuyCandidateRecord,
    Vec<u8>,
) {
    let profile = tests::profile(SwqosType::Default, false, true, false).await;
    let candidate = profile.prepare(None).unwrap();
    let message = candidate.transaction.message.serialize();
    (
        crate::domain::sdk_buy_candidate::DammBuyCandidateRecord::decode(
            &candidate.record.encode(&message).unwrap(),
            &message,
        )
        .unwrap(),
        message,
    )
}

#[cfg(test)]
pub(crate) async fn buy_record_fixture_for_wallet(
    wallet: Pubkey,
    native: bool,
) -> (
    crate::domain::sdk_buy_candidate::DammBuyCandidateRecord,
    Vec<u8>,
) {
    let profile = tests::evidenced(
        wallet,
        native,
        false,
        false,
        false,
        None,
        false,
        SwqosType::Default,
    )
    .await
    .unwrap();
    let candidate = profile.prepare(None).unwrap();
    let message = candidate.transaction.message.serialize();
    (
        crate::domain::sdk_buy_candidate::DammBuyCandidateRecord::decode(
            &candidate.record.encode(&message).unwrap(),
            &message,
        )
        .unwrap(),
        message,
    )
}

#[cfg(test)]
pub(crate) async fn buy_candidate_fixture(
    wallet: Pubkey,
    native: bool,
    provider: SwqosType,
) -> (DammBuyCandidate, DammBuyContext) {
    let profile = tests::evidenced(wallet, native, false, false, false, None, false, provider)
        .await
        .unwrap();
    let context = profile.quote.context;
    let tip = (provider != SwqosType::Default)
        .then(|| crate::adapters::sdk::damm_instructions::tip_accounts(provider)[0]);
    (profile.prepare_live(tip).unwrap(), context)
}
