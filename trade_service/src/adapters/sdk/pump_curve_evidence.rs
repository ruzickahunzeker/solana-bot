//! Offline Pump.fun account-prefix evidence. This is not a quote or an execution permit.
use super::pumpapi_assets::{decode_data, MAX_ACCOUNT_BYTES};
use crate::adapters::rpc::{
    parameter_reads::request::pump_curve::{PumpCurveKeys, ACCOUNT_COUNT},
    parameter_reads::request::ParameterQuery,
    read_http::ReadHttpTransport,
    read_runtime::SharedReadRuntime,
};
use anyhow::{bail, Context};
use sol_trade_sdk::{
    common::bonding_curve::BondingCurveAccount,
    constants::SOL_TOKEN_ACCOUNT,
    instruction::utils::pumpfun::{self, accounts},
};
use solana_account_decoder::UiAccount;
use solana_client::rpc_response::Response;
use solana_program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token_2022_interface::extension::{BaseStateWithExtensions, StateWithExtensions};
use std::time::{Duration, Instant};

const CURVE_DISCRIMINATOR: [u8; 8] = [23, 183, 248, 55, 96, 216, 172, 96];
struct PumpCurveAccounts {
    // Canonical curve, base mint, Global, sharing config. RPC values are positional.
    keys: PumpCurveKeys,
    quote: Pubkey,
}

#[derive(Debug)]
struct PumpCurveEvidence {
    slot: u64,
    keys: [Pubkey; ACCOUNT_COUNT],
    quote: Pubkey,
    curve: CurvePrefix,
    base_mint_layout: BaseMintLayout,
    global_fee_recipient: Pubkey,
    global_data: Vec<u8>,
    sharing: SharingPrefix,
}

#[derive(Debug, PartialEq, Eq)]
struct CurvePrefix {
    virtual_token_reserves: u64,
    virtual_quote_reserves: u64,
    real_token_reserves: u64,
    real_quote_reserves: u64,
    total_supply: u64,
    complete: bool,
    creator: Pubkey,
    mayhem: bool,
    cashback: bool,
    raw_quote: Option<Pubkey>,
}

#[derive(Debug, PartialEq, Eq)]
struct BaseMintLayout {
    program: Pubkey,
    decimals: u8,
    supply: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum SharingPrefix {
    Absent,
    Present { version: u8, bump: u8, status: u8 },
}

impl PumpCurveAccounts {
    fn keys(&self) -> &[Pubkey; ACCOUNT_COUNT] {
        self.keys.addresses()
    }

    fn new(mint: Pubkey, expected_curve: Option<Pubkey>, quote: Pubkey) -> anyhow::Result<Self> {
        if mint == Pubkey::default()
            || mint == SOL_TOKEN_ACCOUNT
            || quote == Pubkey::default()
            || quote == SOL_TOKEN_ACCOUNT
            || mint == quote
        {
            bail!("Pump.fun base or canonical quote mint is invalid");
        }
        Ok(Self {
            keys: PumpCurveKeys::new(mint, expected_curve)?,
            quote,
        })
    }

    async fn read<T: ReadHttpTransport>(
        &self,
        runtime: &SharedReadRuntime<T>,
        url: &str,
        minimum_slot: u64,
        caller_deadline: Instant,
    ) -> anyhow::Result<PumpCurveEvidence> {
        read_bank(
            runtime,
            url,
            caller_deadline,
            self.keys.query(minimum_slot)?,
            |slot, values| self.decode(self.keys(), slot, minimum_slot, values),
        )
        .await
    }

    async fn discover<T: ReadHttpTransport>(
        &self,
        runtime: &SharedReadRuntime<T>,
        url: &str,
        caller_deadline: Instant,
    ) -> anyhow::Result<PumpCurveEvidence> {
        read_bank(
            runtime,
            url,
            caller_deadline,
            self.keys.discovery_query(),
            |slot, values| self.decode(self.keys(), slot, slot, values),
        )
        .await
    }

    // Only a successful, complete read may be passed here. Transport failure is not absence.
    fn decode(
        &self,
        requested_keys: &[Pubkey],
        slot: u64,
        minimum_slot: u64,
        values: &[Option<UiAccount>],
    ) -> anyhow::Result<PumpCurveEvidence> {
        if requested_keys != self.keys().as_slice() || slot == 0 || slot < minimum_slot {
            bail!("Pump.fun snapshot request or bank context is inconsistent");
        }
        let [curve, base, global, sharing] = values else {
            bail!("Pump.fun snapshot account count is inconsistent");
        };
        let curve = decode_data(
            curve.as_ref().context("Pump.fun curve is missing")?,
            accounts::PUMPFUN,
            MAX_ACCOUNT_BYTES,
            "Pump.fun curve",
        )?;
        let body = curve
            .strip_prefix(&CURVE_DISCRIMINATOR)
            .context("Pump.fun curve discriminator is inconsistent")?;
        // Don't interpret appended fields as fee or execution evidence.
        let curve = BondingCurveAccount::decode_body(&body[..body.len().min(107)])
            .context("Pump.fun curve body is invalid")?;
        if curve.effective_quote_mint() != self.quote {
            bail!("Pump.fun curve quote does not match the requested mint");
        }
        let base_mint_layout =
            decode_base_mint(base.as_ref().context("Pump.fun mint is missing")?)?;
        let (global_fee_recipient, global_data) =
            decode_global(global.as_ref().context("Pump.fun Global is missing")?)?;
        let sharing = decode_sharing(sharing.as_ref(), self.keys()[1], self.keys.sharing_bump())?;
        Ok(PumpCurveEvidence {
            slot,
            keys: *self.keys(),
            quote: self.quote,
            curve: CurvePrefix {
                virtual_token_reserves: curve.virtual_token_reserves,
                virtual_quote_reserves: curve.virtual_quote_reserves(),
                real_token_reserves: curve.real_token_reserves,
                real_quote_reserves: curve.real_quote_reserves(),
                total_supply: curve.token_total_supply,
                complete: curve.complete,
                creator: curve.creator,
                mayhem: curve.is_mayhem_mode,
                cashback: curve.is_cashback_coin,
                raw_quote: (body.len() >= 107).then_some(curve.quote_mint),
            },
            base_mint_layout,
            global_fee_recipient,
            global_data,
            sharing,
        })
    }
}

async fn read_bank<T: ReadHttpTransport, E>(
    runtime: &SharedReadRuntime<T>,
    url: &str,
    caller_deadline: Instant,
    query: ParameterQuery,
    decode: impl FnOnce(u64, &[Option<UiAccount>]) -> anyhow::Result<E>,
) -> anyhow::Result<E> {
    let deadline = caller_deadline.min(Instant::now() + Duration::from_secs(2));
    if Instant::now() >= deadline {
        bail!("Pump.fun account snapshot deadline expired before read");
    }
    let _permit = runtime.parameter_slot()?;
    let value = runtime.parameter_step(url, query, deadline).await?;
    let response: Response<Vec<Option<UiAccount>>> = serde_json::from_value(value)
        .map_err(|_| anyhow::anyhow!("Pump.fun account snapshot result is invalid"))?;
    let evidence = decode(response.context.slot, &response.value)?;
    if Instant::now() >= deadline {
        bail!("Pump.fun account snapshot deadline expired after decoding");
    }
    Ok(evidence)
}

fn decode_base_mint(account: &UiAccount) -> anyhow::Result<BaseMintLayout> {
    let program: Pubkey = account
        .owner
        .parse()
        .context("Pump.fun mint owner is invalid")?;
    if program != spl_token_interface::ID && program != spl_token_2022_interface::ID {
        bail!("Pump.fun mint token program is unsupported");
    }
    let bytes = decode_data(account, program, MAX_ACCOUNT_BYTES, "Pump.fun mint")?;
    let (decimals, supply) = if program == spl_token_interface::ID {
        let mint = spl_token_interface::state::Mint::unpack(&bytes)
            .context("Pump.fun SPL mint is invalid")?;
        (mint.decimals, mint.supply)
    } else {
        // Layout only: no epoch, hook, fee, or extension allowlist decision is made here.
        let mint = StateWithExtensions::<spl_token_2022_interface::state::Mint>::unpack(&bytes)
            .context("Pump.fun Token-2022 mint is invalid")?;
        mint.get_extension_types()
            .context("Pump.fun Token-2022 TLV layout is invalid")?;
        (mint.base.decimals, mint.base.supply)
    };
    Ok(BaseMintLayout {
        program,
        decimals,
        supply,
    })
}

fn decode_global(account: &UiAccount) -> anyhow::Result<(Pubkey, Vec<u8>)> {
    let data = decode_data(
        account,
        accounts::PUMPFUN,
        MAX_ACCOUNT_BYTES,
        "Pump.fun Global",
    )?;
    let fee: [u8; 32] = data
        .get(41..73)
        .context("Pump.fun Global prefix is truncated")?
        .try_into()?;
    if !data.starts_with(&pumpfun::GLOBAL_ACCOUNT_DISCRIMINATOR) || data[8] > 1 || fee == [0; 32] {
        bail!("Pump.fun Global prefix is invalid");
    }
    Ok((Pubkey::new_from_array(fee), data))
}

fn decode_sharing(
    account: Option<&UiAccount>,
    mint: Pubkey,
    bump: u8,
) -> anyhow::Result<SharingPrefix> {
    let Some(account) = account else {
        return Ok(SharingPrefix::Absent);
    };
    let data = decode_data(
        account,
        accounts::FEE_PROGRAM,
        MAX_ACCOUNT_BYTES,
        "Pump.fun sharing config",
    )?;
    if !data.starts_with(&pumpfun::SHARING_CONFIG_ACCOUNT_DISCRIMINATOR)
        || data.get(11..43) != Some(mint.as_ref())
        || data[8] != bump
    {
        bail!("Pump.fun sharing config discriminator, mint or bump is inconsistent");
    }
    // Preserve unknown status/version without interpreting the config or granting fee authority.
    Ok(SharingPrefix::Present {
        bump: data[8],
        version: data[9],
        status: data[10],
    })
}

mod instruction_amounts;
mod native_funding;
pub(crate) use native_funding::{
    prepare_native_evidence, NativeBoundIntent as PumpNativeBankEvidence, NativeEvidenceContext,
    NativeProfileUnsupported,
};
#[cfg(test)]
mod tests;
