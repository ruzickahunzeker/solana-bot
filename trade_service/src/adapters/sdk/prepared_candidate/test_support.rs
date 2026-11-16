use crate::{
    adapters::sdk::damm_probe::test_support::buy_record_fixture_for_wallet,
    domain::{
        sdk_prepared_record::SdkPreparedRecord, submission::SignedSubmission, trade::Address,
    },
    models::TradeRequest,
};
use solana_sdk::{
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};

pub(crate) struct Fixture {
    pub record: SdkPreparedRecord,
    pub message: Vec<u8>,
    pub key: Keypair,
    pub request: TradeRequest,
}

impl Fixture {
    /// Synthetic historical evidence using a temporary public-test key, not the controlled signer.
    pub(crate) fn signed(&self) -> SignedSubmission {
        let transaction = VersionedTransaction::try_new(
            bincode::deserialize::<solana_sdk::message::VersionedMessage>(&self.message).unwrap(),
            &[&self.key],
        )
        .unwrap();
        SignedSubmission {
            signature: transaction.signatures[0].to_string(),
            bytes: bincode::serialize(&transaction).unwrap(),
        }
    }
}

pub(crate) async fn buy(native: bool) -> Fixture {
    let key = Keypair::new();
    let (record, message) = buy_record_fixture_for_wallet(key.pubkey(), native).await;
    let d = record.details();
    let mut value = serde_json::json!({
        "wallet_id":"fixture-wallet", "side":"buy", "pool":"meteora-damm-v2",
        "mint":Address::from_bytes(d.mint).to_string(),
        "quote_mint":Address::from_bytes(d.quote_mint).to_string(),
        "pool_id":d.pool_hint.map(|v| Address::from_bytes(v).to_string()),
        "simulate":false, "execution_method":"sdk", "idempotency_key":" sdk-buy-historical-literal ",
        "slippage_bps":d.slippage_bps, "tip_sol":f64::from_bits(d.tip_sol_bits),
        "use_mev":d.use_mev, "mev_provider":d.requested_mev_provider,
    });
    value[if native {
        "amount_lamports"
    } else {
        "amount_quote_units"
    }] = serde_json::json!(d.input_units);
    Fixture {
        record: SdkPreparedRecord::DammBuy(record),
        message,
        key,
        request: serde_json::from_value(value).unwrap(),
    }
}
