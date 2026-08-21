use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const MEV_PROVIDERS: [&str; 9] = [
    "auto",
    "default",
    "rpc",
    "jito",
    "temporal",
    "flashblock",
    "blockrazor",
    "astralane",
    "speedlanding",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    DryRun,
    Simulate,
    Live,
}

impl ExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DryRun => "dry_run",
            Self::Simulate => "simulate",
            Self::Live => "live",
        }
    }

    pub fn from_db(value: &str, legacy_dry_run: bool) -> Self {
        match value {
            "simulate" => Self::Simulate,
            "live" => Self::Live,
            "dry_run" => Self::DryRun,
            _ if legacy_dry_run => Self::DryRun,
            _ => Self::Live,
        }
    }

    pub fn is_dry_run(self) -> bool {
        matches!(self, Self::DryRun)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMethod {
    Sdk,
    PumpApi,
}

impl ExecutionMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sdk => "sdk",
            Self::PumpApi => "pumpapi",
        }
    }

    pub fn from_db(value: &str) -> Self {
        if value == "pumpapi" {
            Self::PumpApi
        } else {
            Self::Sdk
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionParams {
    pub wallet_id: Option<i64>,
    pub mode: ExecutionMode,
    pub method: ExecutionMethod,
    pub slippage_bps: u64,
    pub tip_sol: f64,
    pub mev_mode: String,
}

impl ExecutionParams {
    pub fn validate(&self) -> Result<()> {
        if self.slippage_bps > 10_000 {
            anyhow::bail!("滑点必须在 0 到 10000 bps 之间");
        }
        if !self.tip_sol.is_finite() || self.tip_sol < 0.0 {
            anyhow::bail!("小费必须是大于或等于 0 的数字");
        }
        if !matches!(
            self.mev_mode.as_str(),
            "none"
                | "auto"
                | "default"
                | "rpc"
                | "jito"
                | "temporal"
                | "flashblock"
                | "blockrazor"
                | "astralane"
                | "speedlanding"
        ) {
            anyhow::bail!("不支持的 MEV 模式");
        }
        if self.method == ExecutionMethod::PumpApi && self.mev_mode != "none" {
            anyhow::bail!("PumpAPI 执行方式暂不支持 MEV");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrderSize {
    FixedBuySol { amount_sol: f64 },
    FollowBuyRatio { percent: f64, max_sol: Option<f64> },
    SellPercent { percent: u64 },
}

impl OrderSize {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::FixedBuySol { amount_sol } if amount_sol.is_finite() && *amount_sol > 0.0 => {}
            Self::FollowBuyRatio { percent, max_sol }
                if percent.is_finite()
                    && *percent > 0.0
                    && max_sol.is_none_or(|value| value.is_finite() && value > 0.0) => {}
            Self::SellPercent { percent } if (1..=100).contains(percent) => {}
            Self::FixedBuySol { .. } => anyhow::bail!("固定买入金额必须大于 0 SOL"),
            Self::FollowBuyRatio { .. } => anyhow::bail!("跟买比例和上限必须大于 0"),
            Self::SellPercent { .. } => anyhow::bail!("卖出比例必须是 1 到 100 的整数"),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitAction {
    BuyBelow,
    BuyAbove,
    SellBelow,
    SellAbove,
}

impl LimitAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuyBelow => "buy_below",
            Self::BuyAbove => "buy_above",
            Self::SellBelow => "sell_below",
            Self::SellAbove => "sell_above",
        }
    }

    pub fn from_db(value: &str, side: &str, trigger_type: &str) -> Self {
        match value {
            "buy_above" => Self::BuyAbove,
            "sell_below" => Self::SellBelow,
            "sell_above" => Self::SellAbove,
            "buy_below" => Self::BuyBelow,
            _ => match (side, trigger_type) {
                ("buy", "above" | "price_gte") => Self::BuyAbove,
                ("sell", "below" | "price_lte") => Self::SellBelow,
                ("sell", _) => Self::SellAbove,
                _ => Self::BuyBelow,
            },
        }
    }

    pub fn side(self) -> &'static str {
        match self {
            Self::BuyBelow | Self::BuyAbove => "buy",
            Self::SellBelow | Self::SellAbove => "sell",
        }
    }

    pub fn trigger_type(self) -> &'static str {
        match self {
            Self::BuyBelow | Self::SellBelow => "below",
            Self::BuyAbove | Self::SellAbove => "above",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BuyBelow => "低于价格买入",
            Self::BuyAbove => "高于价格买入",
            Self::SellBelow => "低于价格卖出",
            Self::SellAbove => "高于价格卖出",
        }
    }

    pub fn should_trigger(self, price: f64, trigger_price: f64) -> bool {
        match self {
            Self::BuyBelow | Self::SellBelow => price <= trigger_price,
            Self::BuyAbove | Self::SellAbove => price >= trigger_price,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutionMethod, ExecutionMode, ExecutionParams, LimitAction, OrderSize};

    #[test]
    fn amount_is_validated_separately_from_execution_params() {
        let params = ExecutionParams {
            wallet_id: None,
            mode: ExecutionMode::DryRun,
            method: ExecutionMethod::Sdk,
            slippage_bps: 1500,
            tip_sol: 0.0001,
            mev_mode: "none".to_string(),
        };
        assert!(params.validate().is_ok());
        assert!(OrderSize::FixedBuySol { amount_sol: 0.01 }
            .validate()
            .is_ok());
        assert!(OrderSize::SellPercent { percent: 101 }.validate().is_err());
    }

    #[test]
    fn legacy_limit_fields_map_to_combined_action() {
        assert_eq!(
            LimitAction::from_db("", "buy", "price"),
            LimitAction::BuyBelow
        );
        assert_eq!(
            LimitAction::from_db("", "sell", "price"),
            LimitAction::SellAbove
        );
        assert_eq!(
            LimitAction::from_db("", "sell", "below"),
            LimitAction::SellBelow
        );
    }
}
