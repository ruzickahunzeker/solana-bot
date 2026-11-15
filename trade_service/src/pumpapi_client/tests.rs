//! Pure Local Transaction wire/codec fixtures, with no TradingClient, signer or RPC.
use super::*;
use crate::{
    adapters::pumpapi::request::{build_request, PumpApiRequest},
    application::trade_command::test_support::command,
    config::PumpApiConfig,
    domain::trade::{ExecutionMode, Side, USD1_MINT, USDC_MINT},
};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{v0, Message, VersionedMessage},
    pubkey::Pubkey,
    signature::Signature,
};

fn config() -> PumpApiConfig {
    PumpApiConfig {
        enabled: true,
        url: "https://fixture.invalid/local".into(),
        timeout_ms: 1000,
        default_priority_fee_sol: 0.0002,
        lightning_enabled: false,
    }
}

async fn simulate_unsigned(
    rpc: &RpcClient,
    wallet: Address,
    transaction: VersionedTransaction,
) -> anyhow::Result<ExecuteResult> {
    crate::adapters::sdk::pumpapi_simulation::simulate_unsigned(
        rpc,
        wallet,
        transaction,
        &command(),
    )
    .await
}

fn trade() -> PumpApiRequest {
    let command = command();
    pumpapi_request(
        Address::from_bytes([1; 32]),
        &PumpApiTrade::new(&command, config(), true),
    )
}

fn raw_request() -> PumpApiRequest {
    PumpApiRequest {
        wallet: Address::from_bytes([1; 32]),
        side: Side::Buy,
        mint: Address::from_bytes([2; 32]),
        quote_mint: Address::from_bytes(
            crate::models::WSOL_MINT
                .parse::<Pubkey>()
                .unwrap()
                .to_bytes(),
        ),
        pool_id: None,
        amount_units: Some(1_000_000),
        sell_percent: None,
        slippage_bps: 1500,
        priority_fee_sol: 0.0002,
    }
}

pub(crate) fn transaction(versioned: bool, tail: u8) -> VersionedTransaction {
    let wallet = Pubkey::from([1; 32]);
    let instruction = Instruction {
        program_id: Pubkey::from([3; 32]),
        accounts: vec![
            AccountMeta::new(wallet, true),
            AccountMeta::new(Pubkey::from([2; 32]), false),
        ],
        data: vec![tail],
    };
    let legacy = Message::new_with_blockhash(
        &[instruction],
        Some(&wallet),
        &Hash::new_from_array([8; 32]),
    );
    let message = if versioned {
        VersionedMessage::V0(v0::Message {
            header: legacy.header,
            account_keys: legacy.account_keys,
            recent_blockhash: legacy.recent_blockhash,
            instructions: legacy.instructions,
            address_table_lookups: vec![],
        })
    } else {
        VersionedMessage::Legacy(legacy)
    };
    VersionedTransaction {
        signatures: vec![Signature::default()],
        message,
    }
}

#[test]
fn pumpapi_h1_request_wire_capture_and_known_denominations() {
    let wallet = Pubkey::from([1; 32]);
    let mut samples = vec![];
    for quote in [
        crate::models::WSOL_MINT.parse().unwrap(),
        USDC_MINT.parse().unwrap(),
        USD1_MINT.parse().unwrap(),
        Pubkey::from([4; 32]),
    ] {
        for pool in [None, Some(Pubkey::from([5; 32]))] {
            for side in [Side::Buy, Side::Sell] {
                for amount in [1u64, 1_000_000, 1_000_000_000, (1u64 << 53) + 1, u64::MAX] {
                    let mut input = trade();
                    input.quote_mint = Address::from_bytes(quote.to_bytes());
                    input.pool_id = pool.map(|pool| Address::from_bytes(pool.to_bytes()));
                    input.side = side;
                    input.amount_units = Some(amount);
                    input.sell_percent = Some(100);
                    let request = build_request(&input).unwrap();
                    assert_eq!(request["publicKey"], wallet.to_string());
                    assert_eq!(request["mint"], input.mint.to_string());
                    assert_eq!(request["quoteMint"], quote.to_string());
                    assert_eq!(request["slippage"], json!(15.0));
                    assert_eq!(request["priorityFee"], json!(0.0002));
                    if side == Side::Sell {
                        assert_eq!(request["amount"], "100%");
                        assert_eq!(request["denominatedInQuote"], "false");
                    } else {
                        assert_eq!(request["denominatedInQuote"], "true");
                    }
                    samples.push(request);
                }
            }
        }
    }
    let encoded = serde_json::to_vec(&samples).unwrap();
    assert_eq!(samples.len(), 80);
    assert_eq!(
        format!("{:x}", Sha256::digest(encoded)),
        "79eae7218b84f98ccf08c048282855ff952b057ff1701c2525dfd74b4606cd0c"
    );
    let request = build_request(&trade()).unwrap();
    assert_eq!(request["amount"], json!(0.001));
    assert_eq!(
        request,
        json!({
            "action": "buy", "amount": 0.001, "denominatedInQuote": "true",
            "mint": "8qbHbw2BbbTHBW1sbeqakYXVKRQM8Ne7pLK7m6CVfeR",
            "priorityFee": 0.0002, "publicKey": "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi",
            "quoteMint": crate::domain::trade::WSOL_MINT, "slippage": 15.0
        })
    );
}

#[test]
fn pumpapi_h1_request_rejections_keep_original_priority() {
    for (side, amount, percent, expected) in [
        (Side::Buy, None, Some(99), "buy amount is missing"),
        (
            Side::Buy,
            Some(0),
            Some(100),
            "PumpAPI buy amount must be greater than 0",
        ),
        (Side::Sell, Some(0), None, "sell_percent is missing"),
        (
            Side::Sell,
            None,
            Some(99),
            "PumpAPI first version only supports sell_percent=100",
        ),
    ] {
        let mut input = raw_request();
        input.side = side;
        input.amount_units = amount;
        input.sell_percent = percent;
        assert_eq!(build_request(&input).unwrap_err().to_string(), expected);
    }
}

#[test]
fn pumpapi_h1_legacy_and_v0_wire_capture_and_text_containers() {
    for versioned in [false, true] {
        let transaction = transaction(versioned, 7);
        let bytes = bincode::serialize(&transaction).unwrap();
        let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        assert_eq!(bytes.len(), if versioned { 206 } else { 204 });
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            if versioned {
                "72002e8a37d668eaa11dddd40f8057581a1cb9867f25a0c9e50fb1db5c3a4867"
            } else {
                "a61bd3bfbefcc156288c7ea818676bfd03affcc04e5d1a56481f1a968fadb5a8"
            }
        );
        for body in [
            bytes.clone(),
            serde_json::to_vec(&base64).unwrap(),
            serde_json::to_vec(&vec![base64]).unwrap(),
        ] {
            let decoded = decode_pumpapi_transaction(&body).unwrap();
            assert_eq!(bincode::serialize(&decoded).unwrap(), bytes);
            assert!(decoded
                .signatures
                .iter()
                .all(|signature| *signature == Signature::default()));
        }
    }
}

#[test]
fn pumpapi_h1_binary_trailing_ascii_whitespace_is_transaction_data_not_http_padding() {
    for tail in [b' ', b'\t', b'\n', b'\r', 0x0b, 0x0c] {
        let original = transaction(false, tail);
        let bytes = bincode::serialize(&original).unwrap();
        assert_eq!(bytes.last(), Some(&tail));
        let decoded = decode_pumpapi_transaction(&bytes)
            .expect("binary transaction bytes must not be trimmed");
        assert_eq!(decoded, original);
    }
}

fn envelopes(transaction: &VersionedTransaction) -> [Vec<u8>; 3] {
    let bytes = bincode::serialize(transaction).unwrap();
    let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    [
        bytes,
        serde_json::to_vec(&base64).unwrap(),
        serde_json::to_vec(&vec![base64]).unwrap(),
    ]
}

#[test]
fn pumpapi_h1_text_whitespace_does_not_become_binary_padding_permission() {
    for versioned in [false, true] {
        let expected = transaction(versioned, b'\n');
        for body in envelopes(&expected).into_iter().skip(1) {
            let padded = [b" \r\n\t".as_slice(), &body, b"\n\r \t"].concat();
            assert_eq!(decode_pumpapi_transaction(&padded).unwrap(), expected);
        }
        let bytes = bincode::serialize(&expected).unwrap();
        for trailing in [b"\0".as_slice(), b" ", b"\r\n", &bytes] {
            let padded = [&bytes, trailing].concat();
            assert!(decode_pumpapi_transaction(&padded).is_err());
            let text =
                serde_json::to_vec(&base64::engine::general_purpose::STANDARD.encode(padded))
                    .unwrap();
            assert!(decode_pumpapi_transaction(&text).is_err());
        }
        for leading in [b" ".as_slice(), b"\r\n"] {
            assert!(decode_pumpapi_transaction(&[leading, &bytes].concat()).is_err());
        }
    }
}

#[test]
fn pumpapi_h1_empty_malformed_and_multiple_containers_are_rejected() {
    for body in [b"".as_slice(), b" \r\n\t", b"\x0c"] {
        assert_eq!(
            decode_pumpapi_transaction(body).unwrap_err().to_string(),
            "PumpAPI returned an empty transaction response"
        );
    }
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(bincode::serialize(&transaction(false, 7)).unwrap());
    for count in [0, 2, 3] {
        let body = serde_json::to_vec(&vec![&encoded; count]).unwrap();
        assert_eq!(
            decode_pumpapi_transaction(&body).unwrap_err().to_string(),
            format!("PumpAPI returned {count} transactions; first version supports exactly one")
        );
    }
    for body in [
        b"null".as_slice(),
        b"\x0b\x0c",
        b"true",
        b"42",
        b"[] extra",
        b"[null]",
        b"[1]",
        b"[[]]",
        b"\"%%%\"",
        b"\"\"",
        b"\"abc",
        b"\"abc\" \"abc\"",
        b"[\"abc\"]",
        b"\"\xff\"",
        b"{",
        b"{\"error\":",
    ] {
        assert!(decode_pumpapi_transaction(body).is_err(), "body={body:?}");
    }
}

#[test]
fn pumpapi_h1_json_error_object_is_not_a_transaction_and_redacts_urls() {
    for error in [
        json!({"error": "RPC failed at https://fixture.invalid/secret?key=private"}),
        json!({"result": {"transaction": "not-supported", "url": "http://fixture.invalid/token"}}),
    ] {
        let body = serde_json::to_vec(&error).unwrap();
        let error = format!("{:#}", decode_pumpapi_transaction(&body).unwrap_err());
        assert!(error.contains("JSON instead of transaction bytes"));
        assert!(error.contains("<redacted-url>"));
        assert!(!error.contains("fixture.invalid"));
        assert!(!error.contains("secret"));
        assert!(!error.contains("private"));
    }
}

#[test]
fn pumpapi_h1_all_truncated_packets_fail_without_signing_or_rpc() {
    for versioned in [false, true] {
        let bytes = bincode::serialize(&transaction(versioned, 7)).unwrap();
        for end in 0..bytes.len() {
            assert!(
                decode_pumpapi_transaction(&bytes[..end]).is_err(),
                "end={end}"
            );
        }
    }
    for body in [vec![0xff; 1232], vec![0x80; 1232], vec![0; 1232]] {
        assert!(decode_pumpapi_transaction(&body).is_err());
    }
}

#[test]
fn pumpapi_h1_packet_and_envelope_exact_limits_are_enforced() {
    use crate::adapters::pumpapi::response::{MAX_RESPONSE_BYTES, MAX_TRANSACTION_BYTES};

    for versioned in [false, true] {
        let mut tx = transaction(versioned, 7);
        let mut bytes = Vec::new();
        for len in 0..MAX_TRANSACTION_BYTES {
            match &mut tx.message {
                VersionedMessage::Legacy(message) => message.instructions[0].data = vec![7; len],
                VersionedMessage::V0(message) => message.instructions[0].data = vec![7; len],
                _ => unreachable!(),
            }
            bytes = bincode::serialize(&tx).unwrap();
            if bytes.len() == MAX_TRANSACTION_BYTES {
                break;
            }
        }
        assert_eq!(bytes.len(), MAX_TRANSACTION_BYTES);
        for body in envelopes(&tx) {
            assert_eq!(decode_pumpapi_transaction(&body).unwrap(), tx);
        }
        match &mut tx.message {
            VersionedMessage::Legacy(message) => message.instructions[0].data.push(7),
            VersionedMessage::V0(message) => message.instructions[0].data.push(7),
            _ => unreachable!(),
        }
        assert_eq!(
            bincode::serialized_size(&tx).unwrap(),
            MAX_TRANSACTION_BYTES as u64 + 1
        );
        for body in envelopes(&tx) {
            assert_eq!(
                decode_pumpapi_transaction(&body).unwrap_err().to_string(),
                "PumpAPI transaction exceeds 1232 bytes"
            );
        }
    }
    let text = envelopes(&transaction(false, 7))[1].clone();
    let mut padded = vec![b' '; MAX_RESPONSE_BYTES - text.len()];
    padded.extend(text);
    assert!(decode_pumpapi_transaction(&padded).is_ok());
    padded.push(b' ');
    assert_eq!(
        decode_pumpapi_transaction(&padded).unwrap_err().to_string(),
        "PumpAPI transaction response exceeds 16384 bytes"
    );
    let oversized_base64 = serde_json::to_vec(&"A".repeat(1645)).unwrap();
    assert_eq!(
        decode_pumpapi_transaction(&oversized_base64)
            .unwrap_err()
            .to_string(),
        "PumpAPI base64 transaction exceeds 1644 bytes"
    );
}

#[test]
fn pumpapi_h1_invalid_signature_vectors_and_account_indices_fail_sanitization() {
    for mutation in 0..6 {
        let mut tx = transaction(false, 7);
        let VersionedMessage::Legacy(message) = &mut tx.message else {
            unreachable!()
        };
        match mutation {
            0 => tx.signatures.clear(),
            1 => tx.signatures.push(Signature::default()),
            2 => message.header.num_required_signatures = 4,
            3 => message.instructions[0].program_id_index = 3,
            4 => message.instructions[0].accounts.push(3),
            5 => message.header.num_readonly_signed_accounts = 1,
            _ => unreachable!(),
        }
        for body in envelopes(&tx) {
            assert_eq!(
                decode_pumpapi_transaction(&body).unwrap_err().to_string(),
                "PumpAPI transaction failed Solana sanitization"
            );
        }
    }
}

#[test]
fn pumpapi_h1_v1_and_unknown_versions_are_not_implicitly_enabled() {
    let tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::V1(solana_sdk::message::v1::Message::default()),
    };
    for body in envelopes(&tx) {
        assert_eq!(
            decode_pumpapi_transaction(&body).unwrap_err().to_string(),
            "PumpAPI message version is not supported for account audit"
        );
    }
    let mut bytes = bincode::serialize(&transaction(true, 7)).unwrap();
    assert_eq!(bytes[65], 0x80);
    for prefix in [0x82, 0x83, 0xff] {
        bytes[65] = prefix;
        assert!(decode_pumpapi_transaction(&bytes).is_err());
    }
}

#[test]
fn pumpapi_h1_v0_lookup_bytes_are_preserved_but_not_resolved_or_authorized() {
    let mut tx = transaction(true, 7);
    let VersionedMessage::V0(message) = &mut tx.message else {
        unreachable!()
    };
    message
        .address_table_lookups
        .push(v0::MessageAddressTableLookup {
            account_key: Pubkey::from([9; 32]),
            writable_indexes: vec![1],
            readonly_indexes: vec![2],
        });
    message.instructions[0].accounts.extend([3, 4]);
    for body in envelopes(&tx) {
        assert_eq!(decode_pumpapi_transaction(&body).unwrap(), tx);
    }
    // Structural acceptance does not establish signer, program or spend authorization.
    tx.signatures[0] = Signature::from([9; 64]);
    for body in envelopes(&tx) {
        assert_eq!(decode_pumpapi_transaction(&body).unwrap(), tx);
    }
}

#[test]
fn pumpapi_h1_request_mapping_excludes_runtime_and_configuration_fields() {
    let wallet = Pubkey::from([1; 32]);
    let mut command = command();
    let mut config = config();
    let before = PumpApiTrade::new(&command, config.clone(), true);
    let original = build_request(&pumpapi_request(
        Address::from_bytes(wallet.to_bytes()),
        &before,
    ))
    .unwrap();
    config.url = "https://fixture.invalid/private?token=secret".into();
    config.timeout_ms = u64::MAX;
    config.default_priority_fee_sol = 1.0;
    config.enabled = false;
    config.lightning_enabled = true;
    command.execution.mode = ExecutionMode::Simulate;
    let input = PumpApiTrade::new(&command, config, false);
    let request = pumpapi_request(Address::from_bytes(wallet.to_bytes()), &input);
    assert_eq!(build_request(&request).unwrap(), original);
    assert_eq!(request.wallet.to_bytes(), wallet.to_bytes());
    assert_eq!(request.mint, command.route.mint);
    assert_eq!(request.quote_mint, command.quote_mint());
    assert_eq!(request.pool_id, None);
    assert_eq!(request.amount_units, command.amount.buy_units());
    assert_eq!(request.sell_percent, command.amount.sell_percent());
    assert_eq!(request.slippage_bps, command.execution.slippage_bps);
    assert_eq!(request.priority_fee_sol, command.pumpapi_priority_fee_sol);
}

mod simulation;
