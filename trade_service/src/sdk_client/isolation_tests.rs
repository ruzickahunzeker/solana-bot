use super::*;
use crate::adapters::sdk::quote_submission::{PreparedQuoteTransaction, SdkQuoteSubmission};
use crate::application::ports::quote_execution::QuoteSubmissionBackend;
use crate::domain::execution_policy::BROADCAST_DISABLED_ERROR;

fn client() -> SdkTradeClient {
    // No payer, SDK client, RPC or transport exists; a guard regression fails before networking.
    SdkTradeClient {
        lanes: vec![],
        sol_hop_cache: Mutex::new(HashMap::new()),
        swqos_lanes: vec![],
        mev_protection: false,
        require_mev_submit: false,
        rpc_count: 0,
        broadcast_policy: BroadcastPolicy::Disabled,
    }
}

fn trade(simulate: bool) -> ExecuteTrade {
    ExecuteTrade {
        pool: Pool::Pump,
        mint: Pubkey::default(),
        quote_mint: Pubkey::default(),
        pool_id: None,
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        amount_lamports: Some(1),
        sell_percent: Some(100),
        slippage_bps: 100,
        tip_sol: 0.0,
        recent_blockhash: Hash::default(),
        wait_tx_confirmed: false,
        mev_submit_verify_ms: 0,
        simulate,
        use_mev: Some(true),
        mev_provider: Some(MevProvider::Temporal),
    }
}

fn transaction() -> VersionedTransaction {
    VersionedTransaction {
        signatures: vec![],
        message: solana_sdk::message::VersionedMessage::Legacy(
            solana_sdk::message::Message::default(),
        ),
    }
}

#[test]
fn wsol_signing_is_blocked_before_wallet_or_ephemeral_signer_access() {
    assert_eq!(
        client()
            .sign_prepared_wsol(
                crate::conversion::UnsignedWsolTransaction::unverified_for_policy_test(),
                &crate::application::wsol_submission_types::WsolQuoteSnapshot::from(
                    &crate::conversion::contract_tests::quote()
                )
            )
            .err()
            .unwrap()
            .to_string(),
        BROADCAST_DISABLED_ERROR
    );
}

#[tokio::test]
async fn sdk_buy_and_sell_are_blocked_even_with_simulate_and_mev_overrides() {
    let client = client();
    for simulate in [false, true] {
        assert_eq!(
            client
                .buy(trade(simulate), || panic!(
                    "disabled SDK must not acquire shared RPC runtime"
                ))
                .await
                .err()
                .unwrap()
                .to_string(),
            BROADCAST_DISABLED_ERROR
        );
        assert_eq!(
            client
                .sell(trade(simulate), || panic!(
                    "disabled SDK must not acquire shared RPC runtime"
                ))
                .await
                .err()
                .unwrap()
                .to_string(),
            BROADCAST_DISABLED_ERROR
        );
    }
}

#[tokio::test]
async fn aggregator_send_and_adapter_sign_are_blocked_without_a_signer() {
    let client = client();
    let read_rpc = crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap();
    assert_eq!(
        client
            .send_aggregator_transaction(&transaction())
            .await
            .err()
            .unwrap()
            .to_string(),
        BROADCAST_DISABLED_ERROR
    );
    assert_eq!(
        SdkQuoteSubmission(&client, &read_rpc)
            .sign(PreparedQuoteTransaction::unverified_for_policy_test(
                transaction()
            ))
            .err()
            .unwrap()
            .to_string(),
        BROADCAST_DISABLED_ERROR
    );
}

#[tokio::test]
async fn ordinary_scan_observed_quote_preserves_sign_and_send_guards_without_sdk_or_rpc() {
    use crate::adapters::{
        rpc::ordinary_observation::SingleShotStatusReader, sdk::observed_quote::ObservedQuote,
    };
    use crate::application::ports::reconciliation::SignatureStatusReader;
    let client = client();
    let reader = SingleShotStatusReader::new(
        &["https://node.example.invalid".into()],
        Duration::from_secs(1),
    )
    .unwrap();
    let read_rpc = crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap();
    let backend = ObservedQuote::new(&client, &reader, &read_rpc);
    assert_eq!(
        backend.ensure_live_allowed().unwrap_err().to_string(),
        BROADCAST_DISABLED_ERROR
    );
    assert_eq!(
        backend
            .sign(PreparedQuoteTransaction::unverified_for_policy_test(
                transaction()
            ))
            .err()
            .unwrap()
            .to_string(),
        BROADCAST_DISABLED_ERROR
    );
    let mut unsigned = transaction();
    unsigned.signatures.push(Signature::default());
    let signed = crate::domain::submission::SignedSubmission {
        signature: Signature::default().to_string(),
        bytes: bincode::serialize(&unsigned).unwrap(),
    };
    assert_eq!(
        backend.broadcast(&signed).await.err().unwrap().to_string(),
        BROADCAST_DISABLED_ERROR
    );
    assert_eq!(
        backend
            .lookup_signature("invalid")
            .await
            .err()
            .unwrap()
            .to_string(),
        "invalid persisted transaction signature"
    );
}

#[tokio::test]
async fn sol_transfer_is_blocked_before_lane_and_amount_resolution() {
    let client = client();
    let read_rpc = crate::adapters::rpc::read_runtime::SharedReadRuntime::new(&[]).unwrap();
    for (amount, percent) in [(Some(1), None), (None, Some(100))] {
        assert_eq!(
            client
                .prepare_sol_transfer(&read_rpc, &Pubkey::default(), amount, percent)
                .await
                .err()
                .unwrap()
                .to_string(),
            BROADCAST_DISABLED_ERROR
        );
    }
}

fn pumpapi_trade(simulate: bool) -> PumpApiTrade {
    let mut command = crate::application::trade_command::test_support::command();
    command.route.mint = Address::from_bytes([0; 32]);
    command.route.quote_mint = Address::from_bytes([0; 32]);
    command.amount = crate::domain::trade::TradeAmount::BuySol { lamports: 1 };
    command.execution.slippage_bps = 100;
    command.execution.mode = crate::domain::trade::ExecutionMode::resolve(Some(simulate), false);
    command.pumpapi_priority_fee_sol = 0.0;
    PumpApiTrade::new(
        &command,
        crate::config::PumpApiConfig {
            enabled: true,
            url: "https://example.invalid".into(),
            timeout_ms: 1,
            default_priority_fee_sol: 0.0,
            lightning_enabled: false,
        },
        false,
    )
}

#[tokio::test]
async fn pumpapi_live_trade_is_blocked_before_transport_selection() {
    let client = client();
    assert_eq!(
        client
            .pumpapi_trade(pumpapi_trade(false))
            .await
            .err()
            .unwrap()
            .to_string(),
        BROADCAST_DISABLED_ERROR
    );
}

#[tokio::test]
async fn pumpapi_h2a_unsigned_dispatch_keeps_disabled_broadcast_policy_and_lane_error() {
    assert_eq!(
        client()
            .pumpapi_trade(pumpapi_trade(true))
            .await
            .err()
            .unwrap()
            .to_string(),
        "no Default RPC lane configured for PumpAPI execution"
    );
}
