use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_trade_sdk::constants::{USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT};
use sol_trade_sdk::instruction::utils::pumpfun::global_constants;
use solana_account_decoder::{UiAccountData, UiAccountEncoding};

mod shared_read;

pub(super) const BASE: Pubkey = Pubkey::new_from_array([101; 32]);
const CREATOR: Pubkey = Pubkey::new_from_array([102; 32]);
const FEE: Pubkey = Pubkey::new_from_array([103; 32]);

pub(super) fn ui(owner: Pubkey, data: &[u8]) -> UiAccount {
    UiAccount {
        lamports: 1,
        data: UiAccountData::Binary(STANDARD.encode(data), UiAccountEncoding::Base64),
        owner: owner.to_string(),
        executable: false,
        rent_epoch: 0,
        space: Some(data.len() as u64),
    }
}

fn curve(quote: Option<Pubkey>) -> Vec<u8> {
    // Independently encoded Borsh prefix, using the existing SDK loader fixture values.
    let mut bytes = vec![23, 183, 248, 55, 96, 216, 172, 96];
    for n in [1_000_001u64, 2_000_003, 3_000, 4_000, 9_000_000] {
        bytes.extend(n.to_le_bytes());
    }
    bytes.push(0);
    bytes.extend(CREATOR.as_ref());
    bytes.extend([0, 1]);
    if let Some(quote) = quote {
        bytes.extend(quote.as_ref());
        bytes.extend([0xff; 51]); // Opaque modern curve tail is not used for permissions.
    }
    bytes
}

fn mint(initialized: bool) -> Vec<u8> {
    use spl_token_interface::state::Mint;
    let mut bytes = vec![0; Mint::LEN];
    Mint::pack(
        Mint {
            supply: 9_000_000,
            decimals: 6,
            is_initialized: initialized,
            ..Mint::default()
        },
        &mut bytes,
    )
    .unwrap();
    bytes
}

fn global() -> Vec<u8> {
    let mut bytes = pumpfun::GLOBAL_ACCOUNT_DISCRIMINATOR.to_vec();
    bytes.push(1);
    bytes.extend([104; 32]);
    bytes.extend(FEE.as_ref());
    bytes
}

fn sharing(status: u8) -> Vec<u8> {
    let mut bytes = pumpfun::SHARING_CONFIG_ACCOUNT_DISCRIMINATOR.to_vec();
    bytes.extend([sharing_bump(), 1, status]);
    bytes.extend(BASE.as_ref());
    bytes
}

fn sharing_bump() -> u8 {
    Pubkey::find_program_address(&[b"sharing-config", BASE.as_ref()], &accounts::FEE_PROGRAM).1
}

pub(super) fn snapshot(
    raw_quote: Option<Pubkey>,
    program: Pubkey,
) -> [Option<UiAccount>; ACCOUNT_COUNT] {
    [
        Some(ui(accounts::PUMPFUN, &curve(raw_quote))),
        Some(ui(program, &mint(true))),
        Some(ui(accounts::PUMPFUN, &global())),
        None,
    ]
}

pub(super) fn sol() -> PumpCurveAccounts {
    PumpCurveAccounts::new(BASE, None, WSOL_TOKEN_ACCOUNT).unwrap()
}

fn decode(
    plan: &PumpCurveAccounts,
    values: &[Option<UiAccount>],
) -> anyhow::Result<PumpCurveEvidence> {
    plan.decode(plan.keys(), 99, 98, values)
}

#[test]
fn derives_roles_and_binds_legacy_and_current_quote_prefixes() {
    for (raw, quote, program) in [
        (None, WSOL_TOKEN_ACCOUNT, spl_token_interface::ID),
        (
            Some(Pubkey::default()),
            WSOL_TOKEN_ACCOUNT,
            spl_token_interface::ID,
        ),
        (
            Some(SOL_TOKEN_ACCOUNT),
            WSOL_TOKEN_ACCOUNT,
            spl_token_2022_interface::ID,
        ),
        (
            Some(WSOL_TOKEN_ACCOUNT),
            WSOL_TOKEN_ACCOUNT,
            spl_token_2022_interface::ID,
        ),
        (
            Some(USDC_TOKEN_ACCOUNT),
            USDC_TOKEN_ACCOUNT,
            spl_token_2022_interface::ID,
        ),
        (Some(CREATOR), CREATOR, spl_token_interface::ID),
    ] {
        let curve =
            Pubkey::find_program_address(&[b"bonding-curve", BASE.as_ref()], &accounts::PUMPFUN).0;
        let sharing = Pubkey::find_program_address(
            &[b"sharing-config", BASE.as_ref()],
            &accounts::FEE_PROGRAM,
        )
        .0;
        let plan = PumpCurveAccounts::new(BASE, Some(curve), quote).unwrap();
        let evidence = decode(&plan, &snapshot(raw, program)).unwrap();
        assert_eq!(
            evidence.keys,
            [curve, BASE, global_constants::GLOBAL_ACCOUNT, sharing]
        );
        assert_eq!(evidence.slot, 99);
        assert_eq!(evidence.quote, quote);
        assert_eq!(
            evidence.curve,
            CurvePrefix {
                virtual_token_reserves: 1_000_001,
                virtual_quote_reserves: 2_000_003,
                real_token_reserves: 3_000,
                real_quote_reserves: 4_000,
                total_supply: 9_000_000,
                complete: false,
                creator: CREATOR,
                mayhem: false,
                cashback: true,
                raw_quote: raw,
            }
        );
        assert_eq!(
            evidence.base_mint_layout,
            BaseMintLayout {
                program,
                decimals: 6,
                supply: 9_000_000
            }
        );
        assert_eq!(evidence.global_fee_recipient, FEE);
        assert_eq!(evidence.sharing, SharingPrefix::Absent);
    }
}

#[test]
fn rejects_noncanonical_request_roles_count_bank_and_quote() {
    let plan = sol();
    let accounts = snapshot(None, spl_token_interface::ID);
    assert!(PumpCurveAccounts::new(BASE, Some(CREATOR), WSOL_TOKEN_ACCOUNT).is_err());
    for invalid in [Pubkey::default(), SOL_TOKEN_ACCOUNT, BASE] {
        assert!(PumpCurveAccounts::new(BASE, None, invalid).is_err());
    }
    assert!(
        PumpCurveAccounts::new(global_constants::GLOBAL_ACCOUNT, None, WSOL_TOKEN_ACCOUNT).is_err()
    );
    for (slot, minimum) in [(0, 0), (98, 99)] {
        assert!(plan.decode(plan.keys(), slot, minimum, &accounts).is_err());
    }
    assert!(plan.decode(plan.keys(), 99, 99, &accounts).is_ok());
    let mut reordered = *plan.keys();
    reordered.swap(0, 1);
    assert!(plan.decode(&reordered, 99, 98, &accounts).is_err());
    assert!(decode(&plan, &accounts[..3]).is_err());
    let mut extra = Vec::from(accounts.clone());
    extra.push(None);
    assert!(decode(&plan, &extra).is_err());
    for i in 0..3 {
        let mut missing = accounts.clone();
        missing[i] = None;
        assert!(decode(&plan, &missing).is_err());
    }
    let mut reordered = accounts;
    reordered.swap(0, 2); // Same owner, wrong role discriminator.
    assert!(decode(&plan, &reordered).is_err());
    let mismatch = snapshot(Some(USDC_TOKEN_ACCOUNT), spl_token_interface::ID);
    assert!(decode(&plan, &mismatch)
        .unwrap_err()
        .to_string()
        .contains("quote"));
}

#[test]
fn rejects_curve_owner_discriminator_truncation_and_invalid_account_state() {
    let original = curve(None);
    let mut discriminator = original.clone();
    discriminator[0] ^= 1;
    let mut invalid_bool = original.clone();
    invalid_bool[8 + 40] = 2;
    let mut partial_quote = original.clone();
    partial_quote.push(1);
    let cases = [
        ui(accounts::FEE_PROGRAM, &original),
        ui(accounts::PUMPFUN, &discriminator),
        ui(accounts::PUMPFUN, &original[..original.len() - 1]),
        ui(accounts::PUMPFUN, &invalid_bool),
        ui(accounts::PUMPFUN, &partial_quote),
        UiAccount {
            lamports: 0,
            ..ui(accounts::PUMPFUN, &original)
        },
        UiAccount {
            executable: true,
            ..ui(accounts::PUMPFUN, &original)
        },
        UiAccount {
            space: Some(0),
            ..ui(accounts::PUMPFUN, &original)
        },
        UiAccount {
            data: UiAccountData::Binary("*".into(), UiAccountEncoding::Base64),
            ..ui(accounts::PUMPFUN, &original)
        },
        ui(accounts::PUMPFUN, &[0; MAX_ACCOUNT_BYTES + 1]),
    ];
    let mut values = snapshot(None, spl_token_interface::ID);
    for bad in cases {
        values[0] = Some(bad);
        assert!(decode(&sol(), &values)
            .unwrap_err()
            .to_string()
            .contains("curve"));
    }
    let mut completed = original;
    completed[8 + 40] = 1;
    values[0] = Some(ui(accounts::PUMPFUN, &completed));
    assert!(decode(&sol(), &values).unwrap().curve.complete);
}

#[test]
fn validates_mint_and_global_prefix_without_deriving_fee_permission() {
    let mut values = snapshot(None, spl_token_interface::ID);
    for invalid in [
        ui(accounts::PUMPFUN, &mint(true)),
        ui(spl_token_interface::ID, &mint(false)),
        ui(spl_token_2022_interface::ID, &[]),
    ] {
        values[1] = Some(invalid);
        assert!(decode(&sol(), &values)
            .unwrap_err()
            .to_string()
            .contains("mint"));
    }
    let mut wrong_type = mint(true);
    wrong_type.resize(171, 0);
    wrong_type[165] = 2; // Token account, not mint.
    values[1] = Some(ui(spl_token_2022_interface::ID, &wrong_type));
    assert!(decode(&sol(), &values).is_err());
    values[1] = Some(ui(spl_token_interface::ID, &mint(true)));
    for (i, n) in [(0, 0), (8, 0), (8, 2)] {
        let mut invalid = global();
        invalid[i] = n;
        values[2] = Some(ui(accounts::PUMPFUN, &invalid));
        assert!(decode(&sol(), &values)
            .unwrap_err()
            .to_string()
            .contains("Global"));
    }
    let mut invalid = global();
    invalid[41..73].fill(0);
    for bad in [
        ui(accounts::PUMPFUN, &invalid),
        ui(accounts::PUMPFUN, &global()[..72]),
        ui(accounts::FEE_PROGRAM, &global()),
    ] {
        values[2] = Some(bad);
        assert!(decode(&sol(), &values).is_err());
    }
}

#[test]
fn only_a_missing_slot_entry_is_absent_not_unknown_or_malformed_sharing() {
    let plan = sol();
    let mut values = snapshot(None, spl_token_interface::ID);
    assert_eq!(
        decode(&plan, &values).unwrap().sharing,
        SharingPrefix::Absent
    );
    for status in [0, 1, 255] {
        values[3] = Some(ui(accounts::FEE_PROGRAM, &sharing(status)));
        assert_eq!(
            decode(&plan, &values).unwrap().sharing,
            SharingPrefix::Present {
                version: 1,
                bump: sharing_bump(),
                status
            }
        );
    }
    let valid = sharing(1);
    let mut wrong_discriminator = valid.clone();
    wrong_discriminator[0] ^= 1;
    let mut wrong_mint = valid.clone();
    wrong_mint[11..43].copy_from_slice(CREATOR.as_ref());
    let mut wrong_bump = valid.clone();
    wrong_bump[8] ^= 1;
    for invalid in [
        ui(accounts::PUMPFUN, &valid),
        ui(accounts::FEE_PROGRAM, &wrong_discriminator),
        ui(accounts::FEE_PROGRAM, &wrong_mint),
        ui(accounts::FEE_PROGRAM, &wrong_bump),
        ui(accounts::FEE_PROGRAM, &valid[..42]),
    ] {
        values[3] = Some(invalid);
        assert!(decode(&plan, &values)
            .unwrap_err()
            .to_string()
            .contains("sharing config"));
    }
}

#[test]
fn locked_idl_defines_the_account_prefixes_and_sharing_pda_roles() {
    use serde_json::{json, Value};
    fn named<'a>(idl: &'a Value, group: &str, name: &str) -> &'a Value {
        idl[group]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == name)
            .unwrap()
    }
    let pump: Value = serde_json::from_str(include_str!(
        "../../../../vendor/sol-trade-sdk/idl/pump.json"
    ))
    .unwrap();
    let fees: Value = serde_json::from_str(include_str!(
        "../../../../vendor/sol-trade-sdk/idl/pump_fees.json"
    ))
    .unwrap();
    for (idl, name, discriminator, expected) in [
        (
            &pump,
            "BondingCurve",
            CURVE_DISCRIMINATOR,
            &[
                ("virtual_token_reserves", "u64"),
                ("virtual_quote_reserves", "u64"),
                ("real_token_reserves", "u64"),
                ("real_quote_reserves", "u64"),
                ("token_total_supply", "u64"),
                ("complete", "bool"),
                ("creator", "pubkey"),
                ("is_mayhem_mode", "bool"),
                ("is_cashback_coin", "bool"),
                ("quote_mint", "pubkey"),
            ][..],
        ),
        (
            &pump,
            "Global",
            pumpfun::GLOBAL_ACCOUNT_DISCRIMINATOR,
            &[
                ("initialized", "bool"),
                ("authority", "pubkey"),
                ("fee_recipient", "pubkey"),
            ][..],
        ),
        (
            &fees,
            "SharingConfig",
            pumpfun::SHARING_CONFIG_ACCOUNT_DISCRIMINATOR,
            &[
                ("bump", "u8"),
                ("version", "u8"),
                ("status", "defined"),
                ("mint", "pubkey"),
            ][..],
        ),
    ] {
        assert_eq!(
            named(idl, "accounts", name)["discriminator"],
            json!(discriminator)
        );
        let fields = named(idl, "types", name)["type"]["fields"]
            .as_array()
            .unwrap();
        for (field, (name, ty)) in fields[..expected.len()]
            .iter()
            .zip(expected.iter().copied())
        {
            assert_eq!(field["name"], name);
            if ty == "defined" {
                assert_eq!(field["type"], json!({"defined":{"name":"ConfigStatus"}}));
            } else {
                assert_eq!(field["type"], ty);
            }
        }
    }
    let create = named(&fees, "instructions", "create_fee_sharing_config");
    let config = named(create, "accounts", "sharing_config");
    assert_eq!(
        config["pda"]["seeds"],
        json!([
            {"kind":"const","value":b"sharing-config"}, {"kind":"account","path":"mint"}
        ])
    );
    let evidence = decode(&sol(), &{
        let mut values = snapshot(None, spl_token_interface::ID);
        values[3] = Some(ui(accounts::FEE_PROGRAM, &sharing(1)));
        values
    })
    .unwrap();
    assert_eq!(
        evidence.keys[3],
        pumpfun::get_fee_sharing_config_pda(&BASE).unwrap()
    );
    assert_eq!(
        evidence.sharing,
        SharingPrefix::Present {
            bump: sharing_bump(),
            version: 1,
            status: 1
        }
    );
}
