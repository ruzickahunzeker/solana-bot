use crate::{
    db::{models::PumpApiEvent, Database},
    executor::{ExecutionRouter, LimitOrderExecutionRequest},
    price_oracle::PriceOracle,
};
use teloxide::{
    prelude::*,
    types::{ChatId, ParseMode},
    Bot,
};
use tracing::{info, warn};

#[derive(Clone)]
pub struct LimitOrderEngine {
    db: Database,
    executor: ExecutionRouter,
    bot: Option<Bot>,
    price_oracle: PriceOracle,
}

impl LimitOrderEngine {
    pub fn new(
        db: Database,
        executor: ExecutionRouter,
        bot: Option<Bot>,
        price_oracle: PriceOracle,
    ) -> Self {
        Self {
            db,
            executor,
            bot,
            price_oracle,
        }
    }

    pub async fn observe_event(&self, source_event_id: i64, event: &PumpApiEvent) {
        let Some(mint) = event.mint.as_deref() else {
            return;
        };
        let sol_usd = self.price_oracle.sol_usd_price().await;

        let orders = match self.db.list_pending_limit_orders_for_mint(mint).await {
            Ok(orders) => orders,
            Err(err) => {
                warn!(?err, mint, "failed to list pending limit orders");
                return;
            }
        };

        for order in orders {
            let observed = if order.trigger_metric == "market_cap_usd" {
                estimate_market_cap(event, sol_usd)
            } else {
                estimate_price(event, sol_usd)
            };
            let Some(observed) = observed else {
                continue;
            };
            if !order
                .trigger_currency
                .eq_ignore_ascii_case(&observed.currency)
            {
                continue;
            }
            let action = order.action();
            if !action.should_trigger(observed.value, order.trigger_price) {
                continue;
            }
            let Some(size) = order.order_size() else {
                warn!(limit_order_id = order.id, "limit order has no amount");
                continue;
            };
            if let Err(err) = size.validate() {
                warn!(
                    ?err,
                    limit_order_id = order.id,
                    "invalid limit order amount"
                );
                continue;
            }
            let outcome = match self
                .executor
                .execute_limit_order(LimitOrderExecutionRequest {
                    user_id: order.user_id,
                    wallet_id: order.wallet_id,
                    limit_order_id: &order.id,
                    source_event_id,
                    mint: &order.mint,
                    side: action.side(),
                    amount_sol: order.amount_sol,
                    sell_percent: order.sell_percent,
                    execution: order.execution_params(),
                    trigger_price: order.trigger_price,
                    trigger_currency: &order.trigger_currency,
                    observed_price: observed.value,
                    observed_currency: observed.currency,
                    event,
                })
                .await
            {
                Ok(outcome) => outcome,
                Err(err) => {
                    warn!(
                        ?err,
                        limit_order_id = order.id,
                        "failed to insert limit dry-run order"
                    );
                    continue;
                }
            };
            match self
                .db
                .mark_limit_order_triggered(
                    order.user_id,
                    &order.id,
                    observed.value,
                    observed.currency,
                    source_event_id,
                )
                .await
            {
                Ok(true) => info!(
                    limit_order_id = order.id,
                    mint = order.mint,
                    price = observed.value,
                    currency = observed.currency,
                    "limit order triggered"
                ),
                Ok(false) => {}
                Err(err) => warn!(
                    ?err,
                    limit_order_id = order.id,
                    "failed to mark limit order"
                ),
            }
            if let Err(err) = self
                .notify_trigger(
                    &order.id,
                    outcome.order_id.as_deref().unwrap_or("-"),
                    order.user_id,
                    &order.mint,
                    action.side(),
                    observed.value,
                    observed.currency,
                    order.trigger_price,
                    &order.trigger_currency,
                    action.trigger_type(),
                    order.dry_run,
                )
                .await
            {
                warn!(
                    ?err,
                    limit_order_id = order.id,
                    "failed to send limit trigger notification"
                );
            }
        }
    }

    async fn notify_trigger(
        &self,
        limit_order_id: &str,
        order_id: &str,
        user_id: i64,
        mint: &str,
        side: &str,
        observed_price: f64,
        observed_currency: &str,
        trigger_price: f64,
        trigger_currency: &str,
        trigger_type: &str,
        dry_run: bool,
    ) -> anyhow::Result<()> {
        let Some(bot) = &self.bot else {
            return Ok(());
        };
        let user = self.db.get_user_by_id(user_id).await?;
        bot.send_message(
            ChatId(user.telegram_id),
            format!(
                "限价单已触发\n模式: {}\n动作: {}\n条件: {} {} {}\n观察价: {} {}\nMint: <code>{}</code>\n限价单: <code>{}</code>\n执行订单: <code>{}</code>",
                if dry_run { "Dry-run" } else { "Live" },
                side,
                trigger_direction_label(trigger_type, side),
                trim_float(trigger_price),
                trigger_currency,
                trim_float(observed_price),
                observed_currency,
                mint,
                limit_order_id,
                order_id
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        Ok(())
    }
}

fn trigger_direction_label(trigger_type: &str, side: &str) -> &'static str {
    match trigger_type {
        "below" | "price_lte" => "低于或等于",
        "above" | "price_gte" => "高于或等于",
        _ if side == "buy" => "低于或等于",
        _ => "高于或等于",
    }
}

#[derive(Clone, Copy)]
struct ObservedPrice {
    value: f64,
    currency: &'static str,
}

fn estimate_price(event: &PumpApiEvent, sol_usd_price: Option<f64>) -> Option<ObservedPrice> {
    for field in [
        "priceUsd",
        "priceUSD",
        "usdPrice",
        "price_usd",
        "priceInUsd",
    ] {
        if let Some(value) = json_number(&event.raw_json, field) {
            return Some(ObservedPrice {
                value,
                currency: "USD",
            });
        }
    }
    if let Some(value) = json_number(&event.raw_json, "priceSol") {
        return Some(convert_sol_price(value, sol_usd_price));
    }
    if let Some(value) = json_number(&event.raw_json, "price") {
        return Some(convert_sol_price(value, sol_usd_price));
    }
    let sol = event.sol_in_pool.or(event.quote_in_pool)?;
    let tokens = event.tokens_in_pool?;
    if sol > 0.0 && tokens > 0.0 {
        Some(convert_sol_price(sol / tokens, sol_usd_price))
    } else {
        None
    }
}

fn estimate_market_cap(event: &PumpApiEvent, sol_usd_price: Option<f64>) -> Option<ObservedPrice> {
    for field in [
        "marketCapUsd",
        "marketCapUSD",
        "usdMarketCap",
        "market_cap_usd",
    ] {
        if let Some(value) = json_number(&event.raw_json, field) {
            return Some(ObservedPrice {
                value,
                currency: "USD",
            });
        }
    }
    let market_cap_sol = json_number(&event.raw_json, "marketCapSol")
        .or_else(|| json_number(&event.raw_json, "market_cap_sol"))?;
    Some(convert_sol_price(market_cap_sol, sol_usd_price))
}

fn convert_sol_price(value: f64, sol_usd_price: Option<f64>) -> ObservedPrice {
    match sol_usd_price {
        Some(rate) if rate > 0.0 => ObservedPrice {
            value: value * rate,
            currency: "USD",
        },
        _ => ObservedPrice {
            value,
            currency: "SOL",
        },
    }
}

fn json_number(raw_json: &str, field: &str) -> Option<f64> {
    let value: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    value.get(field).and_then(|item| {
        item.as_f64()
            .or_else(|| item.as_str().and_then(|text| text.parse::<f64>().ok()))
    })
}

fn trim_float(value: f64) -> String {
    let text = format!("{value:.12}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::estimate_market_cap;
    use crate::{db::models::PumpApiEvent, execution_params::LimitAction};

    #[test]
    fn trigger_direction_is_independent_from_side() {
        assert!(LimitAction::BuyBelow.should_trigger(9.0, 10.0));
        assert!(LimitAction::SellBelow.should_trigger(9.0, 10.0));
        assert!(LimitAction::BuyAbove.should_trigger(11.0, 10.0));
        assert!(LimitAction::SellAbove.should_trigger(11.0, 10.0));
        assert!(!LimitAction::BuyBelow.should_trigger(11.0, 10.0));
        assert!(!LimitAction::SellAbove.should_trigger(9.0, 10.0));
    }

    #[test]
    fn legacy_price_trigger_keeps_old_behavior() {
        assert!(LimitAction::from_db("", "buy", "price").should_trigger(9.0, 10.0));
        assert!(LimitAction::from_db("", "sell", "price").should_trigger(11.0, 10.0));
    }

    #[test]
    fn market_cap_prefers_usd_and_converts_sol_fallback() {
        let mut event = PumpApiEvent {
            signature: None,
            mint: None,
            pool: None,
            pool_id: None,
            quote_mint: None,
            tx_type: None,
            signer: None,
            sol_in_pool: None,
            tokens_in_pool: None,
            quote_in_pool: None,
            event_timestamp_ms: None,
            received_unix_ms: 0,
            raw_json: r#"{"marketCapUsd":12345}"#.to_string(),
        };
        assert_eq!(
            estimate_market_cap(&event, Some(150.0)).unwrap().value,
            12345.0
        );
        event.raw_json = r#"{"marketCapSol":100}"#.to_string();
        assert_eq!(
            estimate_market_cap(&event, Some(150.0)).unwrap().value,
            15000.0
        );
    }
}
