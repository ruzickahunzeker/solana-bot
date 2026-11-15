use super::{Funding, PreparedDammBuyProbe};
use crate::adapters::sdk::{
    damm_probe::{
        evidence::{check_snapshot_for_accounts, AccountSnapshot, InputRequirement},
        output::{amount_row, cross_check, net_amount, trace, BalanceCredits, NetAmounts},
        plan::ProbeView,
        simulate_packet,
    },
    pumpapi_assets::decode_data,
};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use solana_account_decoder::{UiAccount, UiAccountEncoding};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcAccountInfoConfig};
use solana_commitment_config::CommitmentConfig;
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

struct BuySnapshot {
    assets: AccountSnapshot,
    wallet_lamports: u64,
}

/// Discovery evidence only; neither serializable nor accepted by the sell signing profile.
pub(crate) struct BuyOutputEvidence {
    message_hash: [u8; 32],
    output_units: u64,
    account_slot: u64,
    simulation_slot: u64,
    probe_fee_lamports: u64,
    output_initialized: bool,
    wallet_remaining_lamports: u64,
    started: Instant,
}

impl BuyOutputEvidence {
    pub(super) fn minimum(&self, plan: &PreparedDammBuyProbe, slippage_bps: u64) -> Result<u64> {
        ensure!(
            self.started.elapsed() <= crate::domain::preparation_balance::MAX_BALANCE_AGE,
            "DAMM buy output evidence expired"
        );
        ensure!(
            self.message_hash == fingerprint(plan)?,
            "DAMM buy output evidence differs from its discovery message"
        );
        crate::protocol::meteora_damm_v2::minimum_output(self.output_units, slippage_bps)
    }
    pub(super) fn output_units(&self) -> u64 {
        self.output_units
    }
    pub(super) fn context(&self) -> (u64, u64, u64) {
        (
            self.account_slot,
            self.simulation_slot,
            self.probe_fee_lamports,
        )
    }
    pub(super) fn output_initialized(&self) -> bool {
        self.output_initialized
    }
    pub(super) fn check_additional_native_budget(
        &self,
        plan: &PreparedDammBuyProbe,
        lamports: u64,
    ) -> Result<()> {
        self.minimum(plan, 0)?;
        ensure!(
            self.wallet_remaining_lamports >= lamports,
            "DAMM buy native balance cannot cover final priority fee and relay tip"
        );
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn expire_for_tests(&mut self) {
        self.started = Instant::now() - Duration::from_secs(4);
    }
}

fn fingerprint(plan: &PreparedDammBuyProbe) -> Result<[u8; 32]> {
    Ok(Sha256::digest(bincode::serialize(&plan.transaction.message)?).into())
}

fn view(plan: &PreparedDammBuyProbe) -> ProbeView<'_> {
    ProbeView {
        message: &plan.transaction.message,
        accounts: &plan.accounts,
    }
}

fn layout(plan: &PreparedDammBuyProbe) -> trace::Layout {
    let output_setup = if plan.funding == Funding::NativeSol {
        3
    } else {
        1
    };
    trace::Layout {
        input_setup: Some(0),
        output_setup,
        swap: output_setup + 1,
    }
}

fn keys(plan: &PreparedDammBuyProbe) -> [Pubkey; 9] {
    let a = &plan.accounts;
    [
        a.pool.pool,
        a.pool.token_a_mint,
        a.pool.token_b_mint,
        a.pool.token_a_vault,
        a.pool.token_b_vault,
        solana_sdk::sysvar::clock::id(),
        a.input_ata,
        a.output_ata,
        a.wallet,
    ]
}

struct NativeState {
    reserve: u64,
    available: u64,
    accounted: u64,
}

fn native_state(account: &UiAccount) -> Result<NativeState> {
    let data = decode_data(
        account,
        spl_token_interface::ID,
        165,
        "DAMM buy native account",
    )?;
    let token = spl_token_interface::state::Account::unpack(&data)?;
    let reserve = Option::<u64>::from(token.is_native)
        .context("DAMM buy WSOL account has no native reserve")?;
    let available = account
        .lamports
        .checked_sub(reserve)
        .context("DAMM buy WSOL lamports are below its native reserve")?;
    ensure!(
        reserve > 0 && token.amount <= available,
        "DAMM buy WSOL amount exceeds available native lamports"
    );
    Ok(NativeState {
        reserve,
        available,
        accounted: token.amount,
    })
}

fn snapshot(
    plan: &PreparedDammBuyProbe,
    slot: u64,
    values: &[Option<UiAccount>],
    post: bool,
) -> Result<BuySnapshot> {
    ensure!(values.len() == 9, "DAMM buy account count is invalid");
    let input = if plan.funding == Funding::NativeSol {
        InputRequirement::NativeDeposit
    } else {
        InputRequirement::ExistingBuy
    };
    let assets = check_snapshot_for_accounts(&plan.accounts, slot, &values[..8], post, input)?;
    let wallet = values[8]
        .as_ref()
        .context("DAMM buy wallet account is missing")?;
    ensure!(
        decode_data(
            wallet,
            solana_system_interface::program::id(),
            0,
            "DAMM buy wallet"
        )?
        .is_empty(),
        "DAMM buy wallet data is invalid"
    );
    if !post && plan.funding == Funding::NativeSol {
        ensure!(
            wallet.lamports >= plan.accounts.amount,
            "DAMM buy native balance is insufficient before rent and fees"
        );
    }
    let a = &plan.accounts;
    for (index, mint) in [
        (3, a.pool.token_a_mint),
        (4, a.pool.token_b_mint),
        (6, a.input_mint),
        (7, a.output_mint),
    ] {
        if mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT {
            if let Some(account) = &values[index] {
                // A buy ATA may be a validated empty System account before initialization.
                if !post
                    && index >= 6
                    && account.owner == solana_system_interface::program::id().to_string()
                {
                    continue;
                }
                let native = native_state(account)?;
                if post && index == 6 && plan.funding == Funding::NativeSol {
                    ensure!(
                        native.accounted == native.available,
                        "DAMM buy input did not synchronize its complete native balance"
                    );
                }
            }
        }
    }
    Ok(BuySnapshot {
        assets,
        wallet_lamports: wallet.lamports,
    })
}

fn index(plan: &PreparedDammBuyProbe, key: Pubkey) -> Result<usize> {
    plan.transaction
        .message
        .account_keys
        .iter()
        .position(|p| *p == key)
        .context("DAMM buy funding role is not in its discovery message")
}

struct FundingProof {
    fee: u64,
    credits: BalanceCredits,
}

fn check_native_budget(
    plan: &PreparedDammBuyProbe,
    result: &solana_client::rpc_response::RpcSimulateTransactionResult,
    snapshot: &BuySnapshot,
    movements: &trace::Transfers,
) -> Result<FundingProof> {
    let before = result
        .pre_balances
        .as_ref()
        .context("DAMM buy omitted same-bank pre balances")?;
    let after = result
        .post_balances
        .as_ref()
        .context("DAMM buy omitted same-bank post balances")?;
    let fee = result.fee.context("DAMM buy simulation omitted its fee")?;
    let count = plan.transaction.message.account_keys.len();
    ensure!(
        before.len() == count && after.len() == count,
        "DAMM buy native balance index set is inconsistent"
    );
    let values = result
        .accounts
        .as_ref()
        .context("DAMM buy simulation omitted account data")?;
    let mut rent = 0u64;
    let mut credits = BalanceCredits::default();
    if plan.funding == Funding::NativeSol {
        credits.input = plan.accounts.amount;
    }
    for (position, key, initialized, mint) in [
        (
            6,
            plan.accounts.input_ata,
            movements.input_initialized,
            plan.accounts.input_mint,
        ),
        (
            7,
            plan.accounts.output_ata,
            movements.output_initialized,
            plan.accounts.output_mint,
        ),
    ] {
        let i = index(plan, key)?;
        let account = values[position]
            .as_ref()
            .context("DAMM buy simulated ATA is missing")?;
        ensure!(
            after[i] == account.lamports,
            "DAMM buy post lamports differ from account data"
        );
        if initialized {
            let (reserve, initial_native) = if mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
            {
                let native = native_state(account)?;
                (native.reserve, before[i].saturating_sub(native.reserve))
            } else {
                (account.lamports, 0)
            };
            let charge = reserve.saturating_sub(before[i]);
            let output_credit =
                if position == 7 && mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT {
                    net_amount(&movements.output, &snapshot.assets.output)?
                } else {
                    0
                };
            ensure!(
                before[i].max(reserve).checked_add(output_credit) == Some(after[i]),
                "DAMM buy initialized ATA funding differs from its native budget"
            );
            rent = rent.checked_add(charge).context("DAMM buy rent overflow")?;
            let credit = if position == 6 {
                &mut credits.input
            } else {
                &mut credits.output
            };
            *credit = credit
                .checked_add(initial_native)
                .context("DAMM buy initial native credit overflow")?;
        } else {
            let credit = if position == 7 && mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT {
                net_amount(&movements.output, &snapshot.assets.output)?
            } else {
                0
            };
            ensure!(
                before[i].checked_add(credit) == Some(after[i]),
                "DAMM buy existing ATA lamports changed outside the quote profile"
            );
            if position == 6 && plan.funding == Funding::NativeSol {
                let native = native_state(account)?;
                let previous = amount_row(
                    result
                        .pre_token_balances
                        .as_deref()
                        .context("DAMM buy omitted same-bank pre token balances")?,
                    u8::try_from(i)?,
                    &snapshot.assets.input,
                    plan.accounts.wallet,
                )?
                .context("DAMM buy existing native input has no pre token balance")?;
                let unsynchronized = before[i]
                    .checked_sub(native.reserve)
                    .and_then(|available| available.checked_sub(previous))
                    .context("DAMM buy pre token balance exceeds its native backing")?;
                credits.input = credits
                    .input
                    .checked_add(unsynchronized)
                    .context("DAMM buy synchronized native credit overflow")?;
            }
        }
    }
    ensure!(
        plan.funding == Funding::NativeSol || !movements.input_initialized,
        "DAMM buy cannot fund a newly created USDC input account"
    );
    let deposit = if plan.funding == Funding::NativeSol {
        plan.accounts.amount
    } else {
        0
    };
    let debit = deposit
        .checked_add(rent)
        .and_then(|v| v.checked_add(fee))
        .context("DAMM buy native funding budget overflow")?;
    let wallet = index(plan, plan.accounts.wallet)?;
    ensure!(
        after[wallet] == snapshot.wallet_lamports
            && before[wallet].checked_sub(debit) == Some(after[wallet]),
        "DAMM buy native funding, rent and fee do not match same-bank wallet balances"
    );
    Ok(FundingProof { fee, credits })
}

#[cfg(test)]
async fn quote(
    rpc: &RpcClient,
    plan: &PreparedDammBuyProbe,
    minimum_slot: u64,
) -> Result<BuyOutputEvidence> {
    ensure!(
        minimum_slot > 0,
        "DAMM buy quote requires a nonzero context slot"
    );
    quote_with_context(rpc, plan, Some(minimum_slot)).await
}

pub(crate) async fn quote_current(
    rpc: &RpcClient,
    plan: &PreparedDammBuyProbe,
) -> Result<BuyOutputEvidence> {
    quote_with_context(rpc, plan, None).await
}

async fn quote_with_context(
    rpc: &RpcClient,
    plan: &PreparedDammBuyProbe,
    minimum_slot: Option<u64>,
) -> Result<BuyOutputEvidence> {
    let started = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(5),
        quote_inner(rpc, plan, minimum_slot, started),
    )
    .await
    .context("DAMM buy unsigned quote timed out")?
}

async fn quote_inner(
    rpc: &RpcClient,
    plan: &PreparedDammBuyProbe,
    minimum_slot: Option<u64>,
    started: Instant,
) -> Result<BuyOutputEvidence> {
    let keys = keys(plan);
    ensure!(
        keys.iter().collect::<HashSet<_>>().len() == keys.len(),
        "DAMM buy account roles alias each other"
    );
    let response = rpc
        .get_multiple_ui_accounts_with_config(
            &keys,
            RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                commitment: Some(CommitmentConfig::processed()),
                data_slice: None,
                min_context_slot: minimum_slot,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{}", crate::sanitize::redact_urls(&error.to_string())))?;
    ensure!(
        response.context.slot > 0
            && minimum_slot.is_none_or(|minimum| response.context.slot >= minimum),
        "DAMM buy account context regressed"
    );
    snapshot(plan, response.context.slot, &response.value, false)?;
    let simulated =
        simulate_packet(rpc, &plan.transaction, &keys, response.context.slot, true).await?;
    ensure!(
        simulated.context.slot >= response.context.slot,
        "DAMM buy simulation context regressed"
    );
    let result = &simulated.value;
    ensure!(
        result.err.is_none(),
        "DAMM buy unsigned simulation failed: {:?}",
        result.err
    );
    ensure!(
        result.replacement_blockhash.is_none()
            && result
                .loaded_addresses
                .as_ref()
                .is_none_or(|a| a.writable.is_empty() && a.readonly.is_empty()),
        "DAMM buy simulation changed its fixed message context"
    );
    let post = snapshot(
        plan,
        simulated.context.slot,
        result
            .accounts
            .as_ref()
            .context("DAMM buy simulation omitted account data")?,
        true,
    )?;
    let movements = trace::transfers_for(
        view(plan),
        result
            .inner_instructions
            .as_deref()
            .context("DAMM buy omitted inner transfers")?,
        layout(plan),
    )?;
    ensure!(
        movements.input.amount == plan.accounts.amount,
        "DAMM buy input differs from exact budget"
    );
    let credited_input = net_amount(&movements.input, &post.assets.input)?;
    let output_units = net_amount(&movements.output, &post.assets.output)?;
    ensure!(
        credited_input > 0 && output_units > 0,
        "DAMM buy simulation returned zero net output"
    );
    let funding = check_native_budget(plan, result, &post, &movements)?;
    cross_check(
        result
            .pre_token_balances
            .as_deref()
            .context("DAMM buy omitted same-bank pre token balances")?,
        result
            .post_token_balances
            .as_deref()
            .context("DAMM buy omitted same-bank post token balances")?,
        view(plan),
        &post.assets,
        &movements,
        NetAmounts {
            input: credited_input,
            output: output_units,
        },
        funding.credits,
    )?;
    let evidence = BuyOutputEvidence {
        message_hash: fingerprint(plan)?,
        output_units,
        account_slot: response.context.slot,
        simulation_slot: simulated.context.slot,
        probe_fee_lamports: funding.fee,
        output_initialized: movements.output_initialized,
        wallet_remaining_lamports: post.wallet_lamports,
        started,
    };
    evidence.minimum(plan, 0)?;
    Ok(evidence)
}

#[cfg(test)]
pub(super) mod tests;
