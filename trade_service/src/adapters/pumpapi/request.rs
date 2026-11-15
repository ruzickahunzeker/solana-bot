use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::domain::trade::{Address, Side, USD1_MINT, USDC_MINT};

/// Public identity and legacy wire amounts only; never a signing capability.
pub(crate) struct PumpApiRequest {
    pub wallet: Address,
    pub side: Side,
    pub mint: Address,
    pub quote_mint: Address,
    pub pool_id: Option<Address>,
    pub amount_units: Option<u64>,
    pub sell_percent: Option<u64>,
    pub slippage_bps: u64,
    pub priority_fee_sol: f64,
}

pub(crate) fn build_request(input: &PumpApiRequest) -> anyhow::Result<Value> {
    let (action, amount, denominated_in_quote) = match input.side {
        Side::Buy => (
            "buy",
            buy_amount(
                input.amount_units.context("buy amount is missing")?,
                input.quote_mint,
            )?,
            "true",
        ),
        Side::Sell => {
            let percent = input.sell_percent.context("sell_percent is missing")?;
            if percent != 100 {
                bail!("PumpAPI first version only supports sell_percent=100");
            }
            ("sell", Value::String("100%".to_string()), "false")
        }
    };
    let mut request = json!({
        "publicKey": input.wallet.to_string(),
        "action": action,
        "mint": input.mint.to_string(),
        "quoteMint": input.quote_mint.to_string(),
        "amount": amount,
        "denominatedInQuote": denominated_in_quote,
        "slippage": input.slippage_bps as f64 / 100.0,
        "priorityFee": input.priority_fee_sol
    });
    if let Some(pool_id) = input.pool_id {
        request["poolId"] = Value::String(pool_id.to_string());
    }
    Ok(request)
}

fn buy_amount(amount_units: u64, quote_mint: Address) -> anyhow::Result<Value> {
    // Freeze legacy 6/9-decimal and f64 wire behavior, not arbitrary SPL support.
    let decimals = if matches!(quote_mint.to_string().as_str(), USDC_MINT | USD1_MINT) {
        6
    } else {
        9
    };
    let scale = 10_u64.pow(decimals) as f64;
    let amount = amount_units as f64 / scale;
    if amount <= 0.0 || !amount.is_finite() {
        bail!("PumpAPI buy amount must be greater than 0");
    }
    Ok(json!(amount))
}
