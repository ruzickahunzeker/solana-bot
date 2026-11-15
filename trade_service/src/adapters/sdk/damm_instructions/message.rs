use anyhow::{ensure, Context, Result};
use sol_trade_sdk::{
    common::TradeTransactionVersion,
    constants::swqos::*,
    swqos::SwqosType,
    trading::{
        common::extend_compute_budget_instructions, core::transaction_pool::compile_message,
    },
};
use solana_sdk::{
    instruction::Instruction,
    message::{v1, Message, VersionedMessage},
    pubkey::Pubkey,
    signature::Signature,
    transaction::VersionedTransaction,
};

pub(in crate::adapters::sdk) fn tip_lamports(tip_sol: f64) -> u64 {
    (tip_sol * 1_000_000_000.0).min(u64::MAX as f64).round() as u64
}

pub(in crate::adapters::sdk) fn full_instructions(
    wallet: Pubkey,
    provider: SwqosType,
    simulate: bool,
    tip_sol: f64,
    destination: Option<Pubkey>,
    protocol: &[Instruction],
) -> Result<Vec<Instruction>> {
    let mut instructions = Vec::with_capacity(protocol.len() + 3);
    let wants_tip = !simulate && provider != SwqosType::Default && tip_sol > 0.0;
    if wants_tip {
        let destination = destination.context("DAMM final relay tip is missing")?;
        ensure!(
            tip_accounts(provider).contains(&destination),
            "DAMM final relay tip does not belong to the selected provider"
        );
        instructions.push(solana_system_interface::instruction::transfer(
            &wallet,
            &destination,
            tip_lamports(tip_sol),
        ));
    } else {
        ensure!(
            destination.is_none(),
            "DAMM candidate has an unexpected relay tip"
        );
    }
    extend_compute_budget_instructions(&mut instructions, 1_000, 300_000);
    instructions.extend_from_slice(protocol);
    Ok(instructions)
}

pub(in crate::adapters::sdk) fn compile_candidate(
    wallet: Pubkey,
    instructions: &[Instruction],
    hash: solana_sdk::hash::Hash,
) -> Result<VersionedTransaction> {
    let message = compile_message(
        &wallet,
        instructions,
        &[],
        hash,
        TradeTransactionVersion::V0,
        v1::TransactionConfig::empty(),
    )?;
    ensure!(
        message.header().num_required_signatures == 1
            && message.static_account_keys().first() == Some(&wallet),
        "DAMM final message requires an unexpected signer or payer"
    );
    validate_compiled_message(wallet, instructions, hash, &message)?;
    let transaction = VersionedTransaction {
        signatures: vec![Signature::default()],
        message,
    };
    ensure!(
        bincode::serialized_size(&transaction)? <= 1232,
        "DAMM final transaction exceeds the V0 wire limit"
    );
    Ok(transaction)
}

pub(in crate::adapters::sdk) fn validate_compiled_message(
    wallet: Pubkey,
    instructions: &[Instruction],
    hash: solana_sdk::hash::Hash,
    actual: &VersionedMessage,
) -> Result<()> {
    // The pinned SDK emits Legacy for V0 without ALT. Changes require a reviewed profile migration.
    let expected = VersionedMessage::Legacy(Message::new_with_blockhash(
        instructions,
        Some(&wallet),
        &hash,
    ));
    ensure!(
        *actual == expected,
        "DAMM compiled message differs from the canonical frozen instructions"
    );
    Ok(())
}

pub(in crate::adapters::sdk) fn tip_accounts(provider: SwqosType) -> &'static [Pubkey] {
    match provider {
        SwqosType::Jito => JITO_TIP_ACCOUNTS,
        SwqosType::NextBlock => NEXTBLOCK_TIP_ACCOUNTS,
        SwqosType::ZeroSlot => ZEROSLOT_TIP_ACCOUNTS,
        SwqosType::Temporal => NOZOMI_TIP_ACCOUNTS,
        SwqosType::Bloxroute => BLOX_TIP_ACCOUNTS,
        SwqosType::Node1 => NODE1_TIP_ACCOUNTS,
        SwqosType::FlashBlock => FLASHBLOCK_TIP_ACCOUNTS,
        SwqosType::BlockRazor => BLOCKRAZOR_TIP_ACCOUNTS,
        SwqosType::Astralane => ASTRALANE_TIP_ACCOUNTS,
        SwqosType::Stellium => STELLIUM_TIP_ACCOUNTS,
        SwqosType::Lightspeed => LIGHTSPEED_TIP_ACCOUNTS,
        SwqosType::Soyas => SOYAS_TIP_ACCOUNTS,
        SwqosType::Speedlanding => SPEEDLANDING_TIP_ACCOUNTS,
        SwqosType::Helius => HELIUS_TIP_ACCOUNTS,
        SwqosType::Solami => SOLAMI_TIP_ACCOUNTS,
        SwqosType::LunarLander => LUNARLANDER_TIP_ACCOUNTS,
        SwqosType::Glaive => GLAIVE_TIP_ACCOUNTS,
        SwqosType::Default => &[],
    }
}
