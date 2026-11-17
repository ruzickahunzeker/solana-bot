use super::*;
use crate::adapters::rpc::read_runtime::aggregator_accounts::tests::{
    address, runtime, ui, urls, Mode,
};
use solana_address_lookup_table_interface::state::LookupTableMeta;
use std::borrow::Cow;

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn table(deactivation: u64, addresses: Vec<Pubkey>) -> Vec<u8> {
    AddressLookupTable {
        meta: LookupTableMeta {
            deactivation_slot: deactivation,
            ..Default::default()
        },
        addresses: Cow::Owned(addresses),
    }
    .serialize_for_tests()
    .unwrap()
}

#[tokio::test]
async fn aggregator_account_loader_both_token_programs_keep_original_wallet_bound_ata() {
    for program in [spl_token::ID, spl_token_2022::ID] {
        let r = runtime(ui(program, &[7, 8, 9]), Mode::Good);
        for wallet in [key(1), key(2)] {
            assert_eq!(
                target_ata(
                    &r,
                    &urls()[0],
                    CommitmentConfig::confirmed(),
                    wallet,
                    key(3)
                )
                .await
                .unwrap(),
                (
                    get_associated_token_address_with_program_id_fast(&wallet, &key(3), &program),
                    program
                )
            );
        }
        let calls = r.test_transport().calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls
            .iter()
            .all(|(_, body, _)| body["method"] == "getAccountInfo"
                && body["params"][0] == address(3).to_string()
                && body["params"][1]["commitment"] == "confirmed"));
    }
}

#[tokio::test]
async fn aggregator_account_loader_unsupported_owner_keeps_original_mint_policy() {
    let r = runtime(ui(key(9), &[]), Mode::Good);
    assert_eq!(
        target_ata(
            &r,
            &urls()[0],
            CommitmentConfig::processed(),
            key(1),
            key(3)
        )
        .await
        .unwrap_err()
        .to_string(),
        "aggregator target mint is not owned by a supported token program"
    );
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn aggregator_account_loader_active_lookup_keeps_sdk_addresses_order_and_empty_tables() {
    for expected in [vec![], (0..=255).map(|n| key(n)).collect::<Vec<_>>()] {
        let data = table(u64::MAX, expected.clone());
        let old = AddressLookupTable::deserialize(&data)
            .unwrap()
            .addresses
            .into_owned();
        let r = runtime(ui(lookup_program::id(), &data), Mode::Good);
        let actual = lookup_table(&r, &urls()[0], CommitmentConfig::finalized(), key(4))
            .await
            .unwrap();
        assert_eq!(actual, old);
        assert_eq!(actual, expected);
        let calls = r.test_transport().calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1["params"][0], address(4).to_string());
        assert_eq!(calls[0].1["params"][1]["commitment"], "finalized");
    }
}

#[tokio::test]
async fn aggregator_account_loader_bad_lookup_owner_state_or_decoder_never_produces_keys() {
    for (owner, data, expected) in [
        (
            key(9),
            table(u64::MAX, vec![key(5)]),
            "aggregator lookup table has invalid owner",
        ),
        (
            lookup_program::id(),
            vec![1, 2, 3],
            "invalid aggregator lookup table data",
        ),
        (
            lookup_program::id(),
            table(42, vec![key(5)]),
            "aggregator lookup table is deactivating or deactivated",
        ),
    ] {
        let r = runtime(ui(owner, &data), Mode::Good);
        assert_eq!(
            lookup_table(&r, &urls()[0], CommitmentConfig::processed(), key(4))
                .await
                .unwrap_err()
                .to_string(),
            expected
        );
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn aggregator_account_loader_missing_or_unavailable_evidence_has_no_cached_or_default_result()
{
    for mode in [Mode::Good, Mode::Error] {
        let r = runtime(serde_json::Value::Null, mode);
        assert!(target_ata(
            &r,
            &urls()[0],
            CommitmentConfig::processed(),
            key(1),
            key(3)
        )
        .await
        .is_err());
        assert!(
            lookup_table(&r, &urls()[0], CommitmentConfig::processed(), key(4))
                .await
                .is_err()
        );
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 2);
    }
}
