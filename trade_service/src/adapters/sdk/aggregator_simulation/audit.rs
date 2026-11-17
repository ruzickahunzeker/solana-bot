use anyhow::{bail, Context};
use sol_trade_sdk::common::{spl_token, spl_token_2022};
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status_client_types::{
    UiInnerInstructions, UiInstruction, UiParsedInstruction,
};
use spl_token_2022_interface::{
    extension::StateWithExtensions, instruction::TokenInstruction as Token2022Instruction,
    state::Account as Token2022Account,
};
use spl_token_interface::{instruction::TokenInstruction, state::Account as TokenAccount};
use std::str::FromStr;

pub(crate) fn check_target_token_instruction(
    program: &Pubkey,
    data: &[u8],
    accounts: &[Pubkey],
    target_ata: &Pubkey,
) -> anyhow::Result<()> {
    if accounts.first() != Some(target_ata) {
        return Ok(());
    }
    let initialization = if *program == spl_token::ID {
        matches!(
            TokenInstruction::unpack(data).context("invalid SPL target ATA instruction")?,
            TokenInstruction::InitializeAccount
                | TokenInstruction::InitializeAccount2 { .. }
                | TokenInstruction::InitializeAccount3 { .. }
                | TokenInstruction::InitializeImmutableOwner
        )
    } else if *program == spl_token_2022::ID {
        matches!(
            Token2022Instruction::unpack(data)
                .context("invalid Token-2022 target ATA instruction")?,
            Token2022Instruction::InitializeAccount
                | Token2022Instruction::InitializeAccount2 { .. }
                | Token2022Instruction::InitializeAccount3 { .. }
                | Token2022Instruction::InitializeImmutableOwner
        )
    } else {
        return Ok(());
    };
    if !initialization {
        bail!("token instruction attempts to modify target ATA beyond initialization");
    }
    Ok(())
}

pub(crate) fn check_inner_target_token_instructions(
    groups: &[UiInnerInstructions],
    keys: &[Pubkey],
    target_ata: &Pubkey,
) -> anyhow::Result<()> {
    for group in groups {
        for instruction in &group.instructions {
            match instruction {
                UiInstruction::Compiled(ix) => {
                    let program = keys
                        .get(usize::from(ix.program_id_index))
                        .context("invalid inner token program index")?;
                    let accounts = ix
                        .accounts
                        .iter()
                        .map(|index| {
                            keys.get(usize::from(*index))
                                .copied()
                                .context("invalid inner token account index")
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    if *program == spl_token::ID || *program == spl_token_2022::ID {
                        let data = bs58::decode(&ix.data)
                            .into_vec()
                            .context("invalid inner token instruction data")?;
                        check_target_token_instruction(program, &data, &accounts, target_ata)?;
                    }
                }
                UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix)) => {
                    let program =
                        Pubkey::from_str(&ix.program_id).context("invalid inner token program")?;
                    if program == spl_token::ID || program == spl_token_2022::ID {
                        let accounts = ix
                            .accounts
                            .iter()
                            .map(|address| {
                                Pubkey::from_str(address).context("invalid inner token account")
                            })
                            .collect::<anyhow::Result<Vec<_>>>()?;
                        let data = bs58::decode(&ix.data)
                            .into_vec()
                            .context("invalid inner token instruction data")?;
                        check_target_token_instruction(&program, &data, &accounts, target_ata)?;
                    }
                }
                UiInstruction::Parsed(UiParsedInstruction::Parsed(ix)) => {
                    let program =
                        Pubkey::from_str(&ix.program_id).context("invalid inner token program")?;
                    let target = target_ata.to_string();
                    let target_is_source_or_account =
                        ["/info/source", "/info/account"].iter().any(|path| {
                            ix.parsed.pointer(path).and_then(|value| value.as_str())
                                == Some(target.as_str())
                        });
                    if (program == spl_token::ID || program == spl_token_2022::ID)
                        && target_is_source_or_account
                        && !matches!(
                            ix.parsed.get("type").and_then(|value| value.as_str()),
                            Some(
                                "initializeAccount"
                                    | "initializeAccount2"
                                    | "initializeAccount3"
                                    | "initializeImmutableOwner"
                            )
                        )
                    {
                        bail!("parsed token instruction references target ATA unexpectedly");
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn decoded_system_transfer_amount(
    data: &[u8],
    accounts: &[Pubkey],
    payer: &Pubkey,
) -> Option<u64> {
    if data.len() != 12 || data[..4] != [2, 0, 0, 0] || accounts.len() != 2 {
        return None;
    }
    if accounts[0] != *payer {
        return Some(0);
    }
    Some(u64::from_le_bytes(data[4..12].try_into().ok()?))
}

pub(crate) fn inner_payer_system_spend(
    groups: &[UiInnerInstructions],
    keys: &[Pubkey],
    payer: &Pubkey,
) -> anyhow::Result<Option<u64>> {
    let system_program = Pubkey::default().to_string();
    let mut total = 0u64;
    for group in groups {
        for instruction in &group.instructions {
            let amount = match instruction {
                UiInstruction::Compiled(ix)
                    if keys.get(usize::from(ix.program_id_index)) == Some(&Pubkey::default()) =>
                {
                    let accounts = ix
                        .accounts
                        .iter()
                        .map(|index| {
                            keys.get(usize::from(*index))
                                .copied()
                                .context("invalid inner System account")
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    bs58::decode(&ix.data)
                        .into_vec()
                        .ok()
                        .and_then(|data| decoded_system_transfer_amount(&data, &accounts, payer))
                }
                UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix))
                    if ix.program_id == system_program =>
                {
                    let accounts = ix
                        .accounts
                        .iter()
                        .map(|address| {
                            Pubkey::from_str(address).context("invalid inner System account")
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    bs58::decode(&ix.data)
                        .into_vec()
                        .ok()
                        .and_then(|data| decoded_system_transfer_amount(&data, &accounts, payer))
                }
                UiInstruction::Parsed(UiParsedInstruction::Parsed(ix))
                    if ix.program_id == system_program =>
                {
                    if ix.parsed.get("type").and_then(|value| value.as_str()) != Some("transfer") {
                        None
                    } else {
                        match ix
                            .parsed
                            .pointer("/info/source")
                            .and_then(|value| value.as_str())
                        {
                            Some(source) if source == payer.to_string() => ix
                                .parsed
                                .pointer("/info/lamports")
                                .and_then(|value| value.as_u64()),
                            Some(_) => Some(0),
                            None => None,
                        }
                    }
                }
                _ => continue,
            };
            let Some(amount) = amount else {
                return Ok(None);
            };
            total = total
                .checked_add(amount)
                .context("inner System transfer total overflow")?;
        }
    }
    Ok(Some(total))
}

pub(crate) fn audit_inner_instruction_inventory(
    groups: &[UiInnerInstructions],
    keys: &[Pubkey],
    outer_count: usize,
) -> anyhow::Result<(usize, Vec<String>)> {
    let mut count = 0usize;
    let mut programs = Vec::new();
    for group in groups {
        if usize::from(group.index) >= outer_count {
            bail!("simulated inner instruction group has invalid outer index");
        }
        for instruction in &group.instructions {
            count += 1;
            if count > 128 {
                bail!("simulated transaction has too many inner instructions");
            }
            let program = match instruction {
                UiInstruction::Compiled(ix) => {
                    for index in &ix.accounts {
                        if usize::from(*index) >= keys.len() {
                            bail!("simulated inner instruction has invalid account index");
                        }
                    }
                    *keys
                        .get(usize::from(ix.program_id_index))
                        .context("simulated inner instruction has invalid program index")?
                }
                UiInstruction::Parsed(UiParsedInstruction::Parsed(ix)) => {
                    Pubkey::from_str(&ix.program_id)
                        .context("invalid simulated inner program ID")?
                }
                UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix)) => {
                    for address in &ix.accounts {
                        let key =
                            Pubkey::from_str(address).context("invalid simulated inner account")?;
                        if !keys.contains(&key) {
                            bail!("simulated inner account is not in transaction message");
                        }
                    }
                    Pubkey::from_str(&ix.program_id)
                        .context("invalid simulated inner program ID")?
                }
            };
            if !keys.contains(&program) {
                bail!("simulated inner program is not in transaction message");
            }
            let program = program.to_string();
            if !programs.contains(&program) {
                programs.push(program);
            }
        }
    }
    Ok((count, programs))
}

pub(crate) fn same_slot_deltas(
    baseline_slot: u64,
    simulation_slot: u64,
    payer_before: u64,
    payer_after: Option<u64>,
    token_before: u64,
    token_after: Option<u64>,
) -> (Option<u64>, Option<u64>) {
    if baseline_slot != simulation_slot {
        return (None, None);
    }
    (
        payer_after.and_then(|after| payer_before.checked_sub(after)),
        token_after.and_then(|after| after.checked_sub(token_before)),
    )
}

pub(crate) fn target_account_amount_from_data(
    account_owner: &Pubkey,
    data: &[u8],
    token_program: &Pubkey,
    mint: &Pubkey,
    wallet: &Pubkey,
) -> anyhow::Result<u64> {
    if account_owner != token_program {
        bail!("target ATA has unexpected token program");
    }
    let (account_mint, account_wallet, amount) = if *token_program == spl_token::ID {
        let account = TokenAccount::unpack(data).context("invalid SPL target ATA")?;
        (account.mint, account.owner, account.amount)
    } else if *token_program == spl_token_2022::ID {
        let account = StateWithExtensions::<Token2022Account>::unpack(data)
            .context("invalid Token-2022 target ATA")?;
        (account.base.mint, account.base.owner, account.base.amount)
    } else {
        bail!("unsupported target ATA token program");
    };
    if account_mint.as_ref() != mint.as_ref() || account_wallet.as_ref() != wallet.as_ref() {
        bail!("target ATA has unexpected mint or wallet owner");
    }
    Ok(amount)
}

pub(crate) fn simulated_target_account_amount(
    account: &solana_account_decoder::UiAccount,
    token_program: &Pubkey,
    mint: &Pubkey,
    wallet: &Pubkey,
) -> anyhow::Result<u64> {
    let owner = Pubkey::from_str(&account.owner).context("invalid simulated ATA program owner")?;
    let data = account
        .data
        .decode()
        .context("simulated target ATA has no binary data")?;
    target_account_amount_from_data(&owner, &data, token_program, mint, wallet)
}

pub(crate) fn target_token_amount(
    balances: &[solana_transaction_status_client_types::UiTransactionTokenBalance],
    account_index: u8,
    mint: &Pubkey,
) -> Option<u64> {
    let mut matching = balances.iter().filter(|balance| {
        balance.account_index == account_index && balance.mint == mint.to_string()
    });
    let amount = matching
        .next()?
        .ui_token_amount
        .amount
        .parse::<u64>()
        .ok()?;
    if matching.next().is_some() {
        return None;
    }
    Some(amount)
}
