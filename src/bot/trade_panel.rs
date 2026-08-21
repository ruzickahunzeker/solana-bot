use crate::{
    db::models::UserTradeSettings,
    execution_params::{ExecutionMethod, ExecutionMode, ExecutionParams, MEV_PROVIDERS},
    trade_client::ResolvedPoolRoute,
};
use chrono::{Duration, Utc};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTab {
    Quick,
    Limit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelSide {
    Buy,
    Sell,
}

impl PanelSide {
    pub fn label(self) -> &'static str {
        match self {
            Self::Buy => "买入",
            Self::Sell => "卖出",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelDirection {
    Below,
    Above,
}

impl PanelDirection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Below => "低于或等于",
            Self::Above => "高于或等于",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMetric {
    PriceUsd,
    MarketCapUsd,
}

impl TriggerMetric {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PriceUsd => "price_usd",
            Self::MarketCapUsd => "market_cap_usd",
        }
    }

    pub fn value_label(self) -> &'static str {
        match self {
            Self::PriceUsd => "触发价格",
            Self::MarketCapUsd => "触发市值",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelExpiry {
    Unlimited,
    Minutes30,
    Hours2,
    Day1,
}

impl PanelExpiry {
    pub fn next(self) -> Self {
        match self {
            Self::Unlimited => Self::Minutes30,
            Self::Minutes30 => Self::Hours2,
            Self::Hours2 => Self::Day1,
            Self::Day1 => Self::Unlimited,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unlimited => "不限",
            Self::Minutes30 => "30分钟",
            Self::Hours2 => "2小时",
            Self::Day1 => "1天",
        }
    }

    pub fn expires_at(self) -> Option<String> {
        let expires_at = match self {
            Self::Unlimited => return None,
            Self::Minutes30 => Utc::now() + Duration::minutes(30),
            Self::Hours2 => Utc::now() + Duration::hours(2),
            Self::Day1 => Utc::now() + Duration::days(1),
        };
        Some(expires_at.format("%Y-%m-%d %H:%M:%S").to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelInputField {
    Amount,
    TriggerValue,
    Slippage,
    Tip,
}

#[derive(Debug, Clone)]
pub struct QuickPanelDraft {
    pub side: PanelSide,
    pub buy_amount_sol: f64,
    pub sell_percent: f64,
    pub execution: ExecutionParams,
}

#[derive(Debug, Clone)]
pub struct LimitPanelDraft {
    pub side: PanelSide,
    pub direction: PanelDirection,
    pub metric: TriggerMetric,
    pub trigger_price_usd: Option<f64>,
    pub trigger_market_cap_usd: Option<f64>,
    pub buy_amount_sol: f64,
    pub sell_percent: f64,
    pub execution: ExecutionParams,
    pub expiry: PanelExpiry,
}

#[derive(Debug, Clone)]
pub struct TradePanelSession {
    pub message_id: Option<MessageId>,
    pub mint: String,
    pub route: ResolvedPoolRoute,
    pub tab: PanelTab,
    pub quick: QuickPanelDraft,
    pub limit: LimitPanelDraft,
}

impl TradePanelSession {
    pub fn new(
        mint: String,
        wallet_id: Option<i64>,
        settings: &UserTradeSettings,
        tab: PanelTab,
        route: ResolvedPoolRoute,
    ) -> Self {
        let mev_provider = if settings.use_mev {
            settings.default_mev_provider.clone()
        } else {
            "none".to_string()
        };
        let common = || ExecutionParams {
            wallet_id,
            mode: ExecutionMode::from_db(&settings.default_execution_mode, true),
            method: ExecutionMethod::from_db(&settings.default_execution_method),
            slippage_bps: settings.default_slippage_bps,
            tip_sol: settings.default_tip_sol,
            mev_mode: mev_provider.clone(),
        };
        Self {
            message_id: None,
            mint,
            route,
            tab,
            quick: QuickPanelDraft {
                side: PanelSide::Buy,
                buy_amount_sol: settings.buy_presets_sol[settings.default_buy_preset_slot],
                sell_percent: settings.sell_presets_percent[settings.default_sell_preset_slot],
                execution: common(),
            },
            limit: LimitPanelDraft {
                side: PanelSide::Buy,
                direction: PanelDirection::Below,
                metric: TriggerMetric::PriceUsd,
                trigger_price_usd: None,
                trigger_market_cap_usd: None,
                buy_amount_sol: settings.buy_presets_sol[settings.default_buy_preset_slot],
                sell_percent: settings.sell_presets_percent[settings.default_sell_preset_slot],
                execution: common(),
                expiry: PanelExpiry::Unlimited,
            },
        }
    }

    pub fn active_execution_mut(&mut self) -> &mut ExecutionParams {
        match self.tab {
            PanelTab::Quick => &mut self.quick.execution,
            PanelTab::Limit => &mut self.limit.execution,
        }
    }

    pub fn active_execution(&self) -> &ExecutionParams {
        match self.tab {
            PanelTab::Quick => &self.quick.execution,
            PanelTab::Limit => &self.limit.execution,
        }
    }

    pub fn active_side(&self) -> PanelSide {
        match self.tab {
            PanelTab::Quick => self.quick.side,
            PanelTab::Limit => self.limit.side,
        }
    }

    pub fn active_amount(&self) -> f64 {
        match (self.tab, self.active_side()) {
            (PanelTab::Quick, PanelSide::Buy) => self.quick.buy_amount_sol,
            (PanelTab::Quick, PanelSide::Sell) => self.quick.sell_percent,
            (PanelTab::Limit, PanelSide::Buy) => self.limit.buy_amount_sol,
            (PanelTab::Limit, PanelSide::Sell) => self.limit.sell_percent,
        }
    }

    pub fn set_active_amount(&mut self, value: f64) {
        match (self.tab, self.active_side()) {
            (PanelTab::Quick, PanelSide::Buy) => self.quick.buy_amount_sol = value,
            (PanelTab::Quick, PanelSide::Sell) => self.quick.sell_percent = value,
            (PanelTab::Limit, PanelSide::Buy) => self.limit.buy_amount_sol = value,
            (PanelTab::Limit, PanelSide::Sell) => self.limit.sell_percent = value,
        }
    }

    pub fn mev_provider_keyboard(&self) -> InlineKeyboardMarkup {
        let current = self.active_execution().mev_mode.as_str();
        let mut rows = MEV_PROVIDERS
            .chunks(2)
            .map(|pair| {
                pair.iter()
                    .map(|provider| {
                        InlineKeyboardButton::callback(
                            format!(
                                "{}{}",
                                if current == *provider { "✅ " } else { "" },
                                provider
                            ),
                            format!("panel:set_mev:{provider}"),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows.push(vec![InlineKeyboardButton::callback(
            "返回交易面板",
            "panel:cancel_mev",
        )]);
        InlineKeyboardMarkup::new(rows)
    }

    pub fn active_trigger_value(&self) -> Option<f64> {
        match self.limit.metric {
            TriggerMetric::PriceUsd => self.limit.trigger_price_usd,
            TriggerMetric::MarketCapUsd => self.limit.trigger_market_cap_usd,
        }
    }

    pub fn set_active_trigger_value(&mut self, value: f64) {
        match self.limit.metric {
            TriggerMetric::PriceUsd => self.limit.trigger_price_usd = Some(value),
            TriggerMetric::MarketCapUsd => self.limit.trigger_market_cap_usd = Some(value),
        }
    }

    pub fn text(&self, wallet_label: &str, use_mev: bool) -> String {
        match self.tab {
            PanelTab::Quick => self.quick_text(wallet_label, use_mev),
            PanelTab::Limit => self.limit_text(wallet_label, use_mev),
        }
    }

    fn quick_text(&self, wallet_label: &str, use_mev: bool) -> String {
        let execution = &self.quick.execution;
        let amount = match self.quick.side {
            PanelSide::Buy => format!("{} SOL", trim_float(self.quick.buy_amount_sol)),
            PanelSide::Sell => format!("{}%", trim_float(self.quick.sell_percent)),
        };
        format!(
            "快速交易\n\nMint: <code>{}</code>\n钱包: {}\n方向: {}\n金额: {}\n模式: {}\n执行方式: {}\n滑点: {}%\n小费: {} SOL\nMEV Provider: {}\nPool: {}{}",
            self.mint,
            wallet_label,
            self.quick.side.label(),
            amount,
            mode_label(execution.mode),
            execution.method.as_str().to_ascii_uppercase(),
            trim_float(execution.slippage_bps as f64 / 100.0),
            trim_float(execution.tip_sol),
            mev_label(execution, use_mev),
            pool_label(&self.route.pool),
            self.route
                .pool_id
                .as_deref()
                .map(|pool_id| format!(" <code>{}</code>", short_text(pool_id)))
                .unwrap_or_default(),
        )
    }

    fn limit_text(&self, wallet_label: &str, use_mev: bool) -> String {
        let execution = &self.limit.execution;
        let amount = match self.limit.side {
            PanelSide::Buy => format!("{} SOL", trim_float(self.limit.buy_amount_sol)),
            PanelSide::Sell => format!("{}%", trim_float(self.limit.sell_percent)),
        };
        let trigger = self
            .active_trigger_value()
            .map(|value| format!("{} USD", trim_float(value)))
            .unwrap_or_else(|| "未设置".to_string());
        format!(
            "创建限价单\n\n类型: 限价{}\n动作: {}{}\n触发依据: {}\nMint: <code>{}</code>\n{}: {}\n{}: {}\n钱包: {}\n模式: {}\n执行方式: {}\n滑点: {}%\n小费: {} SOL\nMEV Provider: {}\n有效期: {}\nPool: 触发时自动解析",
            self.limit.side.label(),
            self.limit.direction.label(),
            self.limit.side.label(),
            match self.limit.metric {
                TriggerMetric::PriceUsd => "价格",
                TriggerMetric::MarketCapUsd => "市值",
            },
            self.mint,
            self.limit.metric.value_label(),
            trigger,
            if self.limit.side == PanelSide::Buy { "买入金额" } else { "卖出比例" },
            amount,
            wallet_label,
            mode_label(execution.mode),
            execution.method.as_str().to_ascii_uppercase(),
            trim_float(execution.slippage_bps as f64 / 100.0),
            trim_float(execution.tip_sol),
            mev_label(execution, use_mev),
            self.limit.expiry.label(),
        )
    }

    pub fn keyboard(&self, settings: &UserTradeSettings) -> InlineKeyboardMarkup {
        let mut rows = vec![vec![
            InlineKeyboardButton::callback(
                if self.tab == PanelTab::Quick {
                    "✅ 交换"
                } else {
                    "交换"
                },
                "panel:tab:quick",
            ),
            InlineKeyboardButton::callback(
                if self.tab == PanelTab::Limit {
                    "✅ 限价"
                } else {
                    "限价"
                },
                "panel:tab:limit",
            ),
            InlineKeyboardButton::callback("返回", "panel:back"),
        ]];

        rows.push(vec![
            InlineKeyboardButton::callback(
                if self.tab == PanelTab::Limit && self.active_side() == PanelSide::Buy {
                    "✅ 限价买入"
                } else if self.tab == PanelTab::Limit {
                    "限价买入"
                } else if self.active_side() == PanelSide::Buy {
                    "✅ 买入"
                } else {
                    "买入"
                },
                "panel:side:buy",
            ),
            InlineKeyboardButton::callback(
                if self.tab == PanelTab::Limit && self.active_side() == PanelSide::Sell {
                    "✅ 限价卖出"
                } else if self.tab == PanelTab::Limit {
                    "限价卖出"
                } else if self.active_side() == PanelSide::Sell {
                    "✅ 卖出"
                } else {
                    "卖出"
                },
                "panel:side:sell",
            ),
            InlineKeyboardButton::callback("刷新", "panel:refresh"),
        ]);

        if self.tab == PanelTab::Limit {
            rows.push(vec![
                InlineKeyboardButton::callback(
                    format!(
                        "动作: {}{}",
                        self.limit.direction.label(),
                        self.limit.side.label()
                    ),
                    "panel:direction",
                ),
                InlineKeyboardButton::callback(
                    if self.limit.metric == TriggerMetric::PriceUsd {
                        "✅ 价格"
                    } else {
                        "价格"
                    },
                    "panel:metric:price",
                ),
                InlineKeyboardButton::callback(
                    if self.limit.metric == TriggerMetric::MarketCapUsd {
                        "✅ 市值"
                    } else {
                        "市值"
                    },
                    "panel:metric:mcap",
                ),
            ]);
            rows.push(vec![InlineKeyboardButton::callback(
                format!(
                    "{}: {}",
                    self.limit.metric.value_label(),
                    self.active_trigger_value()
                        .map(trim_float)
                        .unwrap_or_else(|| "未设置".to_string())
                ),
                "panel:input:trigger",
            )]);
        }

        let presets = if self.active_side() == PanelSide::Buy {
            settings.buy_presets_sol
        } else {
            settings.sell_presets_percent
        };
        for pair in [[0usize, 1usize], [2, 3]] {
            rows.push(
                pair.into_iter()
                    .map(|index| preset_button(self, presets[index], index))
                    .collect(),
            );
        }
        rows.push(vec![
            preset_button(self, presets[4], 4),
            InlineKeyboardButton::callback("自定义", "panel:input:amount"),
        ]);

        if self.tab == PanelTab::Limit {
            rows.push(vec![
                InlineKeyboardButton::callback("钱包", "panel:wallet"),
                InlineKeyboardButton::callback(
                    format!("模式: {}", mode_label(self.limit.execution.mode)),
                    "panel:mode",
                ),
            ]);
        }
        rows.push(vec![
            InlineKeyboardButton::callback(
                format!(
                    "滑点: {}%",
                    trim_float(self.active_execution().slippage_bps as f64 / 100.0)
                ),
                "panel:input:slippage",
            ),
            InlineKeyboardButton::callback(
                format!("小费: {} SOL", trim_float(self.active_execution().tip_sol)),
                "panel:input:tip",
            ),
        ]);
        rows.push(vec![InlineKeyboardButton::callback(
            if settings.use_mev {
                format!("MEV Provider: {}", self.active_execution().mev_mode)
            } else {
                "MEV: 全局关闭".to_string()
            },
            "panel:mev",
        )]);
        if self.tab == PanelTab::Limit {
            rows.push(vec![InlineKeyboardButton::callback(
                format!("有效期: {}", self.limit.expiry.label()),
                "panel:expiry",
            )]);
        }
        rows.push(vec![InlineKeyboardButton::callback(
            if self.tab == PanelTab::Quick {
                "确认交易"
            } else {
                "创建限价单"
            },
            if self.tab == PanelTab::Quick {
                "panel:confirm"
            } else {
                "panel:create_limit"
            },
        )]);
        InlineKeyboardMarkup::new(rows)
    }
}

fn preset_button(session: &TradePanelSession, value: f64, index: usize) -> InlineKeyboardButton {
    let selected = (session.active_amount() - value).abs() < f64::EPSILON;
    let unit = if session.active_side() == PanelSide::Buy {
        "SOL"
    } else {
        "%"
    };
    InlineKeyboardButton::callback(
        format!(
            "{}{} {}",
            if selected { "✅ " } else { "" },
            trim_float(value),
            unit
        ),
        format!("panel:preset:{index}"),
    )
}

fn mode_label(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::DryRun => "Dry-run",
        ExecutionMode::Simulate => "Simulate",
        ExecutionMode::Live => "Live",
    }
}

fn pool_label(pool: &str) -> &str {
    match pool {
        "pump" => "PumpFun",
        "pump-amm" => "PumpSwap",
        "bonk" => "Bonk",
        "meteora-damm-v2" => "Meteora",
        "raydium-amm-v4" => "Raydium AMM V4",
        "raydium-cpmm" => "Raydium CPMM",
        _ => pool,
    }
}

fn short_text(value: &str) -> String {
    if value.len() <= 12 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..6], &value[value.len() - 4..])
    }
}

fn mev_label(execution: &ExecutionParams, use_mev: bool) -> &str {
    if use_mev {
        &execution.mev_mode
    } else {
        "全局关闭"
    }
}

pub fn trim_float(value: f64) -> String {
    let text = format!("{value:.12}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub fn confirm_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("确认执行", "panel:execute")],
        vec![InlineKeyboardButton::callback(
            "取消",
            "panel:cancel_confirm",
        )],
    ])
}

#[cfg(test)]
mod tests {
    use super::{PanelDirection, PanelTab, TradePanelSession, TriggerMetric};
    use crate::{
        db::models::UserTradeSettings, execution_params::ExecutionMode,
        trade_client::ResolvedPoolRoute,
    };

    #[test]
    fn limit_action_dimensions_are_independent() {
        assert_eq!(PanelDirection::Below.label(), "低于或等于");
        assert_eq!(TriggerMetric::MarketCapUsd.as_str(), "market_cap_usd");
    }

    #[test]
    fn panel_uses_user_default_mode_and_slots() {
        let settings = UserTradeSettings {
            user_id: 1,
            use_mev: false,
            default_mev_provider: "auto".to_string(),
            default_execution_mode: "dry_run".to_string(),
            default_execution_method: "sdk".to_string(),
            default_slippage_bps: 1500,
            default_tip_sol: 0.0001,
            default_buy_preset_slot: 3,
            default_sell_preset_slot: 1,
            buy_presets_sol: [0.001, 0.005, 0.01, 0.2, 0.5],
            sell_presets_percent: [10.0, 25.0, 50.0, 75.0, 100.0],
        };
        let panel = TradePanelSession::new(
            "So11111111111111111111111111111111111111112".to_string(),
            None,
            &settings,
            PanelTab::Quick,
            ResolvedPoolRoute {
                mint: "So11111111111111111111111111111111111111112".to_string(),
                pool: "pump".to_string(),
                pool_id: None,
                quote_mint: "So11111111111111111111111111111111111111112".to_string(),
            },
        );
        assert_eq!(panel.quick.execution.mode, ExecutionMode::DryRun);
        assert_eq!(panel.limit.execution.mode, ExecutionMode::DryRun);
        assert_eq!(panel.quick.buy_amount_sol, 0.2);
        assert_eq!(panel.quick.sell_percent, 25.0);
    }
}
