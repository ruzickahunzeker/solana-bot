//! Historical semantic consistency, not current bank evidence or signing authority.
use crate::{
    adapters::{
        rpc::parameter_reads::request::pump_curve::{PumpCurveKeys, PumpNativeFundingKeys},
        sdk::{
            damm_instructions::{compile_candidate, full_instructions},
            ordinary_lanes::ensure_requested_provider,
        },
    },
    domain::pump_native_candidate::PumpNativeCandidateRecord,
};
use anyhow::{ensure, Context, Result};
use sol_trade_sdk::{instruction::utils::pumpfun::get_creator_vault_pda, swqos::SwqosType};
use solana_sdk::{hash::Hash, pubkey::Pubkey, transaction::VersionedTransaction};
use std::{collections::HashSet, num::NonZeroU64};
use wire::{encode, NativeRoles};

pub(super) mod wire;

pub(in crate::adapters::sdk) fn reconstruct(
    record: &PumpNativeCandidateRecord,
) -> Result<VersionedTransaction> {
    let d = record.details();
    let wallet = Pubkey::new_from_array(d.wallet);
    let base_program = Pubkey::new_from_array(d.base_program);
    ensure!(
        base_program == spl_token_interface::ID && d.priority_fee_lamports == 300,
        "historical native Pump record program or priority fee differs from the locked profile"
    );
    let curve = PumpCurveKeys::new(
        Pubkey::new_from_array(d.mint),
        Some(Pubkey::new_from_array(d.curve)),
    )?;
    let keys = PumpNativeFundingKeys::new(&curve, wallet, base_program)?;
    ensure!(
        keys.addresses()[4].to_bytes() == d.curve_vault
            && keys.addresses()[5].to_bytes() == d.user_ata
            && get_creator_vault_pda(&Pubkey::new_from_array(d.creator))
                .context("historical native Pump creator PDA")?
                .to_bytes()
                == d.creator_vault,
        "historical native Pump record derived roles differ"
    );
    let provider = SwqosType::values()
        .into_iter()
        .find(|provider| provider.as_str() == d.relay)
        .context("historical native Pump relay is not a locked SDK provider")?;
    ensure_requested_provider(d.use_mev, d.requested_mev_provider, provider)?;
    let swap = encode(
        NativeRoles {
            keys: keys.addresses(),
            protocol_fee: Pubkey::new_from_array(d.protocol_fee),
            creator_vault: Pubkey::new_from_array(d.creator_vault),
            buyback: Pubkey::new_from_array(d.buyback),
            side: d.side,
            cashback: d.cashback,
        },
        NonZeroU64::new(d.input_units).context("historical native Pump input is zero")?,
        NonZeroU64::new(d.minimum_output_units)
            .context("historical native Pump minimum is zero")?,
    )?;
    ensure!(
        swap.accounts
            .iter()
            .map(|meta| meta.pubkey)
            .collect::<HashSet<_>>()
            .len()
            == swap.accounts.len()
            && d.tip_destination.is_none_or(|key| !swap
                .accounts
                .iter()
                .any(|meta| meta.pubkey.to_bytes() == key)),
        "historical native Pump account roles or tip destination overlap"
    );
    let instructions = full_instructions(
        wallet,
        provider,
        false,
        f64::from_bits(d.tip_sol_bits),
        d.tip_destination.map(Pubkey::new_from_array),
        &[swap],
    )?;
    compile_candidate(
        wallet,
        &instructions,
        Hash::new_from_array(d.recent_blockhash),
    )
}

pub(crate) fn validate(record: &PumpNativeCandidateRecord, message: &[u8]) -> Result<()> {
    record.validate(message)?;
    let expected = reconstruct(record)?;
    ensure!(
        expected.message.serialize() == message,
        "historical native Pump message differs from its nominal roles, amounts or envelope"
    );
    Ok(())
}

#[cfg(test)]
pub(crate) fn assert_native_history_only(json: &str, message: &[u8]) {
    use crate::domain::{sdk_prepared_record::SdkPreparedRecord, submission::SignedSubmission};
    let record = SdkPreparedRecord::decode(json, message).unwrap();
    assert!(matches!(record, SdkPreparedRecord::PumpNative(_)));
    assert_eq!(record.encode(message).unwrap(), json);
    assert!(record.buy().is_err() && record.sell().is_err());
    crate::adapters::sdk::prepared_candidate::validate_candidate(&record, message).unwrap();
    let error = crate::adapters::sdk::prepared_candidate::validate_signed_candidate(
        &record,
        message,
        &SignedSubmission {
            signature: String::new(),
            bytes: vec![],
        },
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("historical Pump candidate cannot enter signing"));
}
