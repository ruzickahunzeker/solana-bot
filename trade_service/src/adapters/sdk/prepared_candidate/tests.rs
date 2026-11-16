use super::test_support::buy;
use super::*;
use crate::adapters::sdk::damm_sell_message::{
    candidate_fixture, validate_signed_candidate as validate_sell,
};
use crate::domain::sdk_buy_candidate::DammBuyCandidateRecord;
use sol_trade_sdk::swqos::SwqosType;
use solana_sdk::{
    message::VersionedMessage,
    signature::{Keypair, Signer},
};

#[tokio::test]
async fn sdk_buy_evidence_reconstructs_both_funding_assets_and_verifies_original_signature_bytes() {
    for native in [false, true] {
        let f = buy(native).await;
        validate_candidate(&f.record, &f.message).unwrap();
        let signed = f.signed();
        validate_signed_candidate(&f.record, &f.message, &signed).unwrap();
        assert!(validate_sell(&f.record, &f.message, &signed).is_err());
        let json = f.record.encode(&f.message).unwrap();
        let decoded = SdkPreparedRecord::decode(&json, &f.message).unwrap();
        validate_signed_candidate(&decoded, &f.message, &signed).unwrap();
        assert_eq!(decoded.wallet(), f.key.pubkey().to_bytes());
    }
}

#[tokio::test]
async fn sdk_buy_evidence_rejects_partial_zero_wrong_wallet_signature_and_noncanonical_packets() {
    let f = buy(true).await;
    let original = f.signed();
    let tx: VersionedTransaction = bincode::deserialize(&original.bytes).unwrap();
    for change in 0..9 {
        let mut tx = tx.clone();
        let mut signed = SignedSubmission {
            signature: original.signature.clone(),
            bytes: original.bytes.clone(),
        };
        match change {
            0 => signed.bytes.clear(),
            1 => signed.bytes = vec![0; 1233],
            2 => signed.bytes.push(0),
            3 => signed.signature = Keypair::new().sign_message(&f.message).to_string(),
            4 => tx.signatures[0] = Signature::default(),
            5 => tx.signatures[0] = Keypair::new().sign_message(&f.message),
            6 => tx.signatures.clear(),
            7 => tx.signatures.push(tx.signatures[0]),
            _ => {
                let VersionedMessage::Legacy(message) = &mut tx.message else {
                    panic!("fixed Legacy fixture")
                };
                message.recent_blockhash = solana_sdk::hash::Hash::new_unique();
            }
        }
        if change >= 4 {
            signed.bytes = bincode::serialize(&tx).unwrap();
        }
        assert!(
            validate_signed_candidate(&f.record, &f.message, &signed).is_err(),
            "{change}"
        );
    }
    validate_signed_candidate(&f.record, &f.message, &original).unwrap();
}

#[tokio::test]
async fn sdk_buy_evidence_rebuilds_protocol_instead_of_accepting_a_matching_tampered_digest() {
    let f = buy(true).await;
    let tx: VersionedTransaction = bincode::deserialize(&f.signed().bytes).unwrap();
    for change in 0..5 {
        let mut message = tx.message.clone();
        let VersionedMessage::Legacy(m) = &mut message else {
            panic!("fixed Legacy fixture")
        };
        match change {
            0 => m.header.num_readonly_unsigned_accounts = 0,
            1 => m.instructions.last_mut().unwrap().data[8] ^= 1,
            2 => m.instructions.last_mut().unwrap().accounts[0] ^= 1,
            3 => m.account_keys[0] = Keypair::new().pubkey(),
            _ => m.recent_blockhash = solana_sdk::hash::Hash::new_unique(),
        }
        let bytes = message.serialize();
        let record = SdkPreparedRecord::DammBuy(
            DammBuyCandidateRecord::new(f.record.buy().unwrap().details().clone(), &bytes).unwrap(),
        );
        record.validate(&bytes).unwrap();
        assert!(validate_candidate(&record, &bytes).is_err(), "{change}");
    }
}

#[tokio::test]
async fn sdk_buy_evidence_wrong_direction_record_cannot_validate_original_sell_packet() {
    let f = buy(false).await;
    let sell = candidate_fixture(SwqosType::Default, false, 100);
    assert!(validate_candidate(sell.metadata(), &f.message).is_err());
    assert!(validate_candidate(&f.record, &sell.message_bytes()).is_err());
    assert!(
        validate_signed_candidate(sell.metadata(), &sell.message_bytes(), &f.signed()).is_err()
    );
}

#[test]
fn sdk_buy_evidence_shared_sell_validator_keeps_existing_provider_mode_percentage_matrix() {
    for provider in [SwqosType::Default, SwqosType::Temporal] {
        for simulate in [false, true] {
            for percent in [1, 50, 100] {
                let candidate = candidate_fixture(provider, simulate, percent);
                validate_candidate(candidate.metadata(), &candidate.message_bytes()).unwrap();
            }
        }
    }
}
