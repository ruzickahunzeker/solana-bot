use super::{super::*, read_rpc::*};
use sol_trade_sdk::instruction::utils::meteora_dbc::{
    accounts::PROGRAM, decode_pool_config, decode_virtual_pool, BASE_MINT_OFFSET,
    POOL_CONFIG_DISCRIMINATOR, VIRTUAL_POOL_ACCOUNT_SIZE, VIRTUAL_POOL_DISCRIMINATOR,
};
use solana_sdk::account::Account;

// Pinned zero-copy layout: current vendor IDL and dynamic-bonding-curve
// 3b540e94b5b20ba37733de6e25f58522a0cd8961, with every used field decoded below.
const CONFIG_OFFSET: usize = 72;
const CREATOR_OFFSET: usize = 104;
const BASE_VAULT_OFFSET: usize = 168;
const QUOTE_VAULT_OFFSET: usize = 200;
const BASE_RESERVE_OFFSET: usize = 232;
const QUOTE_RESERVE_OFFSET: usize = 240;
const SQRT_PRICE_OFFSET: usize = 280;
const POOL_TYPE_OFFSET: usize = 304;
const MIGRATED_OFFSET: usize = 305;

struct Expected {
    id: Pubkey,
    config: Pubkey,
    base: Pubkey,
    quote: Pubkey,
    base_vault: Pubkey,
    quote_vault: Pubkey,
    pool: Account,
    config_account: Account,
}

fn fixture(base_program: Pubkey) -> (ReadRpc, Expected) {
    let id = Pubkey::new_from_array([61; 32]);
    let config = Pubkey::new_from_array([62; 32]);
    let base = Pubkey::new_from_array([63; 32]);
    let quote = Pubkey::from_str(WSOL_MINT).unwrap();
    let creator = Pubkey::new_from_array([64; 32]);
    let base_vault = Pubkey::new_from_array([65; 32]);
    let quote_vault = Pubkey::new_from_array([66; 32]);
    assert_eq!(VIRTUAL_POOL_ACCOUNT_SIZE, 424);
    assert_eq!(BASE_MINT_OFFSET, 136);
    let mut data = vec![0; VIRTUAL_POOL_ACCOUNT_SIZE as usize];
    data[..8].copy_from_slice(&VIRTUAL_POOL_DISCRIMINATOR);
    for (offset, key) in [
        (CONFIG_OFFSET, config),
        (CREATOR_OFFSET, creator),
        (BASE_MINT_OFFSET, base),
        (BASE_VAULT_OFFSET, base_vault),
        (QUOTE_VAULT_OFFSET, quote_vault),
    ] {
        data[offset..offset + 32].copy_from_slice(key.as_ref());
    }
    data[BASE_RESERVE_OFFSET..BASE_RESERVE_OFFSET + 8].copy_from_slice(&1_000_001u64.to_le_bytes());
    data[QUOTE_RESERVE_OFFSET..QUOTE_RESERVE_OFFSET + 8]
        .copy_from_slice(&2_000_003u64.to_le_bytes());
    data[SQRT_PRICE_OFFSET..SQRT_PRICE_OFFSET + 16].copy_from_slice(&(1u128 << 64).to_le_bytes());
    let pool_type = u8::from(base_program == spl_token_2022_interface::ID);
    data[POOL_TYPE_OFFSET] = pool_type;
    let pool = account(PROGRAM, data);
    let decoded = decode_virtual_pool(&pool).unwrap();
    assert_eq!(std::mem::size_of_val(&decoded), 416);
    assert_eq!(decoded.pool_state.config, config);
    assert_eq!(decoded.pool_state.creator, creator);
    assert_eq!(decoded.pool_state.base_mint, base);
    assert_eq!(decoded.pool_state.base_vault, base_vault);
    assert_eq!(decoded.pool_state.quote_vault, quote_vault);
    assert_eq!(decoded.pool_state.base_reserve, 1_000_001);
    assert_eq!(decoded.pool_state.quote_reserve, 2_000_003);
    assert_eq!(decoded.pool_state.sqrt_price, 1u128 << 64);
    assert_eq!(decoded.pool_state.pool_type, pool_type);
    assert_eq!(decoded.pool_state.is_migrated, 0);
    let mut data = vec![0; 8 + 1040];
    data[..8].copy_from_slice(&POOL_CONFIG_DISCRIMINATOR);
    data[8..40].copy_from_slice(quote.as_ref());
    let config_account = account(PROGRAM, data);
    let decoded = decode_pool_config(&config_account).unwrap();
    assert_eq!(std::mem::size_of_val(&decoded), 1040);
    assert_eq!(decoded.quote_mint, quote);
    let mut fixture = ReadRpc::default();
    fixture.account(id, pool.clone());
    fixture.account(config, config_account.clone());
    fixture.account(base, account(base_program, super::cpmm::mint(6)));
    fixture.account(
        quote,
        account(spl_token_interface::ID, super::cpmm::mint(9)),
    );
    (
        fixture,
        Expected {
            id,
            config,
            base,
            quote,
            base_vault,
            quote_vault,
            pool,
            config_account,
        },
    )
}

#[tokio::test]
async fn dbc_real_loader_preserves_curve_state_programs_and_instructions_sysvar() {
    for program in [spl_token_interface::ID, spl_token_2022_interface::ID] {
        let (fixture, expected) = fixture(program);
        let rpc = fixture.client();
        warmup_on_rpc(
            &rpc,
            Pool::MeteoraDbc,
            &expected.base,
            &expected.quote,
            Some(&expected.id),
        )
        .await
        .unwrap();
        let params = dex_params(
            &rpc,
            Pool::MeteoraDbc,
            &expected.base,
            &expected.quote,
            Some(&expected.id),
            None,
            None,
        )
        .await
        .unwrap();
        let DexParamEnum::MeteoraDbc(params) = params else {
            panic!("wrong DBC variant")
        };
        assert_eq!(params.pool, expected.id);
        assert_eq!(params.config, expected.config);
        assert_eq!(params.base_mint, expected.base);
        assert_eq!(params.quote_mint, expected.quote);
        assert_eq!(params.base_vault, expected.base_vault);
        assert_eq!(params.quote_vault, expected.quote_vault);
        assert_eq!(params.base_token_program, program);
        assert_eq!(params.quote_token_program, spl_token_interface::ID);
        assert_eq!(params.virtual_pool.pool_state.base_reserve, 1_000_001);
        assert_eq!(params.virtual_pool.pool_state.quote_reserve, 2_000_003);
        assert_eq!(params.virtual_pool.pool_state.sqrt_price, 1u128 << 64);
        assert_eq!(params.pool_config.quote_mint, expected.quote);
        assert!(params.include_instructions_sysvar);
        assert!(params.referral_token_account.is_none());
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 6);
    }
}

pub(super) fn shared_fixture() -> (ReadRpc, Pubkey, Pubkey, Pubkey) {
    let (read, expected) = fixture(spl_token_2022_interface::ID);
    (read, expected.id, expected.base, expected.quote)
}

pub(super) fn discovery_fixture(base: Pubkey, migrated: bool) -> (ReadRpc, Pubkey, Pubkey) {
    let (mut read, mut expected) = fixture(spl_token_2022_interface::ID);
    expected.pool.data[BASE_MINT_OFFSET..BASE_MINT_OFFSET + 32].copy_from_slice(base.as_ref());
    expected.pool.data[MIGRATED_OFFSET] = u8::from(migrated);
    read.account(expected.id, expected.pool);
    read.account(
        base,
        account(spl_token_2022_interface::ID, super::cpmm::mint(6)),
    );
    read.program_scan(PROGRAM, &[expected.id]);
    (read, expected.id, expected.quote)
}

#[tokio::test]
async fn dbc_migrated_pool_fails_before_reading_config_or_mints() {
    let (mut fixture, mut expected) = fixture(spl_token_interface::ID);
    expected.pool.data[MIGRATED_OFFSET] = 1;
    assert_eq!(
        decode_virtual_pool(&expected.pool)
            .unwrap()
            .pool_state
            .is_migrated,
        1
    );
    fixture.account(expected.id, expected.pool);
    let rpc = fixture.client();
    let error = warmup_on_rpc(
        &rpc,
        Pool::MeteoraDbc,
        &expected.base,
        &expected.quote,
        Some(&expected.id),
    )
    .await
    .unwrap_err()
    .to_string();
    assert_eq!(
        error,
        format!("Meteora DBC pool {} has migrated", expected.id)
    );
    fixture.assert_read_only();
    assert_eq!(fixture.calls().len(), 1);
}

#[tokio::test]
async fn dbc_reversed_or_unrelated_pair_cannot_use_loaded_pool() {
    for reverse in [true, false] {
        let (fixture, expected) = fixture(spl_token_interface::ID);
        let (mint, quote, message) = if reverse {
            (
                expected.quote,
                expected.base,
                "does not contain requested mint",
            )
        } else {
            (
                expected.base,
                Pubkey::new_from_array([99; 32]),
                "does not contain requested quote_mint",
            )
        };
        let rpc = fixture.client();
        assert!(
            warmup_on_rpc(&rpc, Pool::MeteoraDbc, &mint, &quote, Some(&expected.id))
                .await
                .unwrap_err()
                .to_string()
                .contains(message)
        );
        assert!(dex_params(
            &rpc,
            Pool::MeteoraDbc,
            &mint,
            &quote,
            Some(&expected.id),
            None,
            None
        )
        .await
        .err()
        .unwrap()
        .to_string()
        .contains(message));
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 6);
    }
}

#[tokio::test]
async fn dbc_pool_and_config_layout_errors_stop_at_the_affected_account() {
    for config in [false, true] {
        for damage in 0..3 {
            let (mut fixture, expected) = fixture(spl_token_interface::ID);
            let (address, mut invalid, label) = if config {
                (expected.config, expected.config_account, "PoolConfig")
            } else {
                (expected.id, expected.pool, "VirtualPool")
            };
            match damage {
                0 => invalid.owner = Pubkey::default(),
                1 => invalid.data[0] ^= 1,
                _ => {
                    invalid.data.pop();
                }
            }
            fixture.account(address, invalid);
            let error = dex_params(
                &fixture.client(),
                Pool::MeteoraDbc,
                &expected.base,
                &expected.quote,
                Some(&expected.id),
                None,
                None,
            )
            .await
            .err()
            .unwrap()
            .to_string();
            assert_eq!(
                error,
                match damage {
                    0 => format!("{label} account is not owned by Meteora DBC"),
                    1 => format!("invalid Meteora DBC {label} discriminator"),
                    _ => format!(
                        "Meteora DBC {label} account is shorter than {} bytes",
                        if config { 1040 } else { 416 }
                    ),
                }
            );
            fixture.assert_read_only();
            assert_eq!(fixture.calls().len(), if config { 2 } else { 1 });
        }
    }
}

#[tokio::test]
async fn dbc_absent_mints_keep_distinct_base_and_quote_errors() {
    for base in [true, false] {
        let (mut fixture, expected) = fixture(spl_token_interface::ID);
        fixture.absent(if base { expected.base } else { expected.quote });
        let error = MeteoraDbcParams::from_pool_address_by_rpc(&fixture.client(), &expected.id)
            .await
            .err()
            .unwrap()
            .to_string();
        assert_eq!(
            error,
            if base {
                "Meteora DBC base mint account not found"
            } else {
                "Meteora DBC quote mint account not found"
            }
        );
        fixture.assert_read_only();
        assert_eq!(fixture.calls().len(), 3);
    }
}
