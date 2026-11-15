use super::*;
use crate::api::dto::sol_buy::{BoundSolBuyQuote, SolBuyQuoteRequest, SolBuyQuoteResponse};
use std::cell::Cell;

fn preview() -> SolBuyQuoteResponse {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/contracts/sol_buy_v1.json"
    ))
    .unwrap();
    serde_json::from_value::<BoundQuote>(fixture["bound"].clone())
        .unwrap()
        .quote
        .into()
}
fn request() -> SolBuyQuoteRequest {
    SolBuyQuoteRequest {
        wallet_id: " literal wallet ".into(),
        mint: "request-mint-is-not-used-by-cache".into(),
        amount_lamports: 42,
        slippage_bps: 1500,
    }
}

#[test]
fn quote_store_v1_binding_bytes_baseline() {
    use sha2::{Digest, Sha256};
    let mut rows = vec![];
    for wallet in ["", "literal-wallet", " wallet ", "another-wallet"] {
        for pubkey in ["", "literal-public-key", " public key "] {
            for slippage in [0, 1500, u16::MAX] {
                for units in [0, 9_007_199_254_740_993, u64::MAX] {
                    let store = QuoteStore::default();
                    let request = SolBuyQuoteRequest {
                        wallet_id: wallet.into(),
                        slippage_bps: slippage,
                        ..request()
                    };
                    let mut quote = preview();
                    quote.amount_lamports = units;
                    let id = format!("fixed-quote-{}", rows.len());
                    let now = 1_000_000;
                    let bound =
                        insert_at(&store, &request, pubkey.into(), quote, now, || id.clone())
                            .unwrap();
                    assert_eq!(bound.expires_at_ms, now + 15_000);
                    assert_eq!(bound.wallet_id, wallet);
                    assert_eq!(bound.wallet_pubkey, pubkey);
                    assert_eq!(bound.quote_slippage_bps, slippage);
                    assert_eq!(bound.quote.amount_lamports, units);
                    assert_ne!(bound.quote.output_mint, request.mint);
                    let encoded = serde_json::to_string(&bound).unwrap();
                    let mut read = get_at(&store, &id, wallet, pubkey, || now).unwrap();
                    assert_eq!(serde_json::to_string(&read).unwrap(), encoded);
                    read.quote.expected_output_units = 0;
                    assert_eq!(
                        serde_json::to_string(
                            &get_at(&store, &id, wallet, pubkey, || now).unwrap()
                        )
                        .unwrap(),
                        encoded
                    );
                    rows.push(encoded);
                }
            }
        }
    }
    assert_eq!(rows.len(), 108);
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&rows).unwrap())),
        "e543fd29e04c691f13cccd17a8afe094a9881f71b7a6cc6ec6b81348a06d94d3"
    );
}

#[test]
fn quote_store_expiry_precedes_wallet_mismatch_and_missing_never_reads_clock() {
    let store = QuoteStore::default();
    let request = request();
    let bound = insert_at(&store, &request, "payer".into(), preview(), 100, || {
        "expiry-quote".into()
    })
    .unwrap();
    let clock_calls = Cell::new(0);
    let now = |value| {
        clock_calls.set(clock_calls.get() + 1);
        value
    };
    for (wallet, payer) in [("other", "payer"), (request.wallet_id.as_str(), "other")] {
        assert_eq!(
            get_at(&store, &bound.quote_id, wallet, payer, || now(bound
                .expires_at_ms
                - 1))
            .unwrap_err()
            .to_string(),
            "quote wallet mismatch"
        );
        assert_eq!(store.quotes.len(), 1);
    }
    assert!(get_at(
        &store,
        &bound.quote_id,
        &request.wallet_id,
        "payer",
        || now(bound.expires_at_ms - 1)
    )
    .is_ok());
    assert_eq!(
        get_at(&store, &bound.quote_id, "other", "other", || now(
            bound.expires_at_ms
        ))
        .unwrap_err()
        .to_string(),
        "quote expired"
    );
    assert!(store.quotes.is_empty());
    let before = clock_calls.get();
    assert_eq!(
        get_at(
            &store,
            &bound.quote_id,
            &request.wallet_id,
            "payer",
            || now(0)
        )
        .unwrap_err()
        .to_string(),
        "quote not found or service restarted"
    );
    assert_eq!(clock_calls.get(), before);
}

#[test]
fn quote_store_capacity_prunes_expired_before_limit_and_id_generation() {
    let store = QuoteStore::default();
    let request = request();
    for index in 0..512 {
        insert_at(&store, &request, "payer".into(), preview(), 100, || {
            format!("capacity-{index}")
        })
        .unwrap();
    }
    let ids = Cell::new(0);
    let new_id = || {
        ids.set(ids.get() + 1);
        "replacement".into()
    };
    assert_eq!(
        insert_at(&store, &request, "payer".into(), preview(), 100, new_id)
            .unwrap_err()
            .to_string(),
        "quote capacity reached; retry after active quotes expire"
    );
    assert_eq!(store.quotes.len(), 512);
    assert_eq!(ids.get(), 0);
    store.quotes.get_mut("capacity-0").unwrap().expires_at_ms = 100;
    store.quotes.get_mut("capacity-1").unwrap().expires_at_ms = 99;
    let bound = insert_at(&store, &request, "payer".into(), preview(), 100, new_id).unwrap();
    assert_eq!(bound.quote_id, "replacement");
    assert_eq!(ids.get(), 1);
    assert_eq!(store.quotes.len(), 511);
    assert!(!store.quotes.contains_key("capacity-0"));
    assert!(!store.quotes.contains_key("capacity-1"));
    assert!(store.quotes.contains_key("capacity-2"));
}

#[test]
fn quote_store_keeps_distinct_ids_and_does_not_revalidate_or_deduplicate_requests() {
    let store = QuoteStore::default();
    let request = SolBuyQuoteRequest {
        wallet_id: "".into(),
        slippage_bps: u16::MAX,
        ..request()
    };
    let quote = preview();
    let first = store.insert(&request, "".into(), quote.clone()).unwrap();
    let second = store.insert(&request, "".into(), quote).unwrap();
    assert_ne!(first.quote_id, second.quote_id);
    assert_eq!(
        Uuid::parse_str(&first.quote_id).unwrap().get_version_num(),
        4
    );
    assert_eq!(
        Uuid::parse_str(&second.quote_id).unwrap().get_version_num(),
        4
    );
    assert_eq!(store.quotes.len(), 2);
    assert!(store.get(&first.quote_id, "", "").is_ok());
    assert!(store.get(&second.quote_id, "", "").is_ok());
}

fn insert_at(
    store: &QuoteStore,
    request: &SolBuyQuoteRequest,
    wallet_pubkey: String,
    quote: SolBuyQuoteResponse,
    now: u64,
    new_id: impl FnOnce() -> String,
) -> Result<BoundSolBuyQuote> {
    store
        .bind_at(
            QuoteBinding {
                wallet_id: &request.wallet_id,
                wallet_pubkey,
                slippage_bps: request.slippage_bps,
            },
            quote.into(),
            now,
            new_id,
        )
        .map(Into::into)
}

fn get_at(
    store: &QuoteStore,
    quote_id: &str,
    wallet_id: &str,
    wallet_pubkey: &str,
    now: impl FnOnce() -> u64,
) -> Result<BoundSolBuyQuote> {
    store
        .read_at(quote_id, wallet_id, wallet_pubkey, now)
        .map(Into::into)
}

#[test]
fn bound_quote_rejects_other_wallet_and_expiry() {
    let store = QuoteStore::default();
    let request = SolBuyQuoteRequest {
        wallet_id: "2TE".into(),
        ..request()
    };
    let bound = store
        .insert(&request, "wallet-pubkey".into(), preview())
        .unwrap();
    assert!(store.get(&bound.quote_id, "2TE", "wallet-pubkey").is_ok());
    assert!(store
        .get(&bound.quote_id, "other", "wallet-pubkey")
        .is_err());
    assert!(store.get(&bound.quote_id, "2TE", "other-pubkey").is_err());
    store.quotes.get_mut(&bound.quote_id).unwrap().expires_at_ms = 0;
    assert!(store.get(&bound.quote_id, "2TE", "wallet-pubkey").is_err());
}

#[test]
fn native_quote_cache_port_keeps_complete_integer_estimate_and_binding_identity() {
    let store = QuoteStore::default();
    let cache: &dyn QuoteCache = &store;
    let quote: QuoteEstimate = preview().into();
    let expected = quote.clone();
    let bound = cache
        .bind(
            QuoteBinding {
                wallet_id: " literal wallet ",
                wallet_pubkey: "literal public key".into(),
                slippage_bps: 1234,
            },
            quote,
        )
        .unwrap();
    assert_eq!(bound.quote, expected);
    assert_eq!(bound.quote.amount_lamports, 9_007_199_254_740_993);
    assert_eq!(bound.quote.expected_output_units, u64::MAX);
    assert_eq!(bound.wallet_id, " literal wallet ");
    assert_eq!(bound.wallet_pubkey, "literal public key");
    assert_eq!(bound.quote_slippage_bps, 1234);
    assert_eq!(
        cache
            .read(&bound.quote_id, &bound.wallet_id, &bound.wallet_pubkey)
            .unwrap(),
        bound
    );
}

#[test]
fn native_quote_cache_port_and_legacy_bridge_share_the_same_entries_and_wire_bytes() {
    let store = QuoteStore::default();
    let request = request();
    let first = store.insert(&request, "payer".into(), preview()).unwrap();
    let internal = store
        .read(&first.quote_id, &request.wallet_id, "payer")
        .unwrap();
    assert_eq!(
        serde_json::to_string(&internal).unwrap(),
        serde_json::to_string(&first).unwrap()
    );
    assert_eq!(store.quotes.len(), 1);
    let second = store
        .bind(
            QuoteBinding {
                wallet_id: &request.wallet_id,
                wallet_pubkey: "payer".into(),
                slippage_bps: request.slippage_bps,
            },
            preview().into(),
        )
        .unwrap();
    let wire = store
        .get(&second.quote_id, &request.wallet_id, "payer")
        .unwrap();
    assert_eq!(
        serde_json::to_string(&wire).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert_ne!(first.quote_id, second.quote_id);
    assert_eq!(store.quotes.len(), 2);
}
