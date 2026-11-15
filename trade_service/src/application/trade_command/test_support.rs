use super::*;

/// Project types and fixed public addresses only; no SDK, config loading or signing.
pub(crate) fn command() -> TradeCommand {
    let quote = Address::from_bytes(
        bs58::decode(WSOL_MINT)
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap(),
    );
    TradeCommand {
        wallet_id: "fixture-wallet".into(),
        idempotency_key: " literal-key ".into(),
        route: TradeRoute {
            pool: Pool::Pump,
            mint: Address::from_bytes([2; 32]),
            quote_mint: quote,
            pool_id: None,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
        },
        amount: TradeAmount::BuySol {
            lamports: 1_000_000,
        },
        execution: TradeExecution {
            method: ExecutionMethod::Pumpapi,
            slippage_bps: 1500,
            tip_sol: 0.0001,
            use_mev: Some(false),
            mev_provider: Some(MevProvider::Default),
            mode: ExecutionMode::Live,
        },
        aggregator_sol: false,
        pumpapi_priority_fee_sol: 0.0002,
    }
}
