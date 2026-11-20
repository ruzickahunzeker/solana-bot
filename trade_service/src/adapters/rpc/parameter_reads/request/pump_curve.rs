use super::{invalid, Kind, ParameterQuery};
use anyhow::{ensure, Context, Result};
use serde_json::json;
use sol_trade_sdk::{
    common::fast_fn::get_associated_token_address_with_program_id_fast,
    instruction::utils::pumpfun::{self, accounts, global_constants, seeds},
};
use solana_sdk::{pubkey::Pubkey, sysvar};

pub(crate) const ACCOUNT_COUNT: usize = 4;
pub(crate) const NATIVE_ACCOUNTS: usize = ACCOUNT_COUNT + 4;

pub(crate) struct PumpCurveKeys {
    addresses: [Pubkey; ACCOUNT_COUNT],
    sharing_bump: u8,
}

impl PumpCurveKeys {
    pub(crate) fn new(mint: Pubkey, expected_curve: Option<Pubkey>) -> Result<Self> {
        let curve = pumpfun::get_bonding_curve_pda(&mint).context("Pump.fun curve PDA")?;
        ensure!(
            expected_curve.is_none_or(|expected| expected == curve),
            "Pump.fun curve is not the canonical mint PDA"
        );
        let (sharing, sharing_bump) = Pubkey::try_find_program_address(
            &[seeds::SHARING_CONFIG_SEED, mint.as_ref()],
            &accounts::FEE_PROGRAM,
        )
        .context("Pump.fun sharing PDA")?;
        let keys = [curve, mint, global_constants::GLOBAL_ACCOUNT, sharing];
        ensure!(
            keys.iter()
                .enumerate()
                .all(|(index, key)| !keys[..index].contains(key)),
            "Pump.fun snapshot account roles overlap"
        );
        Ok(Self {
            addresses: keys,
            sharing_bump,
        })
    }

    pub(crate) fn addresses(&self) -> &[Pubkey; ACCOUNT_COUNT] {
        &self.addresses
    }

    pub(crate) fn sharing_bump(&self) -> u8 {
        self.sharing_bump
    }

    pub(crate) fn query(&self, minimum_slot: u64) -> Result<ParameterQuery> {
        snapshot_query(&self.addresses, minimum_slot)
    }

    pub(crate) fn discovery_query(&self) -> ParameterQuery {
        let keys: Vec<_> = self.addresses.iter().map(Pubkey::to_string).collect();
        ParameterQuery {
            kind: Kind::Accounts(ACCOUNT_COUNT),
            params: json!([keys, {"encoding":"base64","commitment":"processed","dataSlice":null,"minContextSlot":null}]),
        }
    }
}

pub(crate) struct PumpNativeFundingKeys([Pubkey; NATIVE_ACCOUNTS]);

impl PumpNativeFundingKeys {
    pub(crate) fn new(curve: &PumpCurveKeys, wallet: Pubkey, base_program: Pubkey) -> Result<Self> {
        ensure!(
            wallet != Pubkey::default(),
            "native Pump funding wallet is invalid"
        );
        ensure!(
            base_program == spl_token_interface::ID || base_program == spl_token_2022_interface::ID,
            "native Pump base token program is unsupported"
        );
        let [curve, mint, global, sharing] = *curve.addresses();
        let curve_base =
            get_associated_token_address_with_program_id_fast(&curve, &mint, &base_program);
        let user_base =
            get_associated_token_address_with_program_id_fast(&wallet, &mint, &base_program);
        let keys = [
            curve,
            mint,
            global,
            sharing,
            curve_base,
            user_base,
            wallet,
            sysvar::clock::id(),
        ];
        ensure!(
            keys.iter()
                .enumerate()
                .all(|(i, key)| !keys[..i].contains(key)),
            "native Pump funding roles overlap"
        );
        Ok(Self(keys))
    }

    pub(crate) fn addresses(&self) -> &[Pubkey; NATIVE_ACCOUNTS] {
        &self.0
    }

    pub(crate) fn query(&self, minimum_slot: u64) -> Result<ParameterQuery> {
        snapshot_query(&self.0, minimum_slot)
    }
}

fn snapshot_query<const N: usize>(
    addresses: &[Pubkey; N],
    minimum_slot: u64,
) -> Result<ParameterQuery> {
    ensure!(
        minimum_slot > 0,
        invalid("Pump.fun snapshot requires a nonzero minimum slot")
    );
    let keys: Vec<_> = addresses.iter().map(Pubkey::to_string).collect();
    Ok(ParameterQuery {
        kind: Kind::Accounts(N),
        params: json!([keys, {"encoding":"base64","commitment":"processed","dataSlice":null,"minContextSlot":minimum_slot}]),
    })
}
