use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sol_trade_sdk::instruction::utils::meteora_damm_v2::accounts;
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status_client_types::{
    UiInnerInstructions, UiInstruction, UiParsedInstruction,
};
use spl_token_2022_interface::instruction::TokenInstruction;

use super::super::plan::{PreparedDammProbe, ProbeView};

pub(in crate::adapters::sdk::damm_probe) struct Transfer {
    pub program: Pubkey,
    pub from: Pubkey,
    pub to: Pubkey,
    pub authority: Pubkey,
    pub mint: Option<Pubkey>,
    pub amount: u64,
    pub decimals: Option<u8>,
    pub fee: Option<u64>,
}
pub(in crate::adapters::sdk::damm_probe) struct Transfers {
    pub input: Transfer,
    pub output: Transfer,
    pub output_initialized: bool,
    pub input_initialized: bool,
    pub native_deposit_units: u64,
}
enum Action {
    Transfer(Transfer),
    Initialize {
        account: Pubkey,
        mint: Pubkey,
        owner: Pubkey,
    },
    Immutable(Pubkey),
    Size(Pubkey),
}

pub(super) fn integer(text: &str) -> Result<u64> {
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("DAMM v2 token amount must be an unsigned integer string");
    }
    text.parse().context("DAMM v2 token amount exceeds u64")
}
fn key(text: &str, keys: &[Pubkey]) -> Result<Pubkey> {
    let key = text
        .parse::<Pubkey>()
        .context("DAMM v2 inner instruction address is invalid")?;
    if !keys.contains(&key) {
        bail!("DAMM v2 inner account is not in the fixed message");
    }
    Ok(key)
}
fn index(index: u8, keys: &[Pubkey]) -> Result<Pubkey> {
    keys.get(usize::from(index))
        .copied()
        .context("DAMM v2 inner account index is invalid")
}

pub(super) fn transfers(
    prepared: &PreparedDammProbe,
    groups: &[UiInnerInstructions],
) -> Result<Transfers> {
    transfers_for(
        prepared.view(),
        groups,
        Layout {
            input_setup: None,
            output_setup: 0,
            swap: 1,
            native_deposit: false,
        },
    )
}

pub(in crate::adapters::sdk::damm_probe) struct Layout {
    pub input_setup: Option<u8>,
    pub output_setup: u8,
    pub swap: u8,
    pub native_deposit: bool,
}

pub(in crate::adapters::sdk::damm_probe) fn transfers_for(
    view: ProbeView<'_>,
    groups: &[UiInnerInstructions],
    layout: Layout,
) -> Result<Transfers> {
    let intent = view.accounts;
    let keys = &view.message.account_keys;
    if layout.native_deposit
        && (intent.input_mint != sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
            || layout.input_setup.is_none())
    {
        bail!("DAMM v2 native deposit trace has inconsistent input roles");
    }
    if groups.len() > if layout.input_setup.is_some() { 3 } else { 2 } {
        bail!("DAMM v2 simulation has too many inner instruction groups");
    }
    let a_in = intent.input_mint == intent.pool.token_a_mint;
    let (input_vault, output_vault, input_program, output_program) = if a_in {
        (
            intent.pool.token_a_vault,
            intent.pool.token_b_vault,
            intent.pool.token_a_program,
            intent.pool.token_b_program,
        )
    } else {
        (
            intent.pool.token_b_vault,
            intent.pool.token_a_vault,
            intent.pool.token_b_program,
            intent.pool.token_a_program,
        )
    };
    let mut previous = None;
    let mut count = 0;
    let mut input = None;
    let mut output = None;
    let mut output_initialized = false;
    let mut input_initialized = false;
    for group in groups {
        if (Some(group.index) != layout.input_setup
            && group.index != layout.output_setup
            && group.index != layout.swap)
            || previous.is_some_and(|index| group.index <= index)
        {
            bail!("DAMM v2 inner instruction group is duplicate or out of order");
        }
        previous = Some(group.index);
        for instruction in &group.instructions {
            count += 1;
            if count > 64 {
                bail!("DAMM v2 simulation has too many inner instructions");
            }
            let program = match instruction {
                UiInstruction::Compiled(ix) => {
                    for account in &ix.accounts {
                        index(*account, keys)?;
                    }
                    index(ix.program_id_index, keys)?
                }
                UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix)) => {
                    for account in &ix.accounts {
                        key(account, keys)?;
                    }
                    key(&ix.program_id, keys)?
                }
                UiInstruction::Parsed(UiParsedInstruction::Parsed(ix)) => {
                    key(&ix.program_id, keys)?
                }
            };
            if program != spl_token_interface::ID && program != spl_token_2022_interface::ID {
                let allowed = if group.index != layout.swap {
                    program == solana_system_interface::program::id()
                        || program == spl_associated_token_account_interface::program::id()
                } else {
                    program == accounts::METEORA_DAMM_V2
                };
                if !allowed {
                    bail!("DAMM v2 simulation invoked an unaudited inner program");
                }
                continue;
            }
            let action = match instruction {
                UiInstruction::Compiled(ix) => raw(
                    program,
                    &ix.accounts
                        .iter()
                        .map(|i| index(*i, keys))
                        .collect::<Result<Vec<_>>>()?,
                    &ix.data,
                )?,
                UiInstruction::Parsed(UiParsedInstruction::PartiallyDecoded(ix)) => raw(
                    program,
                    &ix.accounts
                        .iter()
                        .map(|s| key(s, keys))
                        .collect::<Result<Vec<_>>>()?,
                    &ix.data,
                )?,
                UiInstruction::Parsed(UiParsedInstruction::Parsed(ix)) => {
                    parsed(program, &ix.parsed, keys)?
                }
            };
            match (group.index, action) {
                (
                    setup,
                    Action::Initialize {
                        account,
                        mint,
                        owner,
                    },
                ) if setup == layout.output_setup
                    && program == output_program
                    && account == intent.output_ata
                    && mint == intent.output_mint
                    && owner == intent.wallet =>
                {
                    if output_initialized {
                        bail!("DAMM v2 simulation initialized output ATA twice");
                    }
                    output_initialized = true;
                }
                (setup, Action::Immutable(account))
                    if setup == layout.output_setup
                        && program == output_program
                        && account == intent.output_ata => {}
                (setup, Action::Size(mint))
                    if setup == layout.output_setup
                        && program == output_program
                        && mint == intent.output_mint => {}
                (
                    setup,
                    Action::Initialize {
                        account,
                        mint,
                        owner,
                    },
                ) if Some(setup) == layout.input_setup
                    && program == input_program
                    && account == intent.input_ata
                    && mint == intent.input_mint
                    && owner == intent.wallet =>
                {
                    if input_initialized {
                        bail!("DAMM v2 simulation initialized input ATA twice");
                    }
                    input_initialized = true;
                }
                (setup, Action::Immutable(account))
                    if Some(setup) == layout.input_setup
                        && program == input_program
                        && account == intent.input_ata => {}
                (setup, Action::Size(mint))
                    if Some(setup) == layout.input_setup
                        && program == input_program
                        && mint == intent.input_mint => {}
                (swap, Action::Transfer(transfer)) if swap == layout.swap => {
                    if transfer.program == input_program
                        && transfer.from == intent.input_ata
                        && transfer.to == input_vault
                        && transfer.authority == intent.wallet
                    {
                        if input.replace(transfer).is_some() {
                            bail!("DAMM v2 simulation has multiple input transfers");
                        }
                    } else if transfer.program == output_program
                        && transfer.from == output_vault
                        && transfer.to == intent.output_ata
                        && transfer.authority == accounts::AUTHORITY
                    {
                        if output.replace(transfer).is_some() {
                            bail!("DAMM v2 simulation has multiple output transfers");
                        }
                    } else {
                        bail!("DAMM v2 transfer does not match this swap's account roles");
                    }
                }
                _ => {
                    bail!("DAMM v2 simulation token instruction is outside the fixed quote profile")
                }
            }
        }
    }
    Ok(Transfers {
        input: input.context("DAMM v2 simulation omitted the input transfer")?,
        output: output.context("DAMM v2 simulation omitted the output transfer")?,
        output_initialized,
        input_initialized,
        native_deposit_units: if layout.native_deposit {
            intent.amount
        } else {
            0
        },
    })
}

fn raw(program: Pubkey, accounts: &[Pubkey], encoded: &str) -> Result<Action> {
    if encoded.len() > 512 {
        bail!("DAMM v2 inner token data exceeds limit");
    }
    let data = bs58::decode(encoded)
        .into_vec()
        .context("DAMM v2 inner token data is not base58")?;
    if data.len() > 256 {
        bail!("DAMM v2 inner token data exceeds limit");
    }
    if program == spl_token_interface::ID {
        let classic = spl_token_interface::instruction::TokenInstruction::unpack(&data)
            .context("DAMM v2 SPL token instruction is invalid")?;
        if let spl_token_interface::instruction::TokenInstruction::Transfer { amount } = classic {
            if classic.pack() != data || accounts.len() != 3 {
                bail!("DAMM v2 SPL transfer has trailing data or extra signers");
            }
            return Ok(Action::Transfer(Transfer {
                program,
                from: accounts[0],
                to: accounts[1],
                authority: accounts[2],
                mint: None,
                amount,
                decimals: None,
                fee: None,
            }));
        }
    }
    let instruction =
        TokenInstruction::unpack(&data).context("DAMM v2 inner token instruction is invalid")?;
    let (amount, decimals, fee, checked) = if matches!(
        instruction,
        TokenInstruction::TransferFeeExtension
    ) {
        use spl_token_2022_interface::extension::transfer_fee::instruction::TransferFeeInstruction;
        if program != spl_token_2022_interface::ID {
            bail!("DAMM v2 transfer fee instruction uses the wrong program");
        }
        let extension = TransferFeeInstruction::unpack(&data[1..])?;
        let mut canonical = instruction.pack();
        extension.pack(&mut canonical);
        if canonical != data {
            bail!("DAMM v2 inner transfer fee data has trailing bytes");
        }
        let TransferFeeInstruction::TransferCheckedWithFee {
            amount,
            decimals,
            fee,
        } = extension
        else {
            bail!("DAMM v2 transfer fee instruction is not a transfer");
        };
        (amount, Some(decimals), Some(fee), true)
    } else {
        if instruction.pack() != data {
            bail!("DAMM v2 inner token data has trailing bytes");
        }
        match instruction {
            TokenInstruction::TransferChecked { amount, decimals } => {
                (amount, Some(decimals), None, true)
            }
            TokenInstruction::InitializeAccount if accounts.len() == 4 => {
                return Ok(Action::Initialize {
                    account: accounts[0],
                    mint: accounts[1],
                    owner: accounts[2],
                })
            }
            TokenInstruction::InitializeAccount2 { owner } if accounts.len() == 3 => {
                return Ok(Action::Initialize {
                    account: accounts[0],
                    mint: accounts[1],
                    owner,
                })
            }
            TokenInstruction::InitializeAccount3 { owner } if accounts.len() == 2 => {
                return Ok(Action::Initialize {
                    account: accounts[0],
                    mint: accounts[1],
                    owner,
                })
            }
            TokenInstruction::InitializeImmutableOwner if accounts.len() == 1 => {
                return Ok(Action::Immutable(accounts[0]))
            }
            TokenInstruction::GetAccountDataSize { .. } if accounts.len() == 1 => {
                return Ok(Action::Size(accounts[0]))
            }
            _ => bail!("DAMM v2 simulation token mutation is not audited"),
        }
    };
    if accounts.len() != if checked { 4 } else { 3 } {
        bail!("DAMM v2 transfer has missing accounts or extra signers");
    }
    Ok(Action::Transfer(Transfer {
        program,
        from: accounts[0],
        to: accounts[if checked { 2 } else { 1 }],
        authority: accounts[if checked { 3 } else { 2 }],
        mint: checked.then_some(accounts[1]),
        amount,
        decimals,
        fee,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransferInfo {
    source: String,
    destination: String,
    authority: String,
    mint: Option<String>,
    amount: Option<String>,
    token_amount: Option<RawAmount>,
    fee_amount: Option<RawAmount>,
}
#[derive(Deserialize)]
struct RawAmount {
    amount: String,
    decimals: u8,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitInfo {
    account: String,
    mint: Option<String>,
    owner: Option<String>,
    rent_sysvar: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SizeInfo {
    mint: String,
    extension_types: Option<Vec<String>>,
}

fn parsed(program: Pubkey, value: &serde_json::Value, keys: &[Pubkey]) -> Result<Action> {
    let kind = value
        .get("type")
        .and_then(|v| v.as_str())
        .context("DAMM v2 parsed token instruction has no type")?;
    let info = value
        .get("info")
        .context("DAMM v2 parsed token instruction has no info")?;
    match kind {
        "transfer" | "transferChecked" | "transferCheckedWithFee" => {
            let info: TransferInfo = serde_json::from_value(info.clone())
                .context("DAMM v2 parsed transfer layout is invalid")?;
            let (amount, decimals, fee) = if kind == "transfer" {
                if program != spl_token_interface::ID {
                    bail!("DAMM v2 unchecked Token-2022 transfer is not audited");
                }
                if info.mint.is_some() || info.token_amount.is_some() || info.fee_amount.is_some() {
                    bail!("DAMM v2 parsed unchecked transfer is ambiguous");
                }
                (
                    integer(
                        info.amount
                            .as_deref()
                            .context("DAMM v2 transfer amount missing")?,
                    )?,
                    None,
                    None,
                )
            } else {
                if info.amount.is_some() || info.mint.is_none() {
                    bail!("DAMM v2 parsed checked transfer is ambiguous");
                }
                let token = info
                    .token_amount
                    .context("DAMM v2 parsed tokenAmount is missing")?;
                let fee = if kind == "transferCheckedWithFee" {
                    if program != spl_token_2022_interface::ID {
                        bail!("DAMM v2 parsed fee transfer uses wrong program");
                    }
                    let fee = info
                        .fee_amount
                        .context("DAMM v2 parsed feeAmount is missing")?;
                    if fee.decimals != token.decimals {
                        bail!("DAMM v2 parsed fee decimals disagree");
                    }
                    Some(integer(&fee.amount)?)
                } else {
                    if info.fee_amount.is_some() {
                        bail!("DAMM v2 parsed transfer has unexpected fee");
                    }
                    None
                };
                (integer(&token.amount)?, Some(token.decimals), fee)
            };
            Ok(Action::Transfer(Transfer {
                program,
                from: key(&info.source, keys)?,
                to: key(&info.destination, keys)?,
                authority: key(&info.authority, keys)?,
                mint: info.mint.as_deref().map(|s| key(s, keys)).transpose()?,
                amount,
                decimals,
                fee,
            }))
        }
        "initializeAccount"
        | "initializeAccount2"
        | "initializeAccount3"
        | "initializeImmutableOwner" => {
            let info: InitInfo = serde_json::from_value(info.clone())
                .context("DAMM v2 parsed initialize layout is invalid")?;
            let account = key(&info.account, keys)?;
            if let Some(rent) = &info.rent_sysvar {
                key(rent, keys)?;
            }
            if kind == "initializeImmutableOwner" {
                if info.mint.is_some() || info.owner.is_some() || info.rent_sysvar.is_some() {
                    bail!("DAMM v2 parsed immutable owner instruction is ambiguous");
                }
                Ok(Action::Immutable(account))
            } else {
                Ok(Action::Initialize {
                    account,
                    mint: key(
                        info.mint
                            .as_deref()
                            .context("DAMM v2 initialize mint missing")?,
                        keys,
                    )?,
                    owner: key(
                        info.owner
                            .as_deref()
                            .context("DAMM v2 initialize owner missing")?,
                        keys,
                    )?,
                })
            }
        }
        "getAccountDataSize" => {
            let info: SizeInfo = serde_json::from_value(info.clone())?;
            if info
                .extension_types
                .as_ref()
                .is_some_and(|types| types.len() > 16)
            {
                bail!("DAMM v2 data-size extensions exceed limit");
            }
            Ok(Action::Size(key(&info.mint, keys)?))
        }
        _ => bail!("DAMM v2 parsed token mutation is not audited"),
    }
}
