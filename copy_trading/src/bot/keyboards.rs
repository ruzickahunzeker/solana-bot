use crate::db::models::{
    LimitOrderSummary, OrderSummary, StrategyConfig, UserTradeSettings, WalletRecord, WatchWallet,
};
use crate::execution_params::MEV_PROVIDERS;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

pub fn main_menu_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("钱包管理", "menu:wallet"),
            InlineKeyboardButton::callback("跟单设置", "menu:copy"),
        ],
        vec![
            InlineKeyboardButton::callback("快速交易", "menu:trade"),
            InlineKeyboardButton::callback("限价单", "menu:limit"),
        ],
        vec![
            InlineKeyboardButton::callback("订单", "menu:orders"),
            InlineKeyboardButton::callback("持仓", "menu:positions"),
            InlineKeyboardButton::callback("状态", "menu:status"),
        ],
        vec![
            InlineKeyboardButton::callback("实盘检查", "menu:preflight"),
            InlineKeyboardButton::callback("全局设置", "menu:settings"),
        ],
        vec![InlineKeyboardButton::callback("全局暂停", "panic:confirm")],
    ])
}

pub fn global_settings_keyboard(settings: &UserTradeSettings) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("执行参数", "settings:execution"),
            InlineKeyboardButton::callback(
                format!(
                    "MEV 设置: {}",
                    if settings.use_mev { "开启" } else { "关闭" }
                ),
                "settings:mev",
            ),
        ],
        vec![
            InlineKeyboardButton::callback("买入金额设置", "settings:buy"),
            InlineKeyboardButton::callback("卖出比例设置", "settings:sell"),
        ],
        vec![InlineKeyboardButton::callback(
            format!(
                "快速交易确认: {}",
                if settings.quick_trade_confirmation_enabled {
                    "开启"
                } else {
                    "关闭"
                }
            ),
            "settings:toggle_quick_confirmation",
        )],
        vec![
            InlineKeyboardButton::callback("刷新", "settings:refresh"),
            InlineKeyboardButton::callback("返回主页", "menu:main"),
        ],
    ])
}

pub fn execution_settings_keyboard(settings: &UserTradeSettings) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            format!(
                "默认模式: {}",
                if settings.default_execution_mode == "live" {
                    "Live"
                } else {
                    "Dry-run"
                }
            ),
            "settings:choose:mode",
        )],
        vec![InlineKeyboardButton::callback(
            format!(
                "执行方式: {}",
                settings.default_execution_method.to_ascii_uppercase()
            ),
            "settings:choose:method",
        )],
        vec![
            InlineKeyboardButton::callback(
                format!(
                    "滑点: {}%",
                    trim_float(settings.default_slippage_bps as f64 / 100.0)
                ),
                "settings:input:slippage",
            ),
            InlineKeyboardButton::callback(
                format!("小费: {} SOL", trim_float(settings.default_tip_sol)),
                "settings:input:tip",
            ),
        ],
        vec![InlineKeyboardButton::callback("返回设置", "settings:home")],
    ])
}

pub fn mev_settings_keyboard(settings: &UserTradeSettings) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            format!(
                "USE_MEV: {}",
                if settings.use_mev { "开启" } else { "关闭" }
            ),
            "settings:toggle_mev",
        )],
        vec![InlineKeyboardButton::callback(
            format!("MEV Provider: {}", settings.default_mev_provider),
            "settings:choose:provider",
        )],
        vec![InlineKeyboardButton::callback("返回设置", "settings:home")],
    ])
}

pub fn trade_preset_settings_keyboard(
    settings: &UserTradeSettings,
    side: &str,
) -> InlineKeyboardMarkup {
    let (values, default_slot, unit) = if side == "buy" {
        (
            settings.buy_presets_sol,
            settings.default_buy_preset_slot,
            "SOL",
        )
    } else {
        (
            settings.sell_presets_percent,
            settings.default_sell_preset_slot,
            "%",
        )
    };
    let button = |index: usize| {
        InlineKeyboardButton::callback(
            format!("{}: {} {}", index + 1, trim_float(values[index]), unit),
            format!("settings:input:{side}:{index}"),
        )
    };
    InlineKeyboardMarkup::new(vec![
        vec![button(0), button(1)],
        vec![button(2), button(3)],
        vec![button(4)],
        vec![InlineKeyboardButton::callback(
            format!("默认档位: {}", default_slot + 1),
            format!("settings:choose:default:{side}"),
        )],
        vec![InlineKeyboardButton::callback("返回设置", "settings:home")],
    ])
}

pub fn execution_mode_selection_keyboard(settings: &UserTradeSettings) -> InlineKeyboardMarkup {
    let button = |value: &str, label: &str| {
        InlineKeyboardButton::callback(
            format!(
                "{}{}",
                if settings.default_execution_mode == value {
                    "✅ "
                } else {
                    ""
                },
                label
            ),
            format!("settings:select:mode:{value}"),
        )
    };
    InlineKeyboardMarkup::new(vec![
        vec![button("dry_run", "Dry-run"), button("live", "Live")],
        vec![InlineKeyboardButton::callback(
            "返回执行参数",
            "settings:execution",
        )],
    ])
}

pub fn execution_method_selection_keyboard(settings: &UserTradeSettings) -> InlineKeyboardMarkup {
    let button = |value: &str, label: &str| {
        InlineKeyboardButton::callback(
            format!(
                "{}{}",
                if settings.default_execution_method == value {
                    "✅ "
                } else {
                    ""
                },
                label
            ),
            format!("settings:select:method:{value}"),
        )
    };
    InlineKeyboardMarkup::new(vec![
        vec![button("sdk", "SDK"), button("pumpapi", "PumpAPI")],
        vec![InlineKeyboardButton::callback(
            "返回执行参数",
            "settings:execution",
        )],
    ])
}

pub fn mev_provider_selection_keyboard(
    settings: &UserTradeSettings,
    providers: &[&str],
) -> InlineKeyboardMarkup {
    let mut rows = providers
        .chunks(2)
        .map(|pair| {
            pair.iter()
                .map(|provider| {
                    InlineKeyboardButton::callback(
                        format!(
                            "{}{}",
                            if settings.default_mev_provider == *provider {
                                "✅ "
                            } else {
                                ""
                            },
                            provider
                        ),
                        format!("settings:select:provider:{provider}"),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    rows.push(vec![InlineKeyboardButton::callback(
        "返回 MEV 设置",
        "settings:mev",
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub fn default_preset_selection_keyboard(
    settings: &UserTradeSettings,
    side: &str,
) -> InlineKeyboardMarkup {
    let (values, default_slot, unit, back) = if side == "buy" {
        (
            settings.buy_presets_sol,
            settings.default_buy_preset_slot,
            "SOL",
            "settings:buy",
        )
    } else {
        (
            settings.sell_presets_percent,
            settings.default_sell_preset_slot,
            "%",
            "settings:sell",
        )
    };
    let button = |index: usize| {
        InlineKeyboardButton::callback(
            format!(
                "{}{}: {} {}",
                if index == default_slot { "✅ " } else { "" },
                index + 1,
                trim_float(values[index]),
                unit
            ),
            format!("settings:select:default:{side}:{index}"),
        )
    };
    InlineKeyboardMarkup::new(vec![
        vec![button(0), button(1)],
        vec![button(2), button(3)],
        vec![button(4)],
        vec![InlineKeyboardButton::callback("返回", back)],
    ])
}

pub fn wallet_list_keyboard(wallets: &[WalletRecord]) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for wallet in wallets {
        let marker = if wallet.is_active { "✅" } else { "•" };
        let remote_id = wallet
            .external_wallet_id
            .as_deref()
            .map(short_wallet_id)
            .unwrap_or_else(|| "未绑定".to_string());
        rows.push(vec![InlineKeyboardButton::callback(
            format!(
                "{marker} {} [{}] {}",
                wallet.label,
                remote_id,
                short_address(&wallet.public_key)
            ),
            format!("wallet:detail:{}", wallet.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "创建钱包",
        "wallet:create_menu",
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "导入钱包",
        "wallet:import",
    )]);
    rows.push(vec![InlineKeyboardButton::callback("返回", "menu:main")]);
    InlineKeyboardMarkup::new(rows)
}

fn short_wallet_id(value: &str) -> String {
    if value.chars().count() <= 14 {
        return value.to_string();
    }
    let start: String = value.chars().take(7).collect();
    let end: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{start}...{end}")
}

pub fn wallet_detail_keyboard(wallet: &WalletRecord) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if !wallet.is_active {
        rows.push(vec![InlineKeyboardButton::callback(
            "设为默认",
            format!("wallet:active:{}", wallet.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "改标签",
        format!("wallet:rename:{}", wallet.id),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "导出私钥",
        format!("wallet:export:{}", wallet.id),
    )]);
    if wallet.external_wallet_id.is_some() {
        rows.push(vec![InlineKeyboardButton::callback(
            "转账 SOL",
            format!("wallet:transfer:{}", wallet.id),
        )]);
        rows.push(vec![InlineKeyboardButton::callback(
            "SOL / WSOL",
            format!("wallet:convert:{}", wallet.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "删除钱包",
        format!("wallet:delete_confirm:{}", wallet.id),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "返回钱包列表",
        "menu:wallet",
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub fn wallet_conversion_keyboard(wallet_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                "包装 0.001 SOL",
                format!("wallet:convert_quote:{wallet_id}:wrap:1000000"),
            ),
            InlineKeyboardButton::callback(
                "包装 0.01 SOL",
                format!("wallet:convert_quote:{wallet_id}:wrap:10000000"),
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                "解包 0.001 WSOL",
                format!("wallet:convert_quote:{wallet_id}:unwrap:1000000"),
            ),
            InlineKeyboardButton::callback(
                "全部解包",
                format!("wallet:convert_quote:{wallet_id}:unwrap:all"),
            ),
        ],
        vec![InlineKeyboardButton::callback(
            "返回钱包",
            format!("wallet:detail:{wallet_id}"),
        )],
    ])
}

pub fn wallet_conversion_quote_keyboard(wallet_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "模拟本笔",
            format!("wallet:convert_sim:{wallet_id}"),
        )],
        vec![InlineKeyboardButton::callback(
            "返回兑换",
            format!("wallet:convert:{wallet_id}"),
        )],
    ])
}

pub fn wallet_create_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("普通钱包", "wallet:create")],
        vec![
            InlineKeyboardButton::callback("自定义开头", "wallet:vanity_input:prefix"),
            InlineKeyboardButton::callback("自定义结尾", "wallet:vanity_input:suffix"),
        ],
        vec![InlineKeyboardButton::callback("返回", "menu:wallet")],
    ])
}

pub fn vanity_job_keyboard(job_id: &str, active: bool) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    rows.push(vec![InlineKeyboardButton::callback(
        "刷新进度",
        format!("wallet:vanity_refresh:{job_id}"),
    )]);
    if active {
        rows.push(vec![InlineKeyboardButton::callback(
            "取消生成",
            format!("wallet:vanity_cancel:{job_id}"),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback("返回", "menu:wallet")]);
    InlineKeyboardMarkup::new(rows)
}

pub fn wallet_transfer_target_keyboard(
    source_wallet_id: i64,
    wallets: &[WalletRecord],
) -> InlineKeyboardMarkup {
    let mut rows = wallets
        .iter()
        .filter(|wallet| wallet.id != source_wallet_id && wallet.external_wallet_id.is_some())
        .map(|wallet| {
            vec![InlineKeyboardButton::callback(
                format!("{} {}", wallet.label, short_address(&wallet.public_key)),
                format!("wallet:transfer_target:{source_wallet_id}:{}", wallet.id),
            )]
        })
        .collect::<Vec<_>>();
    rows.push(vec![InlineKeyboardButton::callback(
        "外部地址",
        format!("wallet:transfer_external:{source_wallet_id}"),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "返回钱包",
        format!("wallet:detail:{source_wallet_id}"),
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub fn wallet_transfer_amount_keyboard(source_wallet_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                "50%",
                format!("wallet:transfer_percent:{source_wallet_id}:50"),
            ),
            InlineKeyboardButton::callback(
                "100%",
                format!("wallet:transfer_percent:{source_wallet_id}:100"),
            ),
        ],
        vec![InlineKeyboardButton::callback(
            "自定义 SOL",
            format!("wallet:transfer_custom:{source_wallet_id}"),
        )],
        vec![InlineKeyboardButton::callback(
            "重新选择目标",
            format!("wallet:transfer:{source_wallet_id}"),
        )],
    ])
}

pub fn wallet_transfer_confirm_keyboard(source_wallet_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "确认转账",
            format!("wallet:transfer_confirm:{source_wallet_id}"),
        )],
        vec![InlineKeyboardButton::callback(
            "取消",
            format!("wallet:detail:{source_wallet_id}"),
        )],
    ])
}

pub fn trade_wallet_selection_keyboard(
    wallets: &[WalletRecord],
    current_wallet_id: Option<i64>,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for wallet in wallets {
        let marker = if Some(wallet.id) == current_wallet_id {
            "✅"
        } else {
            "•"
        };
        let remote_id = wallet
            .external_wallet_id
            .as_deref()
            .map(short_wallet_id)
            .unwrap_or_else(|| "未绑定".to_string());
        rows.push(vec![InlineKeyboardButton::callback(
            format!(
                "{marker} {} [{}] {}",
                wallet.label,
                remote_id,
                short_address(&wallet.public_key)
            ),
            format!("panel:set_wallet:{}", wallet.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "返回",
        "panel:cancel_wallet",
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub fn export_confirm_keyboard(wallet_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "确认导出私钥",
            format!("wallet:export_confirm:{}", wallet_id),
        )],
        vec![InlineKeyboardButton::callback(
            "取消",
            format!("wallet:detail:{wallet_id}"),
        )],
    ])
}

pub fn delete_confirm_keyboard(wallet_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "确认删除",
            format!("wallet:delete:{}", wallet_id),
        )],
        vec![InlineKeyboardButton::callback(
            "取消",
            format!("wallet:detail:{wallet_id}"),
        )],
    ])
}

pub fn watch_wallets_keyboard(items: &[WatchWallet]) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for item in items {
        let marker = if item.enabled { "✅" } else { "暂停" };
        rows.push(vec![InlineKeyboardButton::callback(
            format!("{marker} {} {}", item.label, short_address(&item.address)),
            format!("watch:detail:{}", item.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "添加跟踪钱包",
        "watch:add",
    )]);
    rows.push(vec![InlineKeyboardButton::callback("返回", "menu:main")]);
    InlineKeyboardMarkup::new(rows)
}

pub fn watch_detail_keyboard(item: &WatchWallet) -> InlineKeyboardMarkup {
    let toggle_label = if item.enabled {
        "暂停跟踪"
    } else {
        "启用跟踪"
    };
    let toggle_action = if item.enabled {
        format!("watch:disable:{}", item.id)
    } else {
        format!("watch:enable:{}", item.id)
    };
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(toggle_label, toggle_action)],
        vec![InlineKeyboardButton::callback(
            "策略设置",
            format!("strategy:watch:{}", item.id),
        )],
        vec![InlineKeyboardButton::callback(
            "复制地址",
            format!("watch:copy:{}", item.id),
        )],
        vec![InlineKeyboardButton::callback(
            "改标签",
            format!("watch:rename:{}", item.id),
        )],
        vec![InlineKeyboardButton::callback(
            "删除",
            format!("watch:delete_confirm:{}", item.id),
        )],
        vec![InlineKeyboardButton::callback("返回跟踪列表", "menu:copy")],
    ])
}

pub fn strategy_detail_keyboard(strategy: &StrategyConfig) -> InlineKeyboardMarkup {
    let status = if strategy.enabled { "暂停" } else { "启用" };
    let mode = if strategy.dry_run {
        "切 Live"
    } else {
        "切 Dry-run"
    };
    let buy = if strategy.copy_buy {
        "跟买 关"
    } else {
        "跟买 开"
    };
    let sell = if strategy.copy_sell {
        "跟卖 关"
    } else {
        "跟卖 开"
    };
    let fixed_label = if strategy.buy_mode == "fixed" {
        "✅ 固定SOL"
    } else {
        "⚪ 固定SOL"
    };
    let percentage_label = if strategy.buy_mode == "percentage" {
        "✅ 跟买比例"
    } else {
        "⚪ 跟买比例"
    };
    let buy_amount_row = if strategy.buy_mode == "percentage" {
        vec![InlineKeyboardButton::callback(
            format!("买入金额: {} %", trim_float(strategy.buy_ratio_percent)),
            format!("strategy:input:{}:buy_ratio_percent", strategy.id),
        )]
    } else {
        vec![InlineKeyboardButton::callback(
            format!("买入金额: {} SOL", trim_float(strategy.buy_amount_sol)),
            format!("strategy:input:{}:buy_amount_sol", strategy.id),
        )]
    };

    let mut rows = vec![
        vec![
            InlineKeyboardButton::callback(
                status,
                format!("strategy:toggle:{}:enabled", strategy.id),
            ),
            InlineKeyboardButton::callback(mode, format!("strategy:toggle:{}:dryrun", strategy.id)),
        ],
        vec![
            InlineKeyboardButton::callback(buy, format!("strategy:toggle:{}:copybuy", strategy.id)),
            InlineKeyboardButton::callback(
                sell,
                format!("strategy:toggle:{}:copysell", strategy.id),
            ),
        ],
        vec![InlineKeyboardButton::callback(
            format!(
                "Pool筛选: {}",
                if strategy.allowed_pools.is_empty() {
                    "全部".to_string()
                } else {
                    strategy.allowed_pools.join(",")
                }
            ),
            format!("strategy:input:{}:allowed_pools", strategy.id),
        )],
        vec![
            InlineKeyboardButton::callback(
                fixed_label,
                format!("strategy:mode:{}:fixed", strategy.id),
            ),
            InlineKeyboardButton::callback(
                percentage_label,
                format!("strategy:mode:{}:percentage", strategy.id),
            ),
        ],
        buy_amount_row,
        vec![InlineKeyboardButton::callback(
            format!(
                "卖出比例: {} %",
                trim_float(strategy.sell_percent.unwrap_or(100.0))
            ),
            format!("strategy:input:{}:sell_percent", strategy.id),
        )],
        vec![
            InlineKeyboardButton::callback(
                "最小/最大市值",
                format!("strategy:input:{}:mcap_range", strategy.id),
            ),
            InlineKeyboardButton::callback(
                "钱包买入区间",
                format!("strategy:input:{}:target_buy_range", strategy.id),
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                "小费",
                format!("strategy:input:{}:tip_sol", strategy.id),
            ),
            InlineKeyboardButton::callback("MEV", format!("strategy:mev:{}", strategy.id)),
        ],
        vec![
            InlineKeyboardButton::callback(
                format!(
                    "滑点: {}%",
                    trim_float(strategy.max_slippage_bps as f64 / 100.0)
                ),
                format!("strategy:input:{}:max_slippage_bps", strategy.id),
            ),
            InlineKeyboardButton::callback(
                "最大延迟",
                format!("strategy:input:{}:max_signal_delay_ms", strategy.id),
            ),
        ],
        vec![
            InlineKeyboardButton::callback("刷新", format!("strategy:view:{}", strategy.id)),
            InlineKeyboardButton::callback("返回跟单", "menu:copy"),
        ],
    ];
    if let Some(watch_id) = strategy.watch_wallet_id {
        rows.push(vec![
            InlineKeyboardButton::callback("复制钱包", format!("watch:copy:{}", watch_id)),
            InlineKeyboardButton::callback(
                "删除跟踪",
                format!("watch:delete_confirm:{}", watch_id),
            ),
        ]);
    }
    InlineKeyboardMarkup::new(rows)
}

pub fn strategy_mev_keyboard(strategy: &StrategyConfig) -> InlineKeyboardMarkup {
    let providers = std::iter::once("none")
        .chain(MEV_PROVIDERS)
        .collect::<Vec<_>>();
    let mut rows = providers
        .chunks(2)
        .map(|pair| {
            pair.iter()
                .map(|provider| {
                    InlineKeyboardButton::callback(
                        format!(
                            "{}{}",
                            if strategy.mev_mode == *provider {
                                "✅ "
                            } else {
                                ""
                            },
                            provider
                        ),
                        format!("strategy:set:{}:mev_mode:{provider}", strategy.id),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    rows.push(vec![InlineKeyboardButton::callback(
        "返回",
        format!("strategy:view:{}", strategy.id),
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub fn strategy_live_confirm_keyboard(strategy_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "确认切换 Live",
            format!("strategy:live_confirm:{strategy_id}"),
        )],
        vec![InlineKeyboardButton::callback(
            "取消",
            format!("strategy:view:{strategy_id}"),
        )],
    ])
}

pub fn strategy_live_blocked_keyboard(strategy_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("重新检查", "menu:preflight")],
        vec![InlineKeyboardButton::callback(
            "返回策略",
            format!("strategy:view:{strategy_id}"),
        )],
    ])
}

pub fn watch_delete_confirm_keyboard(watch_id: i64) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "确认删除",
            format!("watch:delete:{}", watch_id),
        )],
        vec![InlineKeyboardButton::callback(
            "取消",
            format!("watch:detail:{watch_id}"),
        )],
    ])
}

pub fn trade_menu_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("买入", "trade:buy"),
            InlineKeyboardButton::callback("卖出", "trade:sell"),
        ],
        vec![InlineKeyboardButton::callback("订单", "menu:orders")],
        vec![InlineKeyboardButton::callback("返回", "menu:main")],
    ])
}

pub fn limit_orders_keyboard(orders: &[LimitOrderSummary]) -> InlineKeyboardMarkup {
    let mut rows = vec![vec![InlineKeyboardButton::callback(
        "创建限价单",
        "limit:panel:new",
    )]];
    for order in orders {
        rows.push(vec![InlineKeyboardButton::callback(
            format!(
                "{} {} {} {} {} {} {}",
                order.side,
                order.status,
                if order.dry_run { "Dry-run" } else { "Live" },
                order.action().label(),
                short_address(&order.mint),
                trim_float(order.trigger_price),
                order.trigger_currency
            ),
            format!("limit:view:{}", order.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback("返回", "menu:main")]);
    InlineKeyboardMarkup::new(rows)
}

pub fn limit_direction_keyboard(side: &str) -> InlineKeyboardMarkup {
    let action = if side == "sell" { "卖出" } else { "买入" };
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                format!("低于价格{action}"),
                format!("limit:create_direction:{side}:below"),
            ),
            InlineKeyboardButton::callback(
                format!("高于价格{action}"),
                format!("limit:create_direction:{side}:above"),
            ),
        ],
        vec![InlineKeyboardButton::callback("取消", "menu:limit")],
    ])
}

pub fn limit_order_detail_keyboard(order: &LimitOrderSummary) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if order.status == "pending" {
        rows.push(vec![InlineKeyboardButton::callback(
            if order.dry_run {
                "切 Live"
            } else {
                "切 Dry-run"
            },
            format!("limit:mode:{}", order.id),
        )]);
        rows.push(vec![
            InlineKeyboardButton::callback(
                format!("滑点: {}%", trim_float(order.slippage_bps as f64 / 100.0)),
                format!("limit:input:{}:slippage_bps", order.id),
            ),
            InlineKeyboardButton::callback(
                format!("小费: {} SOL", trim_float(order.tip_sol)),
                format!("limit:input:{}:tip_sol", order.id),
            ),
        ]);
        rows.push(vec![InlineKeyboardButton::callback(
            format!("MEV: {}", order.mev_mode),
            format!("limit:mev:{}", order.id),
        )]);
        rows.push(vec![InlineKeyboardButton::callback(
            "取消限价单",
            format!("limit:cancel:{}", order.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "返回限价单",
        "menu:limit",
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "返回主菜单",
        "menu:main",
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub fn limit_mev_keyboard(order: &LimitOrderSummary) -> InlineKeyboardMarkup {
    let providers = std::iter::once("none")
        .chain(MEV_PROVIDERS)
        .collect::<Vec<_>>();
    let mut rows = providers
        .chunks(2)
        .map(|pair| {
            pair.iter()
                .map(|provider| {
                    InlineKeyboardButton::callback(
                        format!(
                            "{}{}",
                            if order.mev_mode == *provider {
                                "✅ "
                            } else {
                                ""
                            },
                            provider
                        ),
                        format!("limit:set:{}:mev_mode:{provider}", order.id),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    rows.push(vec![InlineKeyboardButton::callback(
        "返回",
        format!("limit:view:{}", order.id),
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub fn limit_live_confirm_keyboard(order_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "确认切换 Live",
            format!("limit:live_confirm:{order_id}"),
        )],
        vec![InlineKeyboardButton::callback(
            "取消",
            format!("limit:view:{order_id}"),
        )],
    ])
}

pub fn limit_live_blocked_keyboard(order_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("重新检查", "menu:preflight")],
        vec![InlineKeyboardButton::callback(
            "返回限价单",
            format!("limit:view:{order_id}"),
        )],
    ])
}

pub fn trade_confirm_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("确认执行", "trade:confirm")],
        vec![InlineKeyboardButton::callback("取消", "menu:trade")],
    ])
}

pub fn orders_keyboard(orders: &[OrderSummary]) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for order in orders {
        rows.push(vec![InlineKeyboardButton::callback(
            format!(
                "{} {} {}",
                order.side,
                order.status,
                short_address(&order.mint)
            ),
            format!("order:view:{}", order.id),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback("返回", "menu:main")]);
    InlineKeyboardMarkup::new(rows)
}

pub fn order_detail_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("返回订单", "menu:orders")],
        vec![InlineKeyboardButton::callback("返回主菜单", "menu:main")],
    ])
}

pub fn back_to_main_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "返回",
        "menu:main",
    )]])
}

fn short_address(value: &str) -> String {
    if value.len() > 10 {
        format!("{}..{}", &value[..4], &value[value.len() - 4..])
    } else {
        value.to_string()
    }
}

fn trim_float(value: f64) -> String {
    let text = format!("{value:.8}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}
