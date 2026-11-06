mod conversions;
pub mod keyboards;
mod pnl;
mod trade_panel;

use crate::{
    config::AppConfig,
    db::{
        models::{MintSignalDetails, StrategyConfig, UserTradeSettings},
        Database,
    },
    execution_params::{
        ExecutionMethod, ExecutionMode, ExecutionParams, LimitAction, OrderSize, MEV_PROVIDERS,
    },
    pool_route::PoolRouteCache,
    preflight::{self, PreflightReport},
    price_oracle::PriceOracle,
    strategy::StrategyEngine,
    trade_client::{ResolvedPoolRoute, TradeRequest, TradeServiceClient},
    wallet::WalletService,
};
use anyhow::{Context, Result};
use keyboards::{
    back_to_main_keyboard, default_preset_selection_keyboard, delete_confirm_keyboard,
    execution_method_selection_keyboard, execution_mode_selection_keyboard,
    execution_settings_keyboard, export_confirm_keyboard, global_settings_keyboard,
    limit_direction_keyboard, limit_live_blocked_keyboard, limit_live_confirm_keyboard,
    limit_mev_keyboard, limit_order_detail_keyboard, limit_orders_keyboard, main_menu_keyboard,
    mev_provider_selection_keyboard, mev_settings_keyboard, order_detail_keyboard, orders_keyboard,
    strategy_detail_keyboard, strategy_live_blocked_keyboard, strategy_live_confirm_keyboard,
    strategy_mev_keyboard, trade_confirm_keyboard, trade_menu_keyboard,
    trade_preset_settings_keyboard, trade_wallet_selection_keyboard, vanity_job_keyboard,
    wallet_create_keyboard, wallet_detail_keyboard, wallet_list_keyboard,
    wallet_transfer_amount_keyboard, wallet_transfer_confirm_keyboard,
    wallet_transfer_target_keyboard, watch_delete_confirm_keyboard, watch_detail_keyboard,
    watch_wallets_keyboard,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use teloxide::{
    adaptors::DefaultParseMode,
    dispatching::UpdateFilterExt,
    dptree,
    prelude::*,
    types::{BotCommand, CallbackQuery, MaybeInaccessibleMessage, Message, MessageId, ParseMode},
};
use tokio::sync::Mutex;
use trade_panel::{
    trim_float, PanelDirection, PanelInputField, PanelSide, PanelTab, TradePanelSession,
    TriggerMetric,
};
use uuid::Uuid;

type HtmlBot = DefaultParseMode<Bot>;

#[derive(Clone)]
pub struct BotState {
    pub config: AppConfig,
    pub db: Database,
    pub wallet_service: WalletService,
    pub trade_client: TradeServiceClient,
    pub price_oracle: PriceOracle,
    pub follow_strategy: StrategyEngine,
    pub pool_routes: PoolRouteCache,
    pnl_client: Option<crate::wallet_pnl::WalletPnlClient>,
    pending: Arc<Mutex<HashMap<i64, PendingAction>>>,
    panels: Arc<Mutex<HashMap<i64, TradePanelSession>>>,
    transfers: Arc<Mutex<HashMap<i64, TransferDraft>>>,
    trades_in_flight: Arc<Mutex<HashSet<i64>>>,
    conversions: Arc<Mutex<HashMap<i64, conversions::Session>>>,
}

#[derive(Debug, Clone)]
enum PendingAction {
    PnlDate {
        target: pnl::Target,
        panel_message_id: MessageId,
        panel_chat_id: ChatId,
    },
    ImportWallet,
    VanityPattern {
        side: String,
        panel_message_id: MessageId,
    },
    TransferAddress {
        source_wallet_id: i64,
        panel_message_id: MessageId,
    },
    TransferAmount {
        source_wallet_id: i64,
        panel_message_id: MessageId,
    },
    RenameWallet {
        wallet_id: i64,
    },
    AddWatch,
    RenameWatch {
        watch_id: i64,
    },
    StrategyInput {
        strategy_id: i64,
        field: String,
    },
    LimitInput {
        order_id: String,
        field: String,
    },
    ManualTrade {
        side: TradeSide,
    },
    ManualTradeAmount {
        side: TradeSide,
        mint: String,
    },
    PanelInput {
        field: PanelInputField,
        panel_message_id: MessageId,
    },
    OpenLimitPanelMint {
        panel_message_id: MessageId,
    },
    ConfirmTrade {
        draft: TradeDraft,
    },
    CreateLimitOrder {
        side: TradeSide,
        direction: TriggerDirection,
    },
    SettingsInput {
        field: SettingsInputField,
        settings_message_id: MessageId,
    },
}

#[derive(Debug, Clone)]
struct TransferDraft {
    source_wallet_id: i64,
    destination: TransferDestination,
    amount: Option<TransferAmount>,
    panel_message_id: MessageId,
}

#[derive(Debug, Clone)]
enum TransferDestination {
    Wallet(i64),
    Address(String),
}

#[derive(Debug, Clone, Copy)]
enum TransferAmount {
    Percent(u64),
    FixedLamports(u64),
}

#[derive(Debug, Clone, Copy)]
enum SettingsInputField {
    Slippage,
    Tip,
    BuyPreset(usize),
    SellPreset(usize),
}

#[derive(Debug, Clone, Copy)]
enum SettingsPage {
    Home,
    Execution,
    ExecutionMode,
    ExecutionMethod,
    Mev,
    MevProvider,
    Buy,
    Sell,
    DefaultBuyPreset,
    DefaultSellPreset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TradeSide {
    Buy,
    Sell,
}

impl TradeSide {
    fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Buy => "买入",
            Self::Sell => "卖出",
        }
    }
}

#[derive(Debug, Clone)]
enum TriggerDirection {
    Below,
    Above,
}

impl TriggerDirection {
    fn label(&self) -> &'static str {
        match self {
            Self::Below => "低于或等于",
            Self::Above => "高于或等于",
        }
    }

    fn action(&self, side: &TradeSide) -> LimitAction {
        match (side, self) {
            (TradeSide::Buy, Self::Below) => LimitAction::BuyBelow,
            (TradeSide::Buy, Self::Above) => LimitAction::BuyAbove,
            (TradeSide::Sell, Self::Below) => LimitAction::SellBelow,
            (TradeSide::Sell, Self::Above) => LimitAction::SellAbove,
        }
    }
}

#[derive(Debug, Clone)]
struct TradeDraft {
    side: TradeSide,
    mint: String,
    amount: f64,
    execution: ExecutionParams,
    route: ResolvedPoolRoute,
    quote_amount: Option<String>,
    sol_hop_pool_id: Option<String>,
}

fn custom_quote_route(route: &ResolvedPoolRoute) -> bool {
    matches!(route.pool.as_str(), "stonkfun" | "stonkfun-swap")
        && !matches!(
            route.quote_mint.as_str(),
            "So11111111111111111111111111111111111111112"
                | "So11111111111111111111111111111111111111111"
        )
}

fn isolated_sdk_single_pool_supported(draft: &TradeDraft) -> bool {
    if draft.sol_hop_pool_id.is_some() || custom_quote_route(&draft.route) {
        return false;
    }
    draft.route.quote_mint == crate::trade_client::WSOL_MINT
        || (draft.side == TradeSide::Sell
            && draft.mint == crate::trade_client::WSOL_MINT
            && isolated_wsol_sell_route(&draft.route))
}

fn isolated_sdk_bridge_buy_supported(draft: &TradeDraft) -> bool {
    draft.side == TradeSide::Buy
        && draft.sol_hop_pool_id.is_none()
        && draft.route.pool_id.is_some()
        && (draft.route.pool == "pump-amm"
            || (matches!(draft.route.pool.as_str(), "raydium-cpmm" | "raydium-amm-v4")
                && draft.route.quote_mint == crate::trade_client::USDC_MINT))
        && draft.route.mint == draft.mint
        && draft.route.quote_mint != crate::trade_client::WSOL_MINT
        && draft.route.quote_mint != draft.mint
        && !custom_quote_route(&draft.route)
}

fn isolated_sdk_bridge_sell_supported(draft: &TradeDraft) -> bool {
    draft.side == TradeSide::Sell
        && draft.route.pool == "raydium-cpmm"
        && draft.route.pool_id.is_some()
        && draft.route.quote_mint == crate::trade_client::USDT_MINT
        && draft.route.mint == draft.mint
        && draft.mint != draft.route.quote_mint
        && draft.sol_hop_pool_id.is_some()
}

fn isolated_sdk_simulation_supported(draft: &TradeDraft) -> bool {
    isolated_sdk_single_pool_supported(draft)
        || isolated_sdk_bridge_buy_supported(draft)
        || isolated_sdk_bridge_sell_supported(draft)
}

fn isolated_wsol_sell_route(route: &ResolvedPoolRoute) -> bool {
    route.mint == crate::trade_client::WSOL_MINT
        && route.pool == "raydium-cpmm"
        && route.quote_mint == crate::trade_client::USDC_MINT
        && route.pool_id.is_some()
}

fn quote_amount_units(text: &str, decimals: u8) -> Result<u64> {
    let (whole, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    if whole.is_empty()
        || !whole.bytes().all(|value| value.is_ascii_digit())
        || !fraction.bytes().all(|value| value.is_ascii_digit())
        || fraction.len() > decimals as usize
        || decimals > 18
    {
        anyhow::bail!("Quote 金额格式或精度不正确");
    }
    let scale = 10_u64.pow(decimals as u32);
    let whole = whole.parse::<u64>()?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u64>()?
            .checked_mul(10_u64.pow(decimals as u32 - fraction.len() as u32))
            .context("Quote 金额过大")?
    };
    let units = whole
        .checked_mul(scale)
        .and_then(|value| value.checked_add(fraction))
        .context("Quote 金额过大")?;
    if units == 0 {
        anyhow::bail!("Quote 金额必须大于零");
    }
    Ok(units)
}

fn format_quote_units(units: u64, decimals: u8) -> String {
    let scale = 10_u64.pow(decimals.min(18) as u32);
    if scale == 1 {
        return units.to_string();
    }
    let fraction = format!("{:0width$}", units % scale, width = decimals as usize);
    format!("{}.{}", units / scale, fraction.trim_end_matches('0'))
        .trim_end_matches('.')
        .to_string()
}

fn quote_order_amount(request_json: Option<&str>) -> Option<String> {
    let request: serde_json::Value = serde_json::from_str(request_json?).ok()?;
    let quote = request.get("quote_mint")?.as_str()?;
    let amount = request
        .get("amount_quote")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            request
                .get("amount_quote_units")
                .and_then(serde_json::Value::as_u64)
                .map(|value| format!("{value} 最小单位"))
        })?;
    Some(format!("{amount} {}", short_text(quote)))
}

impl BotState {
    pub fn new(
        config: AppConfig,
        db: Database,
        wallet_service: WalletService,
        trade_client: TradeServiceClient,
        price_oracle: PriceOracle,
        follow_strategy: StrategyEngine,
        pool_routes: PoolRouteCache,
    ) -> Self {
        let pnl_client = config
            .web_api_base_url
            .as_deref()
            .zip(config.web_api_token.clone())
            .and_then(|(url, token)| crate::wallet_pnl::WalletPnlClient::new(url, token).ok());
        Self {
            pnl_client,
            config,
            db,
            wallet_service,
            trade_client,
            price_oracle,
            follow_strategy,
            pool_routes,
            pending: Arc::new(Mutex::new(HashMap::new())),
            panels: Arc::new(Mutex::new(HashMap::new())),
            transfers: Arc::new(Mutex::new(HashMap::new())),
            trades_in_flight: Arc::new(Mutex::new(HashSet::new())),
            conversions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub async fn run_bot(bot: Bot, state: BotState) -> Result<()> {
    let bot = bot.parse_mode(ParseMode::Html);
    let state = Arc::new(state);

    install_bot_menu(&bot).await?;

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn install_bot_menu(bot: &HtmlBot) -> Result<()> {
    bot.set_my_commands([
        BotCommand::new("start", "主菜单"),
        BotCommand::new("status", "系统状态"),
        BotCommand::new("wallets", "钱包管理"),
        BotCommand::new("watch", "跟单设置"),
        BotCommand::new("orders", "最近订单"),
        BotCommand::new("positions", "Dry-run 持仓"),
        BotCommand::new("limits", "限价单"),
        BotCommand::new("signals", "最近信号"),
        BotCommand::new("preflight", "实盘检查"),
        BotCommand::new("buy", "买入 mint SOL"),
        BotCommand::new("sell", "卖出 mint 百分比"),
        BotCommand::new("panic", "全局暂停"),
    ])
    .await?;
    Ok(())
}

async fn handle_message(bot: HtmlBot, message: Message, state: Arc<BotState>) -> Result<()> {
    let Some(user) = message.from.as_ref() else {
        return Ok(());
    };
    let telegram_id = user.id.0 as i64;
    if !state.config.is_allowed_user(telegram_id) {
        bot.send_message(message.chat.id, "未授权用户").await?;
        return Ok(());
    }

    let app_user = state
        .db
        .upsert_user(telegram_id, user.username.as_deref())
        .await?;
    let text = message.text().unwrap_or("").trim();

    if text.starts_with('/') {
        state.pending.lock().await.remove(&telegram_id);
        state.panels.lock().await.remove(&telegram_id);
    }

    // Drop the pending-state mutex before handlers can update that state again.
    let pending_action = state.pending.lock().await.remove(&telegram_id);
    if let Some(action) = pending_action {
        if starts_new_trade_panel(&action, text) {
            send_new_trade_panel(
                &bot,
                message.chat.id,
                &state,
                app_user.id,
                telegram_id,
                text,
                PanelTab::Quick,
            )
            .await?;
            return Ok(());
        }
        handle_pending_message(
            &bot,
            message.chat.id,
            &state,
            app_user.id,
            telegram_id,
            action,
            text,
            message.id,
        )
        .await?;
        return Ok(());
    }

    if let Some(private_key) = text.strip_prefix("/import ") {
        let wallet = state
            .wallet_service
            .import_wallet(app_user.id, private_key, Some("Imported"))
            .await?;
        bot.send_message(
            message.chat.id,
            format!("导入成功\n钱包: <code>{}</code>", wallet.public_key),
        )
        .reply_markup(wallet_detail_keyboard(&wallet))
        .await?;
        return Ok(());
    }

    if let Some(args) = text.strip_prefix("/watch_add ") {
        let (address, label) = split_first_arg(args, "Watch");
        state
            .db
            .add_watch_wallet(app_user.id, address, label)
            .await?;
        state.follow_strategy.refresh_cache().await?;
        bot.send_message(message.chat.id, "已添加跟踪钱包")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    }

    if let Some(args) = text.strip_prefix("/buy ") {
        handle_trade_text(
            &bot,
            message.chat.id,
            &state,
            telegram_id,
            TradeSide::Buy,
            args,
        )
        .await?;
        return Ok(());
    }

    if let Some(args) = text.strip_prefix("/pool ") {
        handle_explicit_pool_panel(
            &bot,
            message.chat.id,
            &state,
            app_user.id,
            telegram_id,
            args,
        )
        .await?;
        return Ok(());
    }

    if let Some(args) = text.strip_prefix("/sell ") {
        handle_trade_text(
            &bot,
            message.chat.id,
            &state,
            telegram_id,
            TradeSide::Sell,
            args,
        )
        .await?;
        return Ok(());
    }

    match text {
        "/start" | "/help" => {
            bot.send_message(message.chat.id, help_text())
                .reply_markup(main_menu_keyboard())
                .await?;
        }
        "/status" => {
            state
                .wallet_service
                .sync_trade_service_wallets(app_user.id)
                .await?;
            let wallets = state.db.list_wallets(app_user.id).await?;
            let watch_wallets = state.db.list_watch_wallets(app_user.id).await?;
            bot.send_message(
                message.chat.id,
                format!(
                    "系统状态\n钱包: {}\n跟踪钱包: {}\n默认 dry-run: {}\n默认买入: {} SOL\n默认滑点: {} bps\n默认小费: {} SOL\nSOL/USD: {}",
                    wallets.len(),
                    watch_wallets.len(),
                    state.config.dry_run_default,
                    state.config.default_buy_amount_sol,
                    state.config.default_slippage_bps,
                    state.config.default_tip_sol,
                    state
                        .config
                        .sol_usd_price
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "未配置".to_string())
                ),
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
        }
        "/wallets" => show_wallets(&bot, message.chat.id, None, &state, app_user.id).await?,
        "/watch" => show_watch_wallets(&bot, message.chat.id, &state, app_user.id).await?,
        "/strategies" => show_strategies(&bot, message.chat.id, &state, app_user.id).await?,
        "/orders" => show_orders(&bot, message.chat.id, &state, app_user.id).await?,
        "/positions" => show_positions(&bot, message.chat.id, &state, app_user.id).await?,
        "/limits" => show_limit_orders(&bot, message.chat.id, &state, app_user.id).await?,
        "/signals" => show_signals(&bot, message.chat.id, &state, app_user.id).await?,
        "/preflight" => show_preflight(&bot, message.chat.id, &state, app_user.id).await?,
        "/panic" => {
            state.follow_strategy.panic_pause();
            state
                .db
                .audit(
                    Some(app_user.id),
                    "panic.requested",
                    Some("telegram"),
                    None,
                    None,
                )
                .await?;
            bot.send_message(
                message.chat.id,
                "已立即暂停本进程的全部 Live 自动交易。重启服务后恢复。",
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
        }
        _ if is_probable_wallet_address(text) => {
            send_new_trade_panel(
                &bot,
                message.chat.id,
                &state,
                app_user.id,
                telegram_id,
                text,
                PanelTab::Quick,
            )
            .await?;
        }
        _ => {
            bot.send_message(message.chat.id, "请选择菜单，或发送 /help 查看命令。")
                .reply_markup(main_menu_keyboard())
                .await?;
        }
    }
    Ok(())
}

async fn handle_callback(bot: HtmlBot, query: CallbackQuery, state: Arc<BotState>) -> Result<()> {
    let telegram_id = query.from.id.0 as i64;
    if !state.config.is_allowed_user(telegram_id) {
        bot.answer_callback_query(query.id.clone())
            .text("未授权")
            .await?;
        return Ok(());
    }
    let data = query.data.clone().unwrap_or_default();
    if data.starts_with("panel:") {
        tracing::info!(callback = data.as_str(), "trade panel callback received");
    }
    let hint = match data.as_str() {
        "panel:input:amount" => Some("请直接发送自定义金额"),
        "panel:input:trigger" => Some("请直接发送触发数值，面板会在输入后刷新"),
        "panel:input:slippage" => Some("请发送滑点百分比，例如 15"),
        "panel:input:tip" => Some("请发送小费 SOL，例如 0.0001"),
        "limit:panel:new" => Some("请直接发送 Mint 地址"),
        value if value.starts_with("settings:input:buy:") => Some("请发送新的买入金额（SOL）"),
        value if value.starts_with("settings:input:sell:") => {
            Some("请发送新的卖出比例（1 到 100 的整数）")
        }
        "settings:input:slippage" => Some("请发送滑点百分比，例如 15"),
        "settings:input:tip" => Some("请发送小费 SOL，例如 0.0001"),
        "wallet:vanity_input:prefix" => Some("请发送 1 到 4 位 Base58 开头字符"),
        "wallet:vanity_input:suffix" => Some("请发送 1 到 4 位 Base58 结尾字符"),
        _ => None,
    };
    let mut answer = bot.answer_callback_query(query.id.clone());
    if let Some(hint) = hint {
        answer = answer.text(hint);
    }
    answer.await?;

    let app_user = state
        .db
        .upsert_user(telegram_id, query.from.username.as_deref())
        .await?;
    let Some(message) = query.message else {
        return Ok(());
    };
    let chat_id = message.chat().id;
    if data.starts_with("menu:") {
        state.pending.lock().await.remove(&telegram_id);
        state.panels.lock().await.remove(&telegram_id);
        state.transfers.lock().await.remove(&telegram_id);
        state.conversions.lock().await.remove(&telegram_id);
    }

    let is_pnl = data.starts_with("pnl:") || data.starts_with("ppnl:");
    if !is_pnl {
        let mut pending = state.pending.lock().await;
        if matches!(
            pending.get(&telegram_id),
            Some(PendingAction::PnlDate { .. })
        ) {
            pending.remove(&telegram_id);
        }
    }
    if data == "menu:position_pnl" || data.starts_with("ppnl:") {
        bot.edit_message_text(chat_id, message.id(), "聪明钱仓位筛选已移至 Web 展示页：\n<a href=\"http://10.9.0.7:19081/\">打开仓位盈亏页面</a>")
            .reply_markup(back_to_main_keyboard()).await?;
        return Ok(());
    }
    if is_pnl {
        pnl::handle(&bot, &message, &state, app_user.id, telegram_id, &data).await?;
        return Ok(());
    }
    if data.starts_with("wsol:") {
        conversions::handle(&bot, &message, &state, app_user.id, telegram_id, &data).await?;
        return Ok(());
    }
    if data.starts_with("panel:") || data == "limit:panel:new" {
        handle_trade_panel_callback(&bot, &message, &state, app_user.id, telegram_id, &data)
            .await?;
        return Ok(());
    }
    if data.starts_with("settings:") {
        handle_settings_callback(&bot, &message, &state, app_user.id, telegram_id, &data).await?;
        return Ok(());
    }

    match data.as_str() {
        "menu:main" => {
            bot.edit_message_text(chat_id, message.id(), "主菜单")
                .reply_markup(main_menu_keyboard())
                .await?;
        }
        "menu:settings" => {
            render_settings_page(
                &bot,
                chat_id,
                message.id(),
                &state,
                app_user.id,
                SettingsPage::Home,
            )
            .await?;
        }
        "menu:wallet" => {
            show_wallets(&bot, chat_id, Some(message.id()), &state, app_user.id).await?
        }
        "wallet:create_menu" => {
            bot.edit_message_text(
                chat_id,
                message.id(),
                "创建钱包\n\n普通钱包会立即创建；自定义钱包支持 1 到 4 位 Base58 地址开头或结尾。",
            )
            .reply_markup(wallet_create_keyboard())
            .await?;
        }
        "wallet:create" => {
            let wallet = state
                .wallet_service
                .create_wallet(app_user.id, Some("Wallet"))
                .await?;
            bot.edit_message_text(
                chat_id,
                message.id(),
                format!("创建成功\n<code>{}</code>", wallet.public_key),
            )
            .reply_markup(wallet_detail_keyboard(&wallet))
            .await?;
        }
        "wallet:vanity_input:prefix" | "wallet:vanity_input:suffix" => {
            let side = data
                .strip_prefix("wallet:vanity_input:")
                .unwrap_or("prefix")
                .to_string();
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::VanityPattern {
                    side: side.clone(),
                    panel_message_id: message.id(),
                },
            );
            let label = if side == "prefix" { "开头" } else { "结尾" };
            bot.edit_message_text(
                chat_id,
                message.id(),
                format!("自定义地址{label}\n\n请发送 1 到 4 位 Base58 字符。区分大小写，不支持 0、O、I、l。"),
            )
            .reply_markup(wallet_create_keyboard())
            .await?;
        }
        "wallet:import" => {
            state
                .pending
                .lock()
                .await
                .insert(telegram_id, PendingAction::ImportWallet);
            bot.send_message(
                chat_id,
                "请发送私钥，支持 base58 或 JSON 数组。也可以用：\n<code>/import 私钥</code>",
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
        }
        "menu:copy" => show_watch_wallets(&bot, chat_id, &state, app_user.id).await?,
        "menu:positions" => show_positions(&bot, chat_id, &state, app_user.id).await?,
        "menu:trade" => show_trade_menu(&bot, chat_id, &state, app_user.id).await?,
        "menu:limit" => {
            show_limit_orders(&bot, chat_id, &state, app_user.id).await?;
        }
        "menu:orders" => show_orders(&bot, chat_id, &state, app_user.id).await?,
        "menu:preflight" => show_preflight(&bot, chat_id, &state, app_user.id).await?,
        "menu:status" => {
            bot.send_message(
                chat_id,
                "服务已启动。ZMQ 入库和 Telegram 管理为第一阶段能力。",
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
        }
        "panic:confirm" => {
            state
                .db
                .audit(
                    Some(app_user.id),
                    "panic.confirmed",
                    Some("telegram"),
                    None,
                    None,
                )
                .await?;
            bot.send_message(chat_id, "已确认暂停。自动实盘在第一版默认关闭。")
                .reply_markup(back_to_main_keyboard())
                .await?;
        }
        "watch:add" => {
            state
                .pending
                .lock()
                .await
                .insert(telegram_id, PendingAction::AddWatch);
            bot.send_message(
                chat_id,
                "请发送要跟踪的钱包地址，可追加标签：\n<code>钱包地址 标签</code>",
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
        }
        "trade:buy" => {
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::ManualTrade {
                    side: TradeSide::Buy,
                },
            );
            bot.send_message(chat_id, "请发送：\n<code>mint 买入SOL数量</code>\n例如：\n<code>So11111111111111111111111111111111111111112 0.01</code>")
                .reply_markup(back_to_main_keyboard())
                .await?;
        }
        "trade:sell" => {
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::ManualTrade {
                    side: TradeSide::Sell,
                },
            );
            bot.send_message(chat_id, "请发送：\n<code>mint 卖出百分比</code>\n例如：\n<code>So11111111111111111111111111111111111111112 100</code>")
                .reply_markup(back_to_main_keyboard())
                .await?;
        }
        "limit:create:buy" => {
            bot.send_message(chat_id, "选择买入触发方向")
                .reply_markup(limit_direction_keyboard("buy"))
                .await?;
        }
        "limit:create:sell" => {
            bot.send_message(chat_id, "选择卖出触发方向")
                .reply_markup(limit_direction_keyboard("sell"))
                .await?;
        }
        "trade:confirm" => {
            execute_pending_trade(
                &bot,
                chat_id,
                message.id(),
                &state,
                app_user.id,
                telegram_id,
            )
            .await?
        }
        _ => {
            handle_structured_callback(&bot, &message, &state, app_user.id, telegram_id, &data)
                .await?
        }
    }

    Ok(())
}

async fn handle_structured_callback(
    bot: &HtmlBot,
    message: &MaybeInaccessibleMessage,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    data: &str,
) -> Result<()> {
    let chat_id = message.chat().id;
    if let Some(id) = data.strip_prefix("wallet:detail:") {
        state
            .wallet_service
            .sync_trade_service_wallets(user_id)
            .await?;
        let wallet = state.db.get_wallet(user_id, id.parse()?).await?;
        let balance = wallet_balance_for(state, user_id, &wallet).await;
        let (keyboard, conversion_ready) = conversions::detail_keyboard(
            state,
            user_id,
            telegram_id,
            chat_id,
            message.id(),
            &wallet,
        )
        .await;
        let mut text = wallet_detail_text(&wallet, balance);
        if wallet.external_wallet_id.is_some() && !conversion_ready {
            text.push_str(
                "\nSOL / WSOL 预览暂不可用：交易端报价接口尚未就绪或钱包暂不满足报价条件。",
            );
        }
        bot.edit_message_text(chat_id, message.id(), text)
            .reply_markup(keyboard)
            .await?;
    } else if let Some(job_id) = data.strip_prefix("wallet:vanity_refresh:") {
        let job = state.wallet_service.vanity_job(user_id, job_id).await?;
        if job.status == "completed" {
            state
                .wallet_service
                .sync_trade_service_wallets(user_id)
                .await?;
        }
        render_vanity_job(bot, chat_id, message.id(), &job).await?;
    } else if let Some(job_id) = data.strip_prefix("wallet:vanity_cancel:") {
        let job = state
            .wallet_service
            .cancel_vanity_job(user_id, job_id)
            .await?;
        render_vanity_job(bot, chat_id, message.id(), &job).await?;
    } else if let Some(id) = data.strip_prefix("wallet:transfer:") {
        let source_wallet_id = id.parse()?;
        let source = state.db.get_wallet(user_id, source_wallet_id).await?;
        ensure_transferable_wallet(&source)?;
        state.transfers.lock().await.remove(&telegram_id);
        let wallets = state.db.list_wallets(user_id).await?;
        let balance = wallet_balance_for(state, user_id, &source).await;
        bot.edit_message_text(
            chat_id,
            message.id(),
            format!(
                "SOL 转账\n\n来源: {}\n地址: <code>{}</code>\n余额: {}\n\n请选择接收钱包或外部地址。",
                escape_html(&source.label),
                escape_html(&source.public_key),
                format_sol_balance(balance)
            ),
        )
        .reply_markup(wallet_transfer_target_keyboard(source_wallet_id, &wallets))
        .await?;
    } else if let Some(value) = data.strip_prefix("wallet:transfer_target:") {
        let (source, target) = parse_two_i64(value)?;
        let target_wallet = state.db.get_wallet(user_id, target).await?;
        ensure_transferable_wallet(&target_wallet)?;
        let draft = TransferDraft {
            source_wallet_id: source,
            destination: TransferDestination::Wallet(target),
            amount: None,
            panel_message_id: message.id(),
        };
        state
            .transfers
            .lock()
            .await
            .insert(telegram_id, draft.clone());
        render_transfer_amount(bot, chat_id, state, user_id, &draft).await?;
    } else if let Some(id) = data.strip_prefix("wallet:transfer_external:") {
        let source_wallet_id = id.parse()?;
        let source = state.db.get_wallet(user_id, source_wallet_id).await?;
        ensure_transferable_wallet(&source)?;
        state.pending.lock().await.insert(
            telegram_id,
            PendingAction::TransferAddress {
                source_wallet_id,
                panel_message_id: message.id(),
            },
        );
        bot.edit_message_text(
            chat_id,
            message.id(),
            "SOL 转账\n\n请发送接收方 Solana 地址。",
        )
        .reply_markup(wallet_transfer_target_keyboard(
            source_wallet_id,
            &state.db.list_wallets(user_id).await?,
        ))
        .await?;
    } else if let Some(value) = data.strip_prefix("wallet:transfer_percent:") {
        let (source, percent) = parse_two_i64(value)?;
        let mut transfers = state.transfers.lock().await;
        let draft = transfers
            .get_mut(&telegram_id)
            .context("转账面板已过期，请重新选择目标")?;
        if draft.source_wallet_id != source {
            anyhow::bail!("转账面板来源钱包不匹配");
        }
        draft.amount = Some(TransferAmount::Percent(percent as u64));
        let draft = draft.clone();
        drop(transfers);
        render_transfer_confirm(bot, chat_id, state, user_id, &draft).await?;
    } else if let Some(id) = data.strip_prefix("wallet:transfer_custom:") {
        let source_wallet_id = id.parse()?;
        let draft = state
            .transfers
            .lock()
            .await
            .get(&telegram_id)
            .cloned()
            .context("请先选择转账目标")?;
        if draft.source_wallet_id != source_wallet_id {
            anyhow::bail!("转账面板来源钱包不匹配");
        }
        state.pending.lock().await.insert(
            telegram_id,
            PendingAction::TransferAmount {
                source_wallet_id,
                panel_message_id: message.id(),
            },
        );
        bot.edit_message_text(
            chat_id,
            message.id(),
            "SOL 转账\n\n请发送自定义 SOL 数量，例如 0.1。",
        )
        .reply_markup(wallet_transfer_amount_keyboard(source_wallet_id))
        .await?;
    } else if let Some(id) = data.strip_prefix("wallet:transfer_confirm:") {
        let source_wallet_id = id.parse()?;
        let draft = state
            .transfers
            .lock()
            .await
            .get(&telegram_id)
            .cloned()
            .context("转账面板已过期")?;
        if draft.source_wallet_id != source_wallet_id {
            anyhow::bail!("转账面板来源钱包不匹配");
        }
        bot.edit_message_text(
            chat_id,
            message.id(),
            "正在重新检查余额、手续费并提交转账...",
        )
        .await?;
        let (to_wallet_id, to_address) = match &draft.destination {
            TransferDestination::Wallet(id) => (Some(*id), None),
            TransferDestination::Address(address) => (None, Some(address.as_str())),
        };
        let (amount_lamports, percent) = match draft.amount.context("尚未选择转账金额")? {
            TransferAmount::Percent(value) => (None, Some(value)),
            TransferAmount::FixedLamports(value) => (Some(value), None),
        };
        let result = state
            .wallet_service
            .transfer_sol(
                user_id,
                source_wallet_id,
                to_wallet_id,
                to_address,
                amount_lamports,
                percent,
            )
            .await?;
        state.transfers.lock().await.remove(&telegram_id);
        bot.edit_message_text(chat_id, message.id(), format_transfer_result(&result))
            .reply_markup(wallet_detail_keyboard(
                &state.db.get_wallet(user_id, source_wallet_id).await?,
            ))
            .await?;
    } else if let Some(id) = data.strip_prefix("wallet:active:") {
        let wallet_id = id.parse()?;
        state.db.set_active_wallet(user_id, wallet_id).await?;
        state.follow_strategy.refresh_cache().await?;
        bot.send_message(chat_id, "已设为默认钱包")
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else if let Some(id) = data.strip_prefix("wallet:rename:") {
        let wallet_id = id.parse()?;
        state
            .pending
            .lock()
            .await
            .insert(telegram_id, PendingAction::RenameWallet { wallet_id });
        bot.send_message(chat_id, "请发送新的钱包标签。")
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else if let Some(id) = data.strip_prefix("wallet:export:") {
        let wallet_id = id.parse()?;
        bot.send_message(
            chat_id,
            "导出私钥会在 Telegram 中显示敏感信息。确认后请立即保存并删除消息。",
        )
        .reply_markup(export_confirm_keyboard(wallet_id))
        .await?;
    } else if let Some(id) = data.strip_prefix("wallet:export_confirm:") {
        let wallet_id = id.parse()?;
        let private_key = state
            .wallet_service
            .export_private_key_base58(user_id, wallet_id)
            .await?;
        let sent = bot
            .send_message(
                chat_id,
                format!("私钥 base58：\n<code>{private_key}</code>\n请保存后删除这条消息。"),
            )
            .await?;
        let bot_clone = bot.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let _ = bot_clone.delete_message(chat_id, sent.id).await;
        });
    } else if let Some(id) = data.strip_prefix("wallet:delete_confirm:") {
        bot.send_message(
            chat_id,
            "确认删除钱包？第一版执行软删除，密文仍保留以便恢复。",
        )
        .reply_markup(delete_confirm_keyboard(id.parse()?))
        .await?;
    } else if let Some(id) = data.strip_prefix("wallet:delete:") {
        state
            .wallet_service
            .delete_wallet(user_id, id.parse()?)
            .await?;
        bot.send_message(chat_id, "已删除钱包")
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else if let Some(id) = data.strip_prefix("watch:detail:") {
        let watch_id = id.parse()?;
        let item = state.db.get_watch_wallet(user_id, watch_id).await?;
        let wallet = state.db.active_wallet(user_id).await?;
        let strategy = state
            .db
            .get_or_create_strategy_for_watch(
                user_id,
                watch_id,
                wallet.as_ref().map(|wallet| wallet.id),
                &format!("跟单 {}", item.label),
                state.config.default_buy_amount_sol,
                state.config.default_slippage_bps,
                state.config.default_tip_sol,
            )
            .await?;
        state.follow_strategy.refresh_cache().await?;
        send_strategy_detail(bot, chat_id, &strategy).await?;
    } else if let Some(id) = data.strip_prefix("watch:copy:") {
        let item = state.db.get_watch_wallet(user_id, id.parse()?).await?;
        bot.send_message(chat_id, format!("<code>{}</code>", item.address))
            .reply_markup(watch_detail_keyboard(&item))
            .await?;
    } else if let Some(id) = data.strip_prefix("strategy:watch:") {
        let watch_id = id.parse()?;
        let item = state.db.get_watch_wallet(user_id, watch_id).await?;
        let wallet = state.db.active_wallet(user_id).await?;
        let strategy = state
            .db
            .get_or_create_strategy_for_watch(
                user_id,
                watch_id,
                wallet.as_ref().map(|wallet| wallet.id),
                &format!("跟单 {}", item.label),
                state.config.default_buy_amount_sol,
                state.config.default_slippage_bps,
                state.config.default_tip_sol,
            )
            .await?;
        state.follow_strategy.refresh_cache().await?;
        send_strategy_detail(bot, chat_id, &strategy).await?;
    } else if let Some(id) = data.strip_prefix("strategy:view:") {
        if let Some(strategy) = state.db.get_strategy(user_id, id.parse()?).await? {
            edit_strategy_detail(bot, message, &strategy).await?;
        }
    } else if let Some(id) = data.strip_prefix("strategy:live_confirm:") {
        confirm_strategy_live(bot, chat_id, state, user_id, id.parse()?).await?;
    } else if let Some(rest) = data.strip_prefix("strategy:toggle:") {
        let parts: Vec<_> = rest.split(':').collect();
        if parts.len() == 2 {
            let strategy_id: i64 = parts[0].parse()?;
            if let Some(strategy) = state.db.get_strategy(user_id, strategy_id).await? {
                if parts[1] == "dryrun" && strategy.dry_run {
                    request_strategy_live(bot, chat_id, state, user_id, strategy_id).await?;
                    return Ok(());
                }
                let (field, value) = match parts[1] {
                    "enabled" => ("enabled", if strategy.enabled { "0" } else { "1" }),
                    "dryrun" => ("dry_run", if strategy.dry_run { "0" } else { "1" }),
                    "copybuy" => ("copy_buy", if strategy.copy_buy { "0" } else { "1" }),
                    "copysell" => ("copy_sell", if strategy.copy_sell { "0" } else { "1" }),
                    _ => ("", ""),
                };
                if !field.is_empty() {
                    if field == "enabled" {
                        state
                            .db
                            .set_strategy_enabled_with_watch(user_id, strategy_id, value == "1")
                            .await?;
                    } else {
                        state
                            .db
                            .update_strategy_field(user_id, strategy_id, field, value)
                            .await?;
                    }
                    state.follow_strategy.refresh_cache().await?;
                    if let Some(updated) = state.db.get_strategy(user_id, strategy_id).await? {
                        send_strategy_detail(bot, chat_id, &updated).await?;
                    }
                }
            }
        }
    } else if let Some(rest) = data.strip_prefix("strategy:mode:") {
        let parts: Vec<_> = rest.split(':').collect();
        if parts.len() == 2 {
            let strategy_id: i64 = parts[0].parse()?;
            state
                .db
                .update_strategy_field(user_id, strategy_id, "buy_mode", parts[1])
                .await?;
            state.follow_strategy.refresh_cache().await?;
            if let Some(strategy) = state.db.get_strategy(user_id, strategy_id).await? {
                send_strategy_detail(bot, chat_id, &strategy).await?;
            }
        }
    } else if let Some(rest) = data.strip_prefix("strategy:input:") {
        let parts: Vec<_> = rest.split(':').collect();
        if parts.len() == 2 {
            let strategy_id: i64 = parts[0].parse()?;
            let field = parts[1].to_string();
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::StrategyInput {
                    strategy_id,
                    field: field.clone(),
                },
            );
            bot.send_message(chat_id, strategy_input_prompt(&field))
                .reply_markup(back_to_main_keyboard())
                .await?;
        }
    } else if let Some(id) = data.strip_prefix("strategy:mev:") {
        let strategy_id = id.parse()?;
        if let Some(strategy) = state.db.get_strategy(user_id, strategy_id).await? {
            bot.edit_message_text(chat_id, message.id(), "选择 MEV Provider")
                .reply_markup(strategy_mev_keyboard(&strategy))
                .await?;
        }
    } else if let Some(rest) = data.strip_prefix("strategy:set:") {
        let parts: Vec<_> = rest.split(':').collect();
        if parts.len() == 3 {
            let strategy_id: i64 = parts[0].parse()?;
            state
                .db
                .update_strategy_field(user_id, strategy_id, parts[1], parts[2])
                .await?;
            state.follow_strategy.refresh_cache().await?;
            if let Some(strategy) = state.db.get_strategy(user_id, strategy_id).await? {
                edit_strategy_detail(bot, message, &strategy).await?;
            }
        }
    } else if let Some(rest) = data.strip_prefix("limit:create_direction:") {
        let parts: Vec<_> = rest.split(':').collect();
        if parts.len() == 2 {
            let side = match parts[0] {
                "buy" => TradeSide::Buy,
                "sell" => TradeSide::Sell,
                _ => anyhow::bail!("unsupported limit side"),
            };
            let direction = match parts[1] {
                "below" => TriggerDirection::Below,
                "above" => TriggerDirection::Above,
                _ => anyhow::bail!("unsupported trigger direction"),
            };
            let side_label = if matches!(side, TradeSide::Buy) {
                "买入"
            } else {
                "卖出"
            };
            let amount_label = if matches!(side, TradeSide::Buy) {
                "买入SOL数量"
            } else {
                "卖出百分比"
            };
            let direction_label = direction.label();
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::CreateLimitOrder { side, direction },
            );
            bot.send_message(
                chat_id,
                format!(
                    "{direction_label}指定价格时{side_label}\n请发送：\n<code>mint USD触发价格 {amount_label}</code>"
                ),
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
        }
    } else if let Some(rest) = data.strip_prefix("trade:mint:") {
        let mut parts = rest.splitn(2, ':');
        let side = match parts.next() {
            Some("buy") => TradeSide::Buy,
            Some("sell") => TradeSide::Sell,
            _ => anyhow::bail!("unsupported trade side"),
        };
        let mint = parts.next().unwrap_or("");
        if !is_probable_wallet_address(mint) {
            anyhow::bail!("invalid mint in trade callback");
        }
        let prompt = if matches!(side, TradeSide::Buy) {
            "发送买入 SOL 数量，例如：\n<code>0.01</code>"
        } else {
            "发送卖出百分比，例如：\n<code>100</code>"
        };
        state.pending.lock().await.insert(
            telegram_id,
            PendingAction::ManualTradeAmount {
                side,
                mint: mint.to_string(),
            },
        );
        bot.send_message(chat_id, format!("{}\nMint: <code>{}</code>", prompt, mint))
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else if let Some(order_id) = data.strip_prefix("order:view:") {
        show_order_detail(bot, chat_id, state, user_id, order_id).await?;
    } else if let Some(order_id) = data.strip_prefix("limit:view:") {
        edit_limit_order_detail(bot, message, state, user_id, order_id).await?;
    } else if let Some(rest) = data.strip_prefix("limit:input:") {
        let parts: Vec<_> = rest.split(':').collect();
        if parts.len() == 2 {
            let order_id = parts[0].to_string();
            let field = parts[1].to_string();
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::LimitInput {
                    order_id,
                    field: field.clone(),
                },
            );
            bot.send_message(chat_id, limit_input_prompt(&field))
                .reply_markup(back_to_main_keyboard())
                .await?;
        }
    } else if let Some(order_id) = data.strip_prefix("limit:mev:") {
        if let Some(order) = state.db.get_limit_order(user_id, order_id).await? {
            bot.edit_message_text(chat_id, message.id(), "选择限价单 MEV Provider")
                .reply_markup(limit_mev_keyboard(&order))
                .await?;
        }
    } else if let Some(rest) = data.strip_prefix("limit:set:") {
        let parts: Vec<_> = rest.split(':').collect();
        if parts.len() == 3 {
            state
                .db
                .update_limit_order_execution_field(user_id, parts[0], parts[1], parts[2])
                .await?;
            edit_limit_order_detail(bot, message, state, user_id, parts[0]).await?;
        }
    } else if let Some(order_id) = data.strip_prefix("limit:live_confirm:") {
        confirm_limit_live(bot, chat_id, state, user_id, order_id).await?;
    } else if let Some(order_id) = data.strip_prefix("limit:mode:") {
        if let Some(order) = state.db.get_limit_order(user_id, order_id).await? {
            if order.dry_run {
                request_limit_live(bot, chat_id, state, user_id, order_id).await?;
                return Ok(());
            }
            let updated = state
                .db
                .set_limit_order_dry_run(user_id, order_id, true)
                .await?;
            if updated {
                show_limit_order_detail(bot, chat_id, state, user_id, order_id).await?;
            } else {
                bot.send_message(chat_id, "只有 pending 限价单可以切换模式")
                    .reply_markup(back_to_main_keyboard())
                    .await?;
            }
        } else {
            bot.send_message(chat_id, "限价单不存在")
                .reply_markup(back_to_main_keyboard())
                .await?;
        }
    } else if let Some(order_id) = data.strip_prefix("limit:cancel:") {
        let cancelled = state.db.cancel_limit_order(user_id, order_id).await?;
        let text = if cancelled {
            "已取消限价单"
        } else {
            "限价单不存在或已不是 pending"
        };
        bot.send_message(chat_id, text)
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else if let Some(id) = data.strip_prefix("watch:enable:") {
        state
            .db
            .set_watch_enabled(user_id, id.parse()?, true)
            .await?;
        state.follow_strategy.refresh_cache().await?;
        bot.send_message(chat_id, "已启用跟踪")
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else if let Some(id) = data.strip_prefix("watch:disable:") {
        state
            .db
            .set_watch_enabled(user_id, id.parse()?, false)
            .await?;
        state.follow_strategy.refresh_cache().await?;
        bot.send_message(chat_id, "已暂停跟踪")
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else if let Some(id) = data.strip_prefix("watch:rename:") {
        state.pending.lock().await.insert(
            telegram_id,
            PendingAction::RenameWatch {
                watch_id: id.parse()?,
            },
        );
        bot.send_message(chat_id, "请发送新的跟踪标签。")
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else if let Some(id) = data.strip_prefix("watch:delete_confirm:") {
        bot.send_message(chat_id, "确认删除这个跟踪钱包？")
            .reply_markup(watch_delete_confirm_keyboard(id.parse()?))
            .await?;
    } else if let Some(id) = data.strip_prefix("watch:delete:") {
        state.db.delete_watch_wallet(user_id, id.parse()?).await?;
        state.follow_strategy.refresh_cache().await?;
        bot.send_message(chat_id, "已删除跟踪钱包并暂停关联策略")
            .reply_markup(back_to_main_keyboard())
            .await?;
    } else {
        bot.send_message(chat_id, "未知操作")
            .reply_markup(back_to_main_keyboard())
            .await?;
    }
    Ok(())
}

async fn show_wallets(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: Option<MessageId>,
    state: &BotState,
    user_id: i64,
) -> Result<()> {
    let panel_message_id = if let Some(id) = message_id {
        bot.edit_message_text(chat_id, id, "正在读取钱包...")
            .reply_markup(back_to_main_keyboard())
            .await?;
        id
    } else {
        bot.send_message(chat_id, "正在读取钱包...")
            .reply_markup(back_to_main_keyboard())
            .await?
            .id
    };
    let budget = std::time::Duration::from_secs(5);
    let (synced, balances) = tokio::join!(
        bounded_wallet_read(
            state.wallet_service.sync_trade_service_wallets(user_id),
            budget
        ),
        bounded_wallet_read(state.wallet_service.balances(user_id), budget),
    );
    let wallets = state.db.list_wallets(user_id).await?;
    let mut lines = if synced.is_some() {
        vec![format!("钱包列表：{} 个", wallets.len())]
    } else {
        vec![
            "钱包服务暂不可用，请稍后重新打开。".to_string(),
            format!("已保存的钱包：{} 个（尚未核对最新状态）", wallets.len()),
        ]
    };
    for wallet in &wallets {
        let balance = balances.as_ref().and_then(|response| {
            wallet
                .external_wallet_id
                .as_deref()
                .and_then(|external_id| {
                    response
                        .wallets
                        .iter()
                        .find(|item| item.wallet_id == external_id)
                        .and_then(|item| item.balance_lamports)
                })
        });
        lines.push(format!(
            "\n{} {}\n<code>{}</code>\n余额: {}",
            if wallet.is_active { "✅" } else { "•" },
            escape_html(&wallet.label),
            escape_html(&wallet.public_key),
            format_sol_balance(balance)
        ));
    }
    let text = lines.join("\n");
    bot.edit_message_text(chat_id, panel_message_id, text)
        .reply_markup(wallet_list_keyboard(&wallets))
        .await?;
    Ok(())
}

async fn bounded_wallet_read<T>(
    request: impl std::future::Future<Output = Result<T>>,
    budget: std::time::Duration,
) -> Option<T> {
    tokio::time::timeout(budget, request).await.ok()?.ok()
}

async fn show_watch_wallets(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
) -> Result<()> {
    let items = state.db.list_watch_wallets(user_id).await?;
    bot.send_message(chat_id, format!("跟踪钱包：{} 个", items.len()))
        .reply_markup(watch_wallets_keyboard(&items))
        .await?;
    Ok(())
}

async fn show_preflight(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
) -> Result<()> {
    let report = build_preflight_report(state, user_id).await?;

    bot.send_message(chat_id, format_preflight_report(&report))
        .reply_markup(back_to_main_keyboard())
        .await?;
    Ok(())
}

async fn build_preflight_report(state: &BotState, user_id: i64) -> Result<PreflightReport> {
    preflight::build_report(
        &state.config,
        &state.db,
        &state.trade_client,
        &state.price_oracle,
        user_id,
    )
    .await
}

async fn request_strategy_live(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    strategy_id: i64,
) -> Result<()> {
    let Some(strategy) = state.db.get_strategy(user_id, strategy_id).await? else {
        bot.send_message(chat_id, "策略不存在")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    if !strategy.dry_run {
        send_strategy_detail(bot, chat_id, &strategy).await?;
        return Ok(());
    }

    let report = build_preflight_report(state, user_id).await?;
    if !report.is_live_ready() {
        bot.send_message(
            chat_id,
            format!("策略无法切换 Live\n\n{}", format_preflight_report(&report)),
        )
        .reply_markup(strategy_live_blocked_keyboard(strategy_id))
        .await?;
        return Ok(());
    }
    if let Some(reason) = target_wallet_block_reason(state, user_id, strategy.wallet_id).await {
        bot.send_message(
            chat_id,
            format!("策略无法切换 Live\n目标钱包检查: {}", escape_html(&reason)),
        )
        .reply_markup(strategy_live_blocked_keyboard(strategy_id))
        .await?;
        return Ok(());
    }

    bot.send_message(
        chat_id,
        format!(
            "实盘检查已通过\n\n确认将策略“{}”切换为 Live？切换后，新信号将进入实盘执行路由。",
            escape_html(&strategy.name)
        ),
    )
    .reply_markup(strategy_live_confirm_keyboard(strategy_id))
    .await?;
    Ok(())
}

async fn confirm_strategy_live(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    strategy_id: i64,
) -> Result<()> {
    let Some(strategy) = state.db.get_strategy(user_id, strategy_id).await? else {
        bot.send_message(chat_id, "策略不存在")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    if !strategy.dry_run {
        send_strategy_detail(bot, chat_id, &strategy).await?;
        return Ok(());
    }

    let report = build_preflight_report(state, user_id).await?;
    if !report.is_live_ready() {
        bot.send_message(
            chat_id,
            format!(
                "环境已变化，策略未切换 Live\n\n{}",
                format_preflight_report(&report)
            ),
        )
        .reply_markup(strategy_live_blocked_keyboard(strategy_id))
        .await?;
        return Ok(());
    }
    if let Some(reason) = target_wallet_block_reason(state, user_id, strategy.wallet_id).await {
        bot.send_message(
            chat_id,
            format!(
                "环境已变化，策略未切换 Live\n目标钱包检查: {}",
                escape_html(&reason)
            ),
        )
        .reply_markup(strategy_live_blocked_keyboard(strategy_id))
        .await?;
        return Ok(());
    }

    state
        .db
        .update_strategy_field(user_id, strategy_id, "dry_run", "0")
        .await?;
    state.follow_strategy.refresh_cache().await?;
    state
        .db
        .audit(
            Some(user_id),
            "strategy.live_enabled",
            Some("strategy"),
            Some(&strategy_id.to_string()),
            None,
        )
        .await?;
    if let Some(updated) = state.db.get_strategy(user_id, strategy_id).await? {
        bot.send_message(chat_id, "策略已切换为 Live").await?;
        send_strategy_detail(bot, chat_id, &updated).await?;
    }
    Ok(())
}

async fn request_limit_live(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    order_id: &str,
) -> Result<()> {
    let Some(order) = state.db.get_limit_order(user_id, order_id).await? else {
        bot.send_message(chat_id, "限价单不存在")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    if order.status != "pending" {
        bot.send_message(chat_id, "只有 pending 限价单可以切换模式")
            .reply_markup(limit_order_detail_keyboard(&order))
            .await?;
        return Ok(());
    }
    if !order.dry_run {
        show_limit_order_detail(bot, chat_id, state, user_id, order_id).await?;
        return Ok(());
    }

    let report = build_preflight_report(state, user_id).await?;
    if !report.is_live_ready() {
        bot.send_message(
            chat_id,
            format!(
                "限价单无法切换 Live\n\n{}",
                format_preflight_report(&report)
            ),
        )
        .reply_markup(limit_live_blocked_keyboard(order_id))
        .await?;
        return Ok(());
    }
    if let Some(reason) = target_wallet_block_reason(state, user_id, order.wallet_id).await {
        bot.send_message(
            chat_id,
            format!(
                "限价单无法切换 Live\n目标钱包检查: {}",
                escape_html(&reason)
            ),
        )
        .reply_markup(limit_live_blocked_keyboard(order_id))
        .await?;
        return Ok(());
    }

    bot.send_message(
        chat_id,
        format!(
            "实盘检查已通过\n\n确认将限价单切换为 Live？\nMint: <code>{}</code>\n触发后将进入实盘执行路由。",
            escape_html(&order.mint)
        ),
    )
    .reply_markup(limit_live_confirm_keyboard(order_id))
    .await?;
    Ok(())
}

async fn confirm_limit_live(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    order_id: &str,
) -> Result<()> {
    let Some(order) = state.db.get_limit_order(user_id, order_id).await? else {
        bot.send_message(chat_id, "限价单不存在")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    if order.status != "pending" {
        bot.send_message(chat_id, "只有 pending 限价单可以切换模式")
            .reply_markup(limit_order_detail_keyboard(&order))
            .await?;
        return Ok(());
    }
    if !order.dry_run {
        show_limit_order_detail(bot, chat_id, state, user_id, order_id).await?;
        return Ok(());
    }

    let report = build_preflight_report(state, user_id).await?;
    if !report.is_live_ready() {
        bot.send_message(
            chat_id,
            format!(
                "环境已变化，限价单未切换 Live\n\n{}",
                format_preflight_report(&report)
            ),
        )
        .reply_markup(limit_live_blocked_keyboard(order_id))
        .await?;
        return Ok(());
    }
    if let Some(reason) = target_wallet_block_reason(state, user_id, order.wallet_id).await {
        bot.send_message(
            chat_id,
            format!(
                "环境已变化，限价单未切换 Live\n目标钱包检查: {}",
                escape_html(&reason)
            ),
        )
        .reply_markup(limit_live_blocked_keyboard(order_id))
        .await?;
        return Ok(());
    }

    let updated = state
        .db
        .set_limit_order_dry_run(user_id, order_id, false)
        .await?;
    if !updated {
        bot.send_message(chat_id, "限价单状态已变化，未切换 Live")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    }
    state
        .db
        .audit(
            Some(user_id),
            "limit_order.live_enabled",
            Some("limit_order"),
            Some(order_id),
            None,
        )
        .await?;
    bot.send_message(chat_id, "限价单已切换为 Live").await?;
    show_limit_order_detail(bot, chat_id, state, user_id, order_id).await?;
    Ok(())
}

async fn target_wallet_block_reason(
    state: &BotState,
    user_id: i64,
    wallet_id: Option<i64>,
) -> Option<String> {
    let Some(wallet_id) = wallet_id else {
        return Some("未选择执行钱包".to_string());
    };
    let wallet = match state.db.get_wallet(user_id, wallet_id).await {
        Ok(wallet) => wallet,
        Err(err) => return Some(format!("本地钱包不可用: {err}")),
    };
    match state
        .trade_client
        .wallet_id_for_pubkey(&wallet.public_key)
        .await
    {
        Ok(Some(_)) => None,
        Ok(None) => Some(format!(
            "钱包 {} 未绑定到 trade_service 同公钥钱包",
            wallet.public_key
        )),
        Err(err) => Some(format!("查询 trade_service 钱包失败: {err}")),
    }
}

async fn show_orders(bot: &HtmlBot, chat_id: ChatId, state: &BotState, user_id: i64) -> Result<()> {
    let orders = state.db.list_recent_orders(user_id, 10).await?;
    let text = if orders.is_empty() {
        "暂无订单".to_string()
    } else {
        let mut lines = vec!["最近订单".to_string()];
        for order in &orders {
            let amount = order
                .amount_sol
                .map(|value| format!("{value} SOL"))
                .or_else(|| order.sell_percent.map(|value| format!("{value}%")))
                .or_else(|| quote_order_amount(order.request_json.as_deref()))
                .unwrap_or_else(|| "-".to_string());
            lines.push(format!(
                "{} {} {} {}\n<code>{}</code>",
                order.side, order.status, amount, order.created_at, order.mint
            ));
        }
        lines.join("\n\n")
    };

    bot.send_message(chat_id, text)
        .reply_markup(orders_keyboard(&orders))
        .await?;
    Ok(())
}

async fn show_trade_menu(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
) -> Result<()> {
    let wallet = state.db.active_wallet(user_id).await?;
    let wallet_text = wallet
        .as_ref()
        .map(|wallet| {
            format!(
                "{} <code>{}</code>",
                escape_html(&wallet.label),
                escape_html(&wallet.public_key)
            )
        })
        .unwrap_or_else(|| "未设置".to_string());
    bot.send_message(
        chat_id,
        format!(
            "快速交易\n\n执行钱包: {}\n默认滑点: {} bps\n默认小费: {} SOL",
            wallet_text, state.config.default_slippage_bps, state.config.default_tip_sol
        ),
    )
    .reply_markup(trade_menu_keyboard())
    .await?;
    Ok(())
}

async fn send_new_trade_panel(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    mint: &str,
    tab: PanelTab,
) -> Result<()> {
    let resolving = bot
        .send_message(chat_id, "正在解析 Mint 和 Pool...")
        .await?;
    let route = match resolve_pool_route(state, mint, false).await {
        Ok(route) => route,
        Err(err) => {
            let mut message = pool_resolution_error_text(&err).to_string();
            if state.config.isolated_sdk_simulation_enabled
                && state.trade_client.isolated_sdk_simulation_available()
            {
                message.push_str(
                    "\n如已知目标池地址，可发送 /pool &lt;Mint&gt; &lt;Pool ID&gt; &lt;协议&gt; 让隔离服务从链上核验。",
                );
            }
            bot.edit_message_text(chat_id, resolving.id, message)
                .reply_markup(back_to_main_keyboard())
                .await?;
            return Ok(());
        }
    };
    finish_new_trade_panel(
        bot,
        chat_id,
        resolving.id,
        state,
        user_id,
        telegram_id,
        mint,
        tab,
        route,
    )
    .await
}

async fn handle_explicit_pool_panel(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    args: &str,
) -> Result<()> {
    if !state.config.isolated_sdk_simulation_enabled
        || !state.trade_client.isolated_sdk_simulation_available()
    {
        bot.send_message(chat_id, "隔离 SDK 模拟未启用，当前不能指定池打开交易面板。")
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    }
    let mut parts = args.split_whitespace();
    let (Some(mint), Some(pool_id), Some(pool), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        bot.send_message(chat_id, "格式：/pool &lt;Mint&gt; &lt;Pool ID&gt; &lt;协议&gt;\n协议支持 pump-amm、raydium-cpmm、raydium-amm-v4。")
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    };
    let resolving = bot
        .send_message(chat_id, "正在从链上核验指定池与真实 Quote Mint...")
        .await?;
    let route = match bounded_panel_read(
        state
            .trade_client
            .isolated_sdk_resolve_explicit_pool(mint, pool, pool_id),
        std::time::Duration::from_secs(30),
        "指定池链上核验超时",
    )
    .await
    {
        Ok(route) => route,
        Err(error) => {
            tracing::warn!(?error, "explicit SDK pool resolution failed");
            bot.edit_message_text(
                chat_id,
                resolving.id,
                "指定池无法通过链上核验，请核对 Mint、Pool ID 和协议。尚未提交交易。",
            )
            .reply_markup(trade_menu_keyboard())
            .await?;
            return Ok(());
        }
    };
    finish_new_trade_panel(
        bot,
        chat_id,
        resolving.id,
        state,
        user_id,
        telegram_id,
        mint,
        PanelTab::Quick,
        route,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn finish_new_trade_panel(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    mint: &str,
    tab: PanelTab,
    route: ResolvedPoolRoute,
) -> Result<()> {
    bot.edit_message_text(chat_id, message_id, "Mint 和 Pool 已解析，正在读取钱包...")
        .await?;
    let mut session = match prepare_trade_panel(state, user_id, mint, tab, route).await {
        Ok(session) => session,
        Err(err) => {
            tracing::warn!(?err, "trade panel wallet preparation failed");
            bot.edit_message_text(
                chat_id,
                message_id,
                "交易面板暂不可用\n钱包或本地数据读取失败，请稍后重新发送 Mint。尚未提交交易。",
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
            return Ok(());
        }
    };
    if session.custom_quote() && session.tab == PanelTab::Limit {
        session.tab = PanelTab::Quick;
        session.notice = Some("自定义 Quote 限价单尚未支持".to_string());
    }
    session.message_id = Some(message_id);
    state.panels.lock().await.insert(telegram_id, session);
    render_trade_panel(bot, chat_id, message_id, state, user_id, telegram_id).await?;
    queue_panel_sol_hop(bot, state, chat_id, message_id, user_id, telegram_id).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn open_trade_panel_at_message(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    mint: &str,
    tab: PanelTab,
) -> Result<()> {
    bot.edit_message_text(chat_id, message_id, "正在解析 Mint 和 Pool...")
        .await?;
    let route = match resolve_pool_route(state, mint, false).await {
        Ok(route) => route,
        Err(err) => {
            bot.edit_message_text(chat_id, message_id, pool_resolution_error_text(&err))
                .reply_markup(back_to_main_keyboard())
                .await?;
            return Ok(());
        }
    };
    bot.edit_message_text(chat_id, message_id, "Mint 和 Pool 已解析，正在读取钱包...")
        .await?;
    let mut session = match prepare_trade_panel(state, user_id, mint, tab, route).await {
        Ok(session) => session,
        Err(err) => {
            tracing::warn!(?err, "trade panel wallet preparation failed");
            bot.edit_message_text(
                chat_id,
                message_id,
                "交易面板暂不可用\n钱包或本地数据读取失败，请稍后重新发送 Mint。尚未提交交易。",
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
            return Ok(());
        }
    };
    if session.custom_quote() && session.tab == PanelTab::Limit {
        session.tab = PanelTab::Quick;
        session.notice = Some("自定义 Quote 限价单尚未支持".to_string());
    }
    session.message_id = Some(message_id);
    state.panels.lock().await.insert(telegram_id, session);
    render_trade_panel(bot, chat_id, message_id, state, user_id, telegram_id).await?;
    queue_panel_sol_hop(bot, state, chat_id, message_id, user_id, telegram_id).await;
    Ok(())
}

async fn prepare_trade_panel(
    state: &BotState,
    user_id: i64,
    mint: &str,
    tab: PanelTab,
    route: ResolvedPoolRoute,
) -> Result<TradePanelSession> {
    bounded_panel_read(
        async {
            if !(state.config.isolated_sdk_simulation_enabled
                && state.trade_client.isolated_sdk_simulation_available())
            {
                state
                    .wallet_service
                    .sync_trade_service_wallets(user_id)
                    .await?;
            }
            let wallet = state.db.active_wallet(user_id).await?;
            let settings = load_user_trade_settings(state, user_id).await?;
            let mut panel = TradePanelSession::new(
                mint.to_string(),
                wallet.as_ref().map(|wallet| wallet.id),
                &settings,
                tab,
                route,
            );
            panel.isolated_sdk_simulation_enabled = state.config.isolated_sdk_simulation_enabled
                && state.trade_client.isolated_sdk_simulation_available();
            Ok(panel)
        },
        std::time::Duration::from_secs(5),
        "trade panel wallet preparation timeout",
    )
    .await
}

async fn refresh_panel_sol_hop(state: &BotState, session: &mut TradePanelSession) {
    session.sol_hop = None;
    session.sol_hop_error = None;
    if !session.needs_sol_hop_lookup() {
        return;
    }
    match bounded_panel_read(
        async {
            if session.route.pool == "raydium-cpmm" {
                state
                    .trade_client
                    .isolated_sdk_resolve_sol_hop(&session.route.quote_mint)
                    .await
            } else {
                state
                    .trade_client
                    .resolve_sol_hop(&session.route.quote_mint)
                    .await
            }
        },
        std::time::Duration::from_secs(12),
        "SOL 两跳查询超时",
    )
    .await
    {
        Ok(hop) => {
            if !hop.available {
                session.sol_hop_error = hop.reason.clone();
            }
            session.sol_hop = Some(hop);
        }
        Err(_) => session.sol_hop_error = Some("查询失败或超时，请稍后刷新".to_string()),
    }
}

fn apply_hop_result(session: &mut TradePanelSession, completed: &TradePanelSession) -> bool {
    if session.message_id != completed.message_id
        || session.route != completed.route
        || session.sol_hop_generation != completed.sol_hop_generation
    {
        return false;
    }
    session.sol_hop = completed.sol_hop.clone();
    session.sol_hop_error = completed.sol_hop_error.clone();
    session.sol_hop_pending = false;
    if session
        .sol_hop
        .as_ref()
        .is_some_and(|hop| hop.verified_pool_id().is_ok())
        && session.notice.as_deref().is_some_and(|notice| {
            notice.contains("路由不可用") || notice.contains("SOL 两跳不可用")
        })
    {
        session.notice =
            Some("SOL 两跳路由已就绪，请核对金额后重新确认；尚未提交交易。".to_string());
    }
    true
}

async fn save_trade_panel(state: &BotState, telegram_id: i64, mut session: TradePanelSession) {
    let mut panels = state.panels.lock().await;
    if let Some(current) = panels.get(&telegram_id) {
        if session.sol_hop_pending && !current.sol_hop_pending {
            apply_hop_result(&mut session, current);
        }
    }
    panels.insert(telegram_id, session);
}

async fn queue_panel_sol_hop(
    bot: &HtmlBot,
    state: &BotState,
    chat_id: ChatId,
    message_id: MessageId,
    user_id: i64,
    telegram_id: i64,
) {
    let mut snapshot = {
        let mut panels = state.panels.lock().await;
        let Some(session) = panels.get_mut(&telegram_id) else {
            return;
        };
        if session.message_id != Some(message_id)
            || !session.needs_sol_hop_lookup()
            || session.sol_hop_pending
            || session.sol_hop.is_some()
            || session.sol_hop_error.is_some()
        {
            return;
        }
        session.sol_hop_generation = session.sol_hop_generation.wrapping_add(1);
        session.sol_hop_pending = true;
        session.clone()
    };
    let bot = bot.clone();
    let state = state.clone();
    tokio::spawn(async move {
        refresh_panel_sol_hop(&state, &mut snapshot).await;
        let applied = {
            let mut panels = state.panels.lock().await;
            panels
                .get_mut(&telegram_id)
                .is_some_and(|session| apply_hop_result(session, &snapshot))
        };
        if applied {
            tracing::info!(
                available = snapshot.sol_hop.as_ref().is_some_and(|hop| hop.available),
                "background SOL hop completed"
            );
            if render_trade_panel(&bot, chat_id, message_id, &state, user_id, telegram_id)
                .await
                .is_err()
            {
                tracing::warn!("background SOL hop panel update failed");
            }
        }
    });
}

fn confirmation_matches(session: &TradePanelSession, callback: &str) -> bool {
    if callback == "panel:confirm" {
        return !session.quick.via_sol && !session.cpmm_sell_hop();
    }
    let Some(value) = callback.strip_prefix("panel:confirm:") else {
        return false;
    };
    let (side, settlement) = value.split_once(':').unwrap_or((value, "quote"));
    side == session.quick.side.as_str()
        && settlement
            == if session.quick.via_sol || session.cpmm_sell_hop() {
                "sol"
            } else {
                "quote"
            }
}

async fn bounded_panel_read<T>(
    read: impl std::future::Future<Output = Result<T>>,
    budget: std::time::Duration,
    timeout_context: &'static str,
) -> Result<T> {
    tokio::time::timeout(budget, read)
        .await
        .context(timeout_context)?
}

async fn render_trade_panel(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
) -> Result<()> {
    let Some(session) = state.panels.lock().await.get(&telegram_id).cloned() else {
        return Ok(());
    };
    let settings = load_user_trade_settings(state, user_id).await?;
    let wallet_label =
        panel_wallet_label(state, user_id, session.active_execution().wallet_id).await;
    let quote_balance = if session.direct_quote() && session.tab == PanelTab::Quick {
        match session.active_execution().wallet_id {
            Some(wallet_id) => match state.db.get_wallet(user_id, wallet_id).await {
                Ok(wallet) => match wallet.external_wallet_id.as_deref() {
                    Some(remote_id) => match bounded_panel_read(
                        state.trade_client.quote_balance(
                            user_id,
                            remote_id,
                            &session.route.quote_mint,
                        ),
                        std::time::Duration::from_secs(5),
                        "trade panel quote balance timeout",
                    )
                    .await
                    {
                        Ok(balance) => format!(
                            "\nQuote 余额: {} {}",
                            format_quote_units(balance.balance_units, balance.decimals),
                            session.quote_label()
                        ),
                        Err(_) => "\nQuote 余额: 查询失败".to_string(),
                    },
                    None => "\nQuote 余额: 钱包未绑定".to_string(),
                },
                Err(_) => "\nQuote 余额: 钱包不可用".to_string(),
            },
            None => "\nQuote 余额: 未选择钱包".to_string(),
        }
    } else {
        String::new()
    };
    let result = bot
        .edit_message_text(
            chat_id,
            message_id,
            format!(
                "{}{}",
                session.text(&wallet_label, settings.use_mev),
                quote_balance
            ),
        )
        .reply_markup(
            session.keyboard(
                &settings,
                state.config.isolated_sdk_simulation_enabled
                    && state.trade_client.isolated_sdk_simulation_available()
                    && session.quick.execution.method == ExecutionMethod::Sdk
                    && quick_trade_draft(&session, &settings)
                        .is_ok_and(|draft| isolated_sdk_simulation_supported(&draft)),
            ),
        )
        .await;
    if let Err(err) = result {
        if !err.to_string().contains("message is not modified") {
            return Err(err.into());
        }
    }
    Ok(())
}

async fn panel_wallet_label(state: &BotState, user_id: i64, wallet_id: Option<i64>) -> String {
    let Some(wallet_id) = wallet_id else {
        return "未设置".to_string();
    };
    match state.db.get_wallet(user_id, wallet_id).await {
        Ok(wallet) => {
            let remote_id = wallet
                .external_wallet_id
                .as_deref()
                .map(escape_html)
                .unwrap_or_else(|| "未绑定".to_string());
            format!(
                "{} | ID: <code>{}</code> | {}",
                escape_html(&wallet.label),
                remote_id,
                short_text(&wallet.public_key)
            )
        }
        Err(_) => "钱包不可用".to_string(),
    }
}

async fn handle_trade_panel_callback(
    bot: &HtmlBot,
    message: &MaybeInaccessibleMessage,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    data: &str,
) -> Result<()> {
    let chat_id = message.chat().id;
    let message_id = message.id();
    if data == "limit:panel:new" {
        state.pending.lock().await.insert(
            telegram_id,
            PendingAction::OpenLimitPanelMint {
                panel_message_id: message_id,
            },
        );
        return Ok(());
    }
    if !data.starts_with("panel:input:") && data != "panel:execute" {
        state.pending.lock().await.remove(&telegram_id);
    }

    let Some(mut session) = state.panels.lock().await.get(&telegram_id).cloned() else {
        bot.edit_message_text(chat_id, message_id, "面板已过期，请重新发送 Mint。")
            .reply_markup(main_menu_keyboard())
            .await?;
        return Ok(());
    };
    if session.message_id.is_some_and(|id| id != message_id) {
        bot.edit_message_text(chat_id, message_id, "面板已过期，请使用最新交易面板。")
            .reply_markup(main_menu_keyboard())
            .await?;
        return Ok(());
    }
    session.message_id = Some(message_id);
    tracing::info!(callback = data, "trade panel callback processing");
    let settings = load_user_trade_settings(state, user_id).await?;

    match data {
        "panel:back" => {
            state.panels.lock().await.remove(&telegram_id);
            state.pending.lock().await.remove(&telegram_id);
            bot.edit_message_text(chat_id, message_id, "主菜单")
                .reply_markup(main_menu_keyboard())
                .await?;
            return Ok(());
        }
        "panel:cancel_confirm" => {
            state.pending.lock().await.remove(&telegram_id);
        }
        "panel:execute" => {
            execute_pending_trade(bot, chat_id, message_id, state, user_id, telegram_id).await?;
            return Ok(());
        }
        "panel:tab:quick" => session.tab = PanelTab::Quick,
        "panel:settlement:quote" | "panel:settlement:sol" => {
            if let Err(error) = session.select_settlement(data == "panel:settlement:sol") {
                session.notice = Some(escape_html(&error.to_string()));
            } else {
                session.notice =
                    Some("结算方式已切换，请核对金额和模式；SOL 两跳仅支持 SDK".to_string());
            }
        }
        "panel:simulate"
            if session.tab == PanelTab::Quick
                && (session.quick.via_sol || state.config.isolated_sdk_simulation_enabled) =>
        {
            let mut draft = match quick_trade_draft(&session, &settings) {
                Ok(draft) => draft,
                Err(error) => {
                    session.notice = Some(format!("未提交：{}", escape_html(&error.to_string())));
                    save_trade_panel(state, telegram_id, session).await;
                    return render_trade_panel(
                        bot,
                        chat_id,
                        message_id,
                        state,
                        user_id,
                        telegram_id,
                    )
                    .await;
                }
            };
            if !session.quick.via_sol {
                anyhow::ensure!(
                    state.trade_client.isolated_sdk_simulation_available()
                        && draft.execution.method == ExecutionMethod::Sdk
                        && isolated_sdk_simulation_supported(&draft),
                    "当前路线不支持隔离 SDK 模拟"
                );
            }
            draft.execution.mode = ExecutionMode::Simulate;
            save_trade_panel(state, telegram_id, session).await;
            submit_quick_trade(bot, chat_id, message_id, state, user_id, telegram_id, draft)
                .await?;
            return Ok(());
        }
        "panel:tab:limit" => {
            if session.custom_quote() {
                session.notice = Some("自定义 Quote 限价单尚未支持，请使用快速交易".to_string());
            } else {
                session.tab = PanelTab::Limit;
            }
        }
        "panel:side:buy" => match session.tab {
            PanelTab::Quick => session.quick.side = PanelSide::Buy,
            PanelTab::Limit => session.limit.side = PanelSide::Buy,
        },
        "panel:side:sell" => match session.tab {
            PanelTab::Quick => session.quick.side = PanelSide::Sell,
            PanelTab::Limit => session.limit.side = PanelSide::Sell,
        },
        "panel:direction" => {
            session.limit.direction = match session.limit.direction {
                PanelDirection::Below => PanelDirection::Above,
                PanelDirection::Above => PanelDirection::Below,
            };
        }
        "panel:metric:price" => session.limit.metric = TriggerMetric::PriceUsd,
        "panel:metric:mcap" => session.limit.metric = TriggerMetric::MarketCapUsd,
        "panel:mode" => {
            session.limit.execution.mode = match session.limit.execution.mode {
                ExecutionMode::DryRun => ExecutionMode::Live,
                _ => ExecutionMode::DryRun,
            };
        }
        "panel:mev" if settings.use_mev => {
            save_trade_panel(state, telegram_id, session.clone()).await;
            bot.edit_message_text(chat_id, message_id, "选择 MEV Provider")
                .reply_markup(session.mev_provider_keyboard())
                .await?;
            return Ok(());
        }
        "panel:mev" => session.active_execution_mut().mev_mode = "none".to_string(),
        "panel:cancel_mev" => {}
        value if value.starts_with("panel:set_mev:") => {
            let provider = value.trim_start_matches("panel:set_mev:");
            if !MEV_PROVIDERS.contains(&provider) {
                anyhow::bail!("不支持的 MEV Provider");
            }
            session.active_execution_mut().mev_mode = provider.to_string();
        }
        "panel:expiry" => session.limit.expiry = session.limit.expiry.next(),
        "panel:wallet" => {
            state
                .wallet_service
                .sync_trade_service_wallets(user_id)
                .await?;
            let wallets = state.db.list_wallets(user_id).await?;
            let current = session.active_execution().wallet_id;
            save_trade_panel(state, telegram_id, session).await;
            bot.edit_message_text(chat_id, message_id, "选择交易钱包（括号内为 Trade API ID）")
                .reply_markup(trade_wallet_selection_keyboard(&wallets, current))
                .await?;
            return Ok(());
        }
        "panel:cancel_wallet" => {}
        value if value.starts_with("panel:set_wallet:") => {
            let wallet_id: i64 = value.trim_start_matches("panel:set_wallet:").parse()?;
            state.db.get_wallet(user_id, wallet_id).await?;
            session.quick.execution.wallet_id = Some(wallet_id);
            session.limit.execution.wallet_id = Some(wallet_id);
        }
        "panel:refresh" => {
            let previous_route = session.route.clone();
            session.route = resolve_pool_route(state, &session.mint, true).await?;
            if session.route != previous_route {
                session.quick.buy_amount_quote = None;
                session.quick.via_sol = session.custom_quote();
                session.quick.buy_amount_sol = 0.001;
            }
            if session.route != previous_route || !session.sol_hop_pending {
                session.sol_hop = None;
                session.sol_hop_error = None;
                session.sol_hop_pending = false;
                session.sol_hop_generation = session.sol_hop_generation.wrapping_add(1);
            }
            state
                .wallet_service
                .sync_trade_service_wallets(user_id)
                .await?;
            if let Some(wallet_id) = session.active_execution().wallet_id {
                if state.db.get_wallet(user_id, wallet_id).await.is_err() {
                    let fallback = state.db.active_wallet(user_id).await?;
                    session.quick.execution.wallet_id = fallback.as_ref().map(|wallet| wallet.id);
                    session.limit.execution.wallet_id = fallback.map(|wallet| wallet.id);
                }
            }
            for execution in [&mut session.quick.execution, &mut session.limit.execution] {
                execution.mode = ExecutionMode::from_db(&settings.default_execution_mode, true);
                execution.method = ExecutionMethod::from_db(&settings.default_execution_method);
                execution.slippage_bps = settings.default_slippage_bps;
                execution.tip_sol = settings.default_tip_sol;
                execution.mev_mode = if settings.use_mev {
                    settings.default_mev_provider.clone()
                } else {
                    "none".to_string()
                };
            }
        }
        "panel:input:amount"
        | "panel:input:trigger"
        | "panel:input:slippage"
        | "panel:input:tip" => {
            let field = match data {
                "panel:input:amount" => PanelInputField::Amount,
                "panel:input:trigger" => PanelInputField::TriggerValue,
                "panel:input:slippage" => PanelInputField::Slippage,
                _ => PanelInputField::Tip,
            };
            save_trade_panel(state, telegram_id, session).await;
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::PanelInput {
                    field,
                    panel_message_id: message_id,
                },
            );
            return Ok(());
        }
        value if value == "panel:confirm" || value.starts_with("panel:confirm:") => {
            if !confirmation_matches(&session, value) {
                session.notice = Some("旧确认按钮已失效：方向或结算方式已变化，请使用下方当前按钮重新确认。尚未提交交易。".to_string());
                save_trade_panel(state, telegram_id, session).await;
                return render_trade_panel(bot, chat_id, message_id, state, user_id, telegram_id)
                    .await;
            }
            let draft = match quick_trade_draft(&session, &settings) {
                Ok(draft) => draft,
                Err(error) => {
                    session.notice = Some(format!("未提交：{}", escape_html(&error.to_string())));
                    save_trade_panel(state, telegram_id, session).await;
                    return render_trade_panel(
                        bot,
                        chat_id,
                        message_id,
                        state,
                        user_id,
                        telegram_id,
                    )
                    .await;
                }
            };
            save_trade_panel(state, telegram_id, session).await;
            submit_quick_trade(bot, chat_id, message_id, state, user_id, telegram_id, draft)
                .await?;
            return Ok(());
        }
        "panel:create_limit" => {
            if session.custom_quote() {
                anyhow::bail!("自定义 Quote 限价单尚未支持");
            }
            create_limit_order_from_panel(
                bot,
                chat_id,
                message_id,
                state,
                user_id,
                telegram_id,
                &session,
            )
            .await?;
            return Ok(());
        }
        _ if data.starts_with("panel:preset:") => {
            if session.direct_quote() && session.active_side() == PanelSide::Buy {
                return Ok(());
            }
            let parts = data.split(':').collect::<Vec<_>>();
            let (expected_side, index_text) = match parts.as_slice() {
                ["panel", "preset", side, index] => (Some(*side), *index),
                ["panel", "preset", index] => (None, *index),
                _ => anyhow::bail!("invalid preset callback"),
            };
            if expected_side.is_some_and(|side| side != session.active_side().as_str()) {
                return Ok(());
            }
            let index: usize = index_text.parse()?;
            let presets = if session.active_side() == PanelSide::Buy {
                settings.buy_presets_sol
            } else {
                settings.sell_presets_percent
            };
            let Some(value) = presets.get(index).copied() else {
                anyhow::bail!("invalid preset index");
            };
            session.set_active_amount(value);
            if session.tab == PanelTab::Quick
                && !settings.quick_trade_confirmation_enabled
                && !session.quick.via_sol
                && !session.cpmm_sell_hop()
            {
                let draft = quick_trade_draft(&session, &settings)?;
                save_trade_panel(state, telegram_id, session).await;
                submit_quick_trade(bot, chat_id, message_id, state, user_id, telegram_id, draft)
                    .await?;
                return Ok(());
            }
        }
        _ => return Ok(()),
    }

    save_trade_panel(state, telegram_id, session).await;
    render_trade_panel(bot, chat_id, message_id, state, user_id, telegram_id).await?;
    queue_panel_sol_hop(bot, state, chat_id, message_id, user_id, telegram_id).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_trade_panel_input(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    panel_message_id: MessageId,
    field: PanelInputField,
    text: &str,
) -> Result<()> {
    let value = text.trim().parse::<f64>()?;
    let settings = load_user_trade_settings(state, user_id).await?;
    let immediate_draft = {
        let mut panels = state.panels.lock().await;
        let Some(session) = panels.get_mut(&telegram_id) else {
            anyhow::bail!("交易面板已过期");
        };
        match field {
            PanelInputField::Amount => {
                if session.direct_quote()
                    && session.tab == PanelTab::Quick
                    && session.active_side() == PanelSide::Buy
                {
                    // Preserve the original decimal text until the on-chain mint precision is known.
                    if text.trim().contains('e') || text.trim().contains('E') {
                        anyhow::bail!("Quote 金额请使用普通小数格式");
                    }
                }
                if !value.is_finite()
                    || value <= 0.0
                    || (session.active_side() == PanelSide::Sell
                        && (value > 100.0 || value.fract() != 0.0))
                {
                    anyhow::bail!("买入金额必须大于 0；卖出比例必须是 1 到 100 的整数");
                }
                if session.direct_quote()
                    && session.tab == PanelTab::Quick
                    && session.active_side() == PanelSide::Buy
                {
                    session.quick.buy_amount_quote = Some(text.trim().to_string());
                } else {
                    session.set_active_amount(value);
                }
            }
            PanelInputField::TriggerValue => {
                if value <= 0.0 || !value.is_finite() {
                    anyhow::bail!("触发数值必须大于 0");
                }
                session.set_active_trigger_value(value);
            }
            PanelInputField::Slippage => {
                if !(0.0..=100.0).contains(&value) {
                    anyhow::bail!("滑点必须在 0% 到 100% 之间");
                }
                session.active_execution_mut().slippage_bps = (value * 100.0).round() as u64;
            }
            PanelInputField::Tip => {
                if value < 0.0 || !value.is_finite() {
                    anyhow::bail!("小费必须大于或等于 0");
                }
                session.active_execution_mut().tip_sol = value;
            }
        }
        if matches!(field, PanelInputField::Amount)
            && session.tab == PanelTab::Quick
            && !settings.quick_trade_confirmation_enabled
            && !session.quick.via_sol
            && !session.cpmm_sell_hop()
        {
            Some(quick_trade_draft(session, &settings)?)
        } else {
            None
        }
    };
    if let Some(draft) = immediate_draft {
        return submit_quick_trade(
            bot,
            chat_id,
            panel_message_id,
            state,
            user_id,
            telegram_id,
            draft,
        )
        .await;
    }
    render_trade_panel(bot, chat_id, panel_message_id, state, user_id, telegram_id).await
}

async fn create_limit_order_from_panel(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    session: &TradePanelSession,
) -> Result<()> {
    let trigger_value = session
        .active_trigger_value()
        .ok_or_else(|| anyhow::anyhow!("请先设置触发价格或市值"))?;
    let side = session.limit.side;
    let direction = session.limit.direction;
    let action = match (side, direction) {
        (PanelSide::Buy, PanelDirection::Below) => LimitAction::BuyBelow,
        (PanelSide::Buy, PanelDirection::Above) => LimitAction::BuyAbove,
        (PanelSide::Sell, PanelDirection::Below) => LimitAction::SellBelow,
        (PanelSide::Sell, PanelDirection::Above) => LimitAction::SellAbove,
    };
    let amount = if side == PanelSide::Buy {
        session.limit.buy_amount_sol
    } else {
        session.limit.sell_percent
    };
    let expires_at = session.limit.expiry.expires_at();
    let settings = load_user_trade_settings(state, user_id).await?;
    let mut execution = session.limit.execution.clone();
    if !settings.use_mev {
        execution.mev_mode = "none".to_string();
    }
    let order_id = state
        .db
        .insert_limit_order(
            user_id,
            session.limit.execution.wallet_id,
            &session.mint,
            action,
            session.limit.metric.as_str(),
            trigger_value,
            (side == PanelSide::Buy).then_some(amount),
            (side == PanelSide::Sell).then_some(amount),
            &execution,
            expires_at.as_deref(),
        )
        .await?;
    state.panels.lock().await.remove(&telegram_id);
    bot.edit_message_text(
        chat_id,
        message_id,
        format!(
            "限价单已创建\n订单: <code>{}</code>\n动作: {}\n触发依据: {}\n触发数值: {} USD\n数量: {} {}\nMint: <code>{}</code>",
            order_id,
            action.label(),
            if session.limit.metric == TriggerMetric::PriceUsd { "价格" } else { "市值" },
            trade_panel::trim_float(trigger_value),
            trade_panel::trim_float(amount),
            if side == PanelSide::Buy { "SOL" } else { "%" },
            session.mint,
        ),
    )
    .reply_markup(limit_order_detail_keyboard(
        &state
            .db
            .get_limit_order(user_id, &order_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("limit order missing after insert"))?,
    ))
    .await?;
    Ok(())
}

fn validate_trade_draft(draft: &TradeDraft) -> Result<()> {
    if let Some(id) = draft.sol_hop_pool_id.as_deref() {
        anyhow::ensure!(
            custom_quote_route(&draft.route) || isolated_sdk_bridge_sell_supported(draft),
            "SOL 两跳仅支持已核验的 SDK 路线"
        );
        anyhow::ensure!(
            draft.execution.method == ExecutionMethod::Sdk,
            "SOL 两跳请使用 SDK 执行方式"
        );
        anyhow::ensure!(
            draft.quote_amount.is_none(),
            "SOL 两跳不能同时传 Quote 金额"
        );
        anyhow::ensure!(
            bs58::decode(id)
                .into_vec()
                .is_ok_and(|bytes| bytes.len() == 32),
            "SOL 两跳池地址无效"
        );
        if matches!(draft.side, TradeSide::Sell) {
            anyhow::ensure!(
                draft.amount.is_finite()
                    && draft.amount.fract() == 0.0
                    && (1.0..=100.0).contains(&draft.amount),
                "卖出比例必须是 1 到 100 的整数"
            );
        }
    }
    if matches!(draft.side, TradeSide::Buy)
        && custom_quote_route(&draft.route)
        && draft.sol_hop_pool_id.is_none()
    {
        if draft.quote_amount.as_deref().unwrap_or("").is_empty() {
            anyhow::bail!("请先输入 Quote 买入金额");
        }
        if draft.execution.method != ExecutionMethod::Sdk {
            anyhow::bail!("自定义 Quote 交易请使用 SDK 执行方式");
        }
        return draft.execution.validate();
    }
    let size = match draft.side {
        TradeSide::Buy => OrderSize::FixedBuySol {
            amount_sol: draft.amount,
        },
        TradeSide::Sell => OrderSize::SellPercent {
            percent: draft.amount.round() as u64,
        },
    };
    size.validate()?;
    draft.execution.validate()
}

fn quick_trade_draft(
    session: &TradePanelSession,
    settings: &UserTradeSettings,
) -> Result<TradeDraft> {
    let side = match session.quick.side {
        PanelSide::Buy => TradeSide::Buy,
        PanelSide::Sell => TradeSide::Sell,
    };
    let amount = match session.quick.side {
        PanelSide::Buy => session.quick.buy_amount_sol,
        PanelSide::Sell => session.quick.sell_percent,
    };
    let mut execution = session.quick.execution.clone();
    if !settings.use_mev {
        execution.mev_mode = "none".to_string();
    }
    let draft = TradeDraft {
        side,
        mint: session.mint.clone(),
        amount,
        execution,
        route: session.route.clone(),
        quote_amount: if session.quick.side == PanelSide::Buy && session.direct_quote() {
            session.quick.buy_amount_quote.clone()
        } else {
            None
        },
        sol_hop_pool_id: if session.quick.via_sol || session.cpmm_sell_hop() {
            anyhow::ensure!(
                session.tab == PanelTab::Quick
                    && (session.custom_quote() || session.cpmm_sell_hop()),
                "SOL 两跳仅支持快速交易"
            );
            Some(
                session
                    .sol_hop
                    .as_ref()
                    .context("SOL 两跳路由不可用，请刷新")?
                    .verified_pool_id()?
                    .to_string(),
            )
        } else {
            None
        },
    };
    validate_trade_draft(&draft)?;
    Ok(draft)
}

fn manual_trade_request(
    draft: &TradeDraft,
    execution: &ExecutionParams,
    trade_service_wallet_id: String,
    quote_units: Option<u64>,
    idempotency_key: &str,
) -> Result<TradeRequest> {
    validate_trade_draft(draft)?;
    execution.validate()?;
    anyhow::ensure!(
        draft.sol_hop_pool_id.is_none() || execution.method == ExecutionMethod::Sdk,
        "SOL 两跳请使用 SDK 执行方式"
    );
    anyhow::ensure!(
        draft.sol_hop_pool_id.is_none() || quote_units.is_none(),
        "SOL 两跳不能传 Quote 金额"
    );
    if matches!(draft.side, TradeSide::Buy)
        && custom_quote_route(&draft.route)
        && draft.sol_hop_pool_id.is_none()
    {
        anyhow::ensure!(
            quote_units.is_some_and(|units| units > 0),
            "缺少有效 Quote 最小单位金额"
        );
    }
    let (use_mev, mev_provider) = manual_mev_settings(&execution.mev_mode)?;
    let bridge_buy =
        execution.mode == ExecutionMode::Simulate && isolated_sdk_bridge_buy_supported(draft);
    let bridge_sell =
        execution.mode == ExecutionMode::Simulate && isolated_sdk_bridge_sell_supported(draft);
    anyhow::ensure!(
        !(bridge_buy || bridge_sell) || !use_mev,
        "隔离 SDK 两跳模拟不支持 MEV 设置"
    );
    Ok(TradeRequest {
        wallet_id: Some(trade_service_wallet_id),
        execution_method: execution.method.as_str().to_string(),
        side: draft.side.as_str().to_string(),
        pool: draft.route.pool.clone(),
        mint: draft.mint.clone(),
        pool_id: crate::executor::trade_service_pool_id(
            &draft.route.pool,
            draft.route.pool_id.as_deref(),
        ),
        quote_mint: Some(draft.route.quote_mint.clone()),
        amount_quote_units: quote_units,
        settlement_asset: draft.sol_hop_pool_id.as_ref().map(|_| "sol".to_string()),
        sol_hop_pool: draft
            .sol_hop_pool_id
            .as_ref()
            .map(|_| "raydium-cpmm".to_string()),
        sol_hop_pool_id: draft.sol_hop_pool_id.clone(),
        sdk_decision_id: None,
        sdk_preview_id: None,
        amount_sol: (matches!(draft.side, TradeSide::Buy) && quote_units.is_none())
            .then_some(draft.amount),
        sell_percent: matches!(draft.side, TradeSide::Sell).then_some(draft.amount.round() as u64),
        slippage_percent: execution.slippage_bps as f64 / 100.0,
        tip_sol: (!(bridge_buy || bridge_sell)).then_some(execution.tip_sol),
        use_mev: (!(bridge_buy || bridge_sell)).then_some(use_mev),
        mev_provider: (!(bridge_buy || bridge_sell))
            .then_some(mev_provider)
            .flatten(),
        simulate: execution.mode == ExecutionMode::Simulate,
        source: Some("copy-trading-manual".to_string()),
        source_signature: None,
        idempotency_key: idempotency_key.to_string(),
    })
}

fn validate_sol_hop_refresh(
    draft: &TradeDraft,
    route: &ResolvedPoolRoute,
    hop: &crate::trade_client::SolHopResponse,
) -> Result<()> {
    anyhow::ensure!(*route == draft.route, "池或 Quote 已变化，请刷新交易面板");
    anyhow::ensure!(
        Some(hop.verified_pool_id()?) == draft.sol_hop_pool_id.as_deref(),
        "SOL 两跳池已变化，请刷新交易面板"
    );
    Ok(())
}

fn manual_mev_settings(mev_mode: &str) -> Result<(bool, Option<String>)> {
    match mev_mode {
        "none" => Ok((false, None)),
        "auto" => Ok((true, None)),
        "default" | "rpc" | "jito" | "temporal" | "flashblock" | "blockrazor" | "astralane"
        | "speedlanding" => Ok((true, Some(mev_mode.to_string()))),
        value => anyhow::bail!("unsupported MEV provider {value}"),
    }
}

async fn show_order_detail(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    order_id: &str,
) -> Result<()> {
    let Some(order) = state.db.get_order_detail(user_id, order_id).await? else {
        bot.send_message(chat_id, "订单不存在")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };

    let amount = order
        .amount_sol
        .map(|value| format!("{value} SOL"))
        .or_else(|| order.sell_percent.map(|value| format!("{value}%")))
        .or_else(|| quote_order_amount(order.request_json.as_deref()))
        .unwrap_or_else(|| "-".to_string());
    let request_summary = order
        .request_json
        .as_deref()
        .map(format_json_summary)
        .unwrap_or_else(|| "-".to_string());
    let response_summary = order
        .response_json
        .as_deref()
        .map(format_json_summary)
        .unwrap_or_else(|| "-".to_string());
    let text = format!(
        "订单详情\nID: <code>{}</code>\n状态: {}\n动作: {}\n数量: {}\nMint: <code>{}</code>\n钱包ID: {}\n策略ID: {}\n信号ID: {}\n幂等键: <code>{}</code>\ntrade_service请求: <code>{}</code>\n签名: <code>{}</code>\n错误: {}\n创建: {}\n更新: {}\n\nRequest:\n<code>{}</code>\n\nResponse:\n<code>{}</code>",
        order.id,
        order.status,
        order.side,
        amount,
        order.mint,
        optional_i64(order.wallet_id),
        optional_i64(order.strategy_id),
        optional_i64(order.signal_id),
        order.idempotency_key,
        order.trade_service_request_id.as_deref().unwrap_or("-"),
        order.tx_signature.as_deref().unwrap_or("-"),
        escape_html(order.error.as_deref().unwrap_or("-")),
        order.created_at,
        order.updated_at,
        request_summary,
        response_summary
    );

    bot.send_message(chat_id, text)
        .reply_markup(order_detail_keyboard())
        .await?;
    Ok(())
}

async fn show_positions(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
) -> Result<()> {
    let positions = state.db.list_open_dry_run_positions(user_id, 20).await?;
    let text = if positions.is_empty() {
        "暂无 dry-run 持仓".to_string()
    } else {
        let mut lines = vec!["Dry-run 持仓".to_string()];
        for position in positions {
            lines.push(format!(
                "{} SOL | 买 {} 次 | 卖 {} 次 | {}\n<code>{}</code>",
                format_optional_number(Some(position.amount_sol)),
                position.buy_count,
                position.sell_count,
                position.updated_at,
                position.mint
            ));
        }
        lines.join("\n\n")
    };

    bot.send_message(chat_id, text)
        .reply_markup(back_to_main_keyboard())
        .await?;
    Ok(())
}

async fn show_limit_orders(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
) -> Result<()> {
    let orders = state.db.list_limit_orders(user_id, 20).await?;
    let wallet = state.db.active_wallet(user_id).await?;
    let wallet_text = wallet
        .as_ref()
        .map(|wallet| {
            format!(
                "{} {}",
                escape_html(&wallet.label),
                short_text(&wallet.public_key)
            )
        })
        .unwrap_or_else(|| "未设置".to_string());
    let sol_usd = state
        .price_oracle
        .sol_usd_price()
        .await
        .or(state.config.sol_usd_price)
        .map(|price| format!("{} USD", format_optional_number(Some(price))))
        .unwrap_or_else(|| "不可用".to_string());
    let text = if orders.is_empty() {
        format!(
            "限价单\n\n执行钱包: {wallet_text}\n价格基准: USD\nSOL/USD: {sol_usd}\n默认模式: Dry-run\n\n暂无限价单"
        )
    } else {
        let mut lines = vec![format!(
            "限价单\n\n执行钱包: {wallet_text}\n价格基准: USD\nSOL/USD: {sol_usd}\n默认模式: Dry-run"
        )];
        for order in &orders {
            let amount = order
                .amount_sol
                .map(|value| format!("{value} SOL"))
                .or_else(|| order.sell_percent.map(|value| format!("{value}%")))
                .unwrap_or_else(|| "-".to_string());
            lines.push(format!(
                "{} {} {} | {} {} {} | {} {}\n{}\n<code>{}</code>",
                order.action().label(),
                order.status,
                if order.dry_run { "Dry-run" } else { "Live" },
                if order.trigger_metric == "market_cap_usd" {
                    "触发市值"
                } else {
                    "触发价格"
                },
                format_optional_number(Some(order.trigger_price)),
                order.trigger_currency,
                amount,
                order.created_at,
                format_limit_trigger_line(
                    order.triggered_price,
                    order.triggered_currency.as_deref(),
                    order.triggered_source_event_id,
                    order.triggered_at.as_deref()
                ),
                order.mint
            ));
        }
        lines.join("\n\n")
    };

    bot.send_message(chat_id, text)
        .reply_markup(limit_orders_keyboard(&orders))
        .await?;
    Ok(())
}

async fn show_limit_order_detail(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    order_id: &str,
) -> Result<()> {
    let Some((text, order)) = limit_order_detail_content(state, user_id, order_id).await? else {
        bot.send_message(chat_id, "限价单不存在")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    bot.send_message(chat_id, text)
        .reply_markup(limit_order_detail_keyboard(&order))
        .await?;
    Ok(())
}

async fn edit_limit_order_detail(
    bot: &HtmlBot,
    message: &MaybeInaccessibleMessage,
    state: &BotState,
    user_id: i64,
    order_id: &str,
) -> Result<()> {
    let Some((text, order)) = limit_order_detail_content(state, user_id, order_id).await? else {
        bot.edit_message_text(message.chat().id, message.id(), "限价单不存在")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    bot.edit_message_text(message.chat().id, message.id(), text)
        .reply_markup(limit_order_detail_keyboard(&order))
        .await?;
    Ok(())
}

async fn limit_order_detail_content(
    state: &BotState,
    user_id: i64,
    order_id: &str,
) -> Result<Option<(String, crate::db::models::LimitOrderSummary)>> {
    let Some(order) = state.db.get_limit_order(user_id, order_id).await? else {
        return Ok(None);
    };
    let linked_order = state
        .db
        .find_order_for_limit_order(user_id, &order.id)
        .await?;
    let amount = order
        .amount_sol
        .map(|value| format!("{value} SOL"))
        .or_else(|| order.sell_percent.map(|value| format!("{value}%")))
        .unwrap_or_else(|| "-".to_string());
    let linked = linked_order
        .map(|item| {
            format!(
                "{} {} {}\n<code>{}</code>",
                item.side, item.status, item.created_at, item.id
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let text = format!(
        "限价单详情\nID: <code>{}</code>\n状态: {}\n模式: {}\n动作: {}\n触发依据: {}\n触发数值: {} {}\n数量: {}\n滑点: {}%\n小费: {} SOL\nMEV: {}\n执行方式: {}\n有效期: {}\nMint: <code>{}</code>\n钱包ID: {}\n创建: {}\n{}\n\n关联订单:\n{}",
        order.id,
        order.status,
        if order.dry_run { "Dry-run" } else { "Live" },
        order.action().label(),
        if order.trigger_metric == "market_cap_usd" {
            "市值"
        } else {
            "价格"
        },
        format_optional_number(Some(order.trigger_price)),
        order.trigger_currency,
        amount,
        format_optional_number(Some(order.slippage_bps as f64 / 100.0)),
        order.tip_sol,
        order.mev_mode,
        order.execution_method,
        order.expires_at.as_deref().unwrap_or("不限"),
        order.mint,
        optional_i64(order.wallet_id),
        order.created_at,
        format_limit_trigger_line(
            order.triggered_price,
            order.triggered_currency.as_deref(),
            order.triggered_source_event_id,
            order.triggered_at.as_deref()
        ),
        linked
    );

    Ok(Some((text, order)))
}

async fn show_strategies(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
) -> Result<()> {
    let strategies = state.db.list_strategies(user_id).await?;
    let text = if strategies.is_empty() {
        "暂无策略。命中 watch 信号时会自动创建默认 dry-run 策略。".to_string()
    } else {
        let mut lines = vec!["策略列表".to_string()];
        for strategy in strategies {
            let buy_size = if strategy.buy_mode == "percentage" {
                format!("{}%", strategy.buy_ratio_percent)
            } else {
                format!("{} SOL", strategy.buy_amount_sol)
            };
            let max_delay = strategy
                .max_signal_delay_ms
                .map(|value| format_delay(Some(value)))
                .unwrap_or_else(|| "不限".to_string());
            lines.push(format!(
                "#{} {}\n启用: {} | 模式: {}\n跟买: {} | 跟卖: {}\n买入: {} | 卖出: {}%\n滑点: {}% | 小费: {} SOL | MEV: {}\n最大延迟: {}",
                strategy.id,
                strategy.name,
                strategy.enabled,
                if strategy.dry_run { "Dry-run" } else { "Live" },
                strategy.copy_buy,
                strategy.copy_sell,
                buy_size,
                strategy.sell_percent.unwrap_or(100.0),
                strategy.max_slippage_bps as f64 / 100.0,
                strategy.tip_sol,
                strategy.mev_mode,
                max_delay
            ));
        }
        lines.join("\n\n")
    };
    bot.send_message(chat_id, text)
        .reply_markup(back_to_main_keyboard())
        .await?;
    Ok(())
}

async fn show_signals(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
) -> Result<()> {
    let signals = state.db.list_recent_signals(user_id, 10).await?;
    let text = if signals.is_empty() {
        "暂无跟单信号".to_string()
    } else {
        let mut lines = vec!["最近跟单信号".to_string()];
        for signal in signals {
            let details = signal
                .metadata_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<MintSignalDetails>(value).ok());
            if let Some(details) = details {
                let filter_reason = details
                    .filter_reason
                    .as_deref()
                    .map(|reason| format!("\n过滤: {}", reason))
                    .unwrap_or_default();
                lines.push(format!(
                    "#{} {} {} {}\nMint: <code>{}</code>\nPool: {} | Quote: <code>{}</code>\nSOL池: {} | Token池: {}\n目标买入: {} SOL | 市值: {}\n链上→接收: {} 参考\n接收→通知: {}\n签名: <code>{}</code>{}",
                    signal.id,
                    signal.side,
                    signal.status,
                    signal.created_at,
                    details.mint,
                    details.pool.as_deref().unwrap_or("-"),
                    details.quote_mint.as_deref().unwrap_or("-"),
                    format_optional_number(details.sol_in_pool),
                    format_optional_number(details.tokens_in_pool),
                    format_optional_number(details.target_buy_sol),
                    format_optional_number(details.market_cap_quote),
                    format_delay(details.ingest_delay_ms),
                    format_delay(details.notification_delay_ms),
                    details.signature.as_deref().unwrap_or("-"),
                    filter_reason
                ));
            } else {
                lines.push(format!(
                    "#{} {} {} {}\nMint: <code>{}</code>\n{}",
                    signal.id,
                    signal.side,
                    signal.status,
                    signal.created_at,
                    signal.mint,
                    signal.reason
                ));
            }
        }
        lines.join("\n\n")
    };

    bot.send_message(chat_id, text)
        .reply_markup(back_to_main_keyboard())
        .await?;
    Ok(())
}

fn starts_new_trade_panel(action: &PendingAction, text: &str) -> bool {
    is_probable_wallet_address(text)
        && matches!(
            action,
            PendingAction::ManualTrade { .. }
                | PendingAction::ManualTradeAmount { .. }
                | PendingAction::ConfirmTrade { .. }
                | PendingAction::PanelInput { .. }
        )
}

async fn handle_pending_message(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    action: PendingAction,
    text: &str,
    user_message_id: MessageId,
) -> Result<()> {
    match action {
        PendingAction::PnlDate {
            target,
            panel_message_id,
            panel_chat_id,
        } => {
            if chat_id != panel_chat_id {
                state.pending.lock().await.insert(
                    telegram_id,
                    PendingAction::PnlDate {
                        target,
                        panel_message_id,
                        panel_chat_id,
                    },
                );
                return Ok(());
            }
            pnl::input_date(
                bot,
                chat_id,
                state,
                user_id,
                telegram_id,
                target,
                panel_message_id,
                text,
            )
            .await?;
        }
        PendingAction::ImportWallet => {
            let wallet = state
                .wallet_service
                .import_wallet(user_id, text, Some("Imported"))
                .await?;
            bot.send_message(
                chat_id,
                format!("导入成功\n钱包: <code>{}</code>", wallet.public_key),
            )
            .reply_markup(wallet_detail_keyboard(&wallet))
            .await?;
        }
        PendingAction::VanityPattern {
            side,
            panel_message_id,
        } => {
            let pattern = text.trim();
            let job = state
                .wallet_service
                .create_vanity_job(user_id, &side, pattern)
                .await?;
            render_vanity_job(bot, chat_id, panel_message_id, &job).await?;
        }
        PendingAction::TransferAddress {
            source_wallet_id,
            panel_message_id,
        } => {
            let address = text.trim();
            if !is_probable_wallet_address(address) {
                anyhow::bail!("接收地址格式无效");
            }
            let draft = TransferDraft {
                source_wallet_id,
                destination: TransferDestination::Address(address.to_string()),
                amount: None,
                panel_message_id,
            };
            state
                .transfers
                .lock()
                .await
                .insert(telegram_id, draft.clone());
            render_transfer_amount(bot, chat_id, state, user_id, &draft).await?;
        }
        PendingAction::TransferAmount {
            source_wallet_id,
            panel_message_id,
        } => {
            let amount_sol = text.trim().parse::<f64>().context("请输入有效 SOL 数量")?;
            if !amount_sol.is_finite() || amount_sol <= 0.0 {
                anyhow::bail!("SOL 数量必须大于 0");
            }
            let lamports = (amount_sol * 1_000_000_000.0).round() as u64;
            if lamports == 0 {
                anyhow::bail!("SOL 数量过小");
            }
            let mut transfers = state.transfers.lock().await;
            let draft = transfers
                .get_mut(&telegram_id)
                .context("转账面板已过期，请重新选择目标")?;
            if draft.source_wallet_id != source_wallet_id
                || draft.panel_message_id != panel_message_id
            {
                anyhow::bail!("转账面板已过期");
            }
            draft.amount = Some(TransferAmount::FixedLamports(lamports));
            let draft = draft.clone();
            drop(transfers);
            render_transfer_confirm(bot, chat_id, state, user_id, &draft).await?;
        }
        PendingAction::RenameWallet { wallet_id } => {
            state
                .wallet_service
                .rename_wallet(user_id, wallet_id, text)
                .await?;
            let wallet = state.db.get_wallet(user_id, wallet_id).await?;
            bot.send_message(
                chat_id,
                format!(
                    "钱包标签已更新。Trade API ID 是稳定交易标识，不会随标签改变。\n\n{}",
                    wallet_detail_text(&wallet, wallet_balance_for(state, user_id, &wallet).await)
                ),
            )
            .reply_markup(wallet_detail_keyboard(&wallet))
            .await?;
        }
        PendingAction::AddWatch => {
            let (address, label) = split_first_arg(text, "Watch");
            if !is_probable_wallet_address(address) {
                bot.send_message(chat_id, "钱包地址格式看起来不对，请重新添加。")
                    .reply_markup(back_to_main_keyboard())
                    .await?;
                return Ok(());
            }
            state.db.add_watch_wallet(user_id, address, label).await?;
            state.follow_strategy.refresh_cache().await?;
            bot.send_message(chat_id, "已添加跟踪钱包")
                .reply_markup(back_to_main_keyboard())
                .await?;
        }
        PendingAction::RenameWatch { watch_id } => {
            state
                .db
                .rename_watch_wallet(user_id, watch_id, text)
                .await?;
            bot.send_message(chat_id, "跟踪标签已更新")
                .reply_markup(back_to_main_keyboard())
                .await?;
        }
        PendingAction::StrategyInput { strategy_id, field } => {
            apply_strategy_input(&state.db, user_id, strategy_id, &field, text).await?;
            state.follow_strategy.refresh_cache().await?;
            if let Some(strategy) = state.db.get_strategy(user_id, strategy_id).await? {
                send_strategy_detail(bot, chat_id, &strategy).await?;
            }
        }
        PendingAction::LimitInput { order_id, field } => {
            apply_limit_input(&state.db, user_id, &order_id, &field, text).await?;
            show_limit_order_detail(bot, chat_id, state, user_id, &order_id).await?;
        }
        PendingAction::ManualTrade { side } => {
            handle_trade_text(bot, chat_id, state, telegram_id, side, text).await?;
        }
        PendingAction::ManualTradeAmount { side, mint } => {
            handle_trade_text(
                bot,
                chat_id,
                state,
                telegram_id,
                side,
                &format!("{mint} {}", text.trim()),
            )
            .await?;
        }
        PendingAction::PanelInput {
            field,
            panel_message_id,
        } => {
            let result = apply_trade_panel_input(
                bot,
                chat_id,
                state,
                user_id,
                telegram_id,
                panel_message_id,
                field,
                text,
            )
            .await;
            match result {
                Ok(()) => {
                    let _ = bot.delete_message(chat_id, user_message_id).await;
                }
                Err(err) => {
                    state.pending.lock().await.insert(
                        telegram_id,
                        PendingAction::PanelInput {
                            field,
                            panel_message_id,
                        },
                    );
                    bot.send_message(
                        chat_id,
                        format!("输入无效：{}\n请重新发送。", escape_html(&err.to_string())),
                    )
                    .await?;
                }
            }
        }
        PendingAction::OpenLimitPanelMint { panel_message_id } => {
            if !is_probable_wallet_address(text) {
                state.pending.lock().await.insert(
                    telegram_id,
                    PendingAction::OpenLimitPanelMint { panel_message_id },
                );
                bot.send_message(chat_id, "Mint 地址格式不正确，请重新发送。")
                    .await?;
                return Ok(());
            }
            open_trade_panel_at_message(
                bot,
                chat_id,
                panel_message_id,
                state,
                user_id,
                telegram_id,
                text,
                PanelTab::Limit,
            )
            .await?;
            let _ = bot.delete_message(chat_id, user_message_id).await;
        }
        PendingAction::SettingsInput {
            field,
            settings_message_id,
        } => {
            let result = apply_settings_input(&state.db, user_id, field, text).await;
            match result {
                Ok(page) => {
                    state
                        .db
                        .audit(
                            Some(user_id),
                            "trade_settings.value_updated",
                            Some("user_trade_settings"),
                            Some(&user_id.to_string()),
                            Some(&serde_json::json!({ "field": format!("{field:?}") }).to_string()),
                        )
                        .await?;
                    render_settings_page(bot, chat_id, settings_message_id, state, user_id, page)
                        .await?;
                    let _ = bot.delete_message(chat_id, user_message_id).await;
                }
                Err(err) => {
                    state.pending.lock().await.insert(
                        telegram_id,
                        PendingAction::SettingsInput {
                            field,
                            settings_message_id,
                        },
                    );
                    bot.send_message(
                        chat_id,
                        format!("输入无效：{}\n请重新发送。", escape_html(&err.to_string())),
                    )
                    .await?;
                }
            }
        }
        PendingAction::CreateLimitOrder { side, direction } => {
            create_limit_order_from_text(bot, chat_id, state, user_id, side, direction, text)
                .await?;
        }
        PendingAction::ConfirmTrade { draft } => {
            state
                .pending
                .lock()
                .await
                .insert(telegram_id, PendingAction::ConfirmTrade { draft });
            bot.send_message(chat_id, "请点击确认按钮执行，或返回菜单取消。")
                .reply_markup(trade_confirm_keyboard())
                .await?;
        }
    }
    Ok(())
}

async fn apply_settings_input(
    db: &Database,
    user_id: i64,
    field: SettingsInputField,
    text: &str,
) -> Result<SettingsPage> {
    let value = text.trim().parse::<f64>()?;
    if !value.is_finite() {
        anyhow::bail!("请输入有效数字");
    }
    match field {
        SettingsInputField::Slippage => {
            if !(0.0..=100.0).contains(&value) {
                anyhow::bail!("滑点必须在 0% 到 100% 之间");
            }
            db.update_user_trade_setting(
                user_id,
                "default_slippage_bps",
                &(value * 100.0).round().to_string(),
            )
            .await?;
            Ok(SettingsPage::Execution)
        }
        SettingsInputField::Tip => {
            if value < 0.0 {
                anyhow::bail!("小费必须大于或等于 0");
            }
            db.update_user_trade_setting(user_id, "default_tip_sol", &value.to_string())
                .await?;
            Ok(SettingsPage::Execution)
        }
        SettingsInputField::BuyPreset(index) => {
            if value <= 0.0 {
                anyhow::bail!("买入金额必须大于 0");
            }
            db.update_user_trade_preset(user_id, "buy", index + 1, value)
                .await?;
            Ok(SettingsPage::Buy)
        }
        SettingsInputField::SellPreset(index) => {
            if !(1.0..=100.0).contains(&value) || value.fract() != 0.0 {
                anyhow::bail!("卖出比例必须是 1 到 100 的整数");
            }
            db.update_user_trade_preset(user_id, "sell", index + 1, value)
                .await?;
            Ok(SettingsPage::Sell)
        }
    }
}

async fn handle_trade_text(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    telegram_id: i64,
    side: TradeSide,
    args: &str,
) -> Result<()> {
    let mut parts = args.split_whitespace();
    let Some(mint) = parts.next() else {
        bot.send_message(chat_id, "格式错误，请发送 mint 和数量。")
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    };
    let Some(amount_text) = parts.next() else {
        if is_probable_wallet_address(mint) {
            let user = state.db.get_user_by_telegram_id(telegram_id).await?;
            send_new_trade_panel(
                bot,
                chat_id,
                state,
                user.id,
                telegram_id,
                mint,
                PanelTab::Quick,
            )
            .await?;
            return Ok(());
        }
        bot.send_message(chat_id, "缺少数量。买入填 SOL 数量，卖出填百分比。")
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    };
    let amount = amount_text.parse::<f64>()?;
    let invalid_sell = matches!(side, TradeSide::Sell)
        && (amount > 100.0 || (amount - amount.round()).abs() > f64::EPSILON);
    if !is_probable_wallet_address(mint) || amount <= 0.0 || invalid_sell {
        bot.send_message(chat_id, "mint 或数量格式不正确。")
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    }

    let size = match side {
        TradeSide::Buy => OrderSize::FixedBuySol { amount_sol: amount },
        TradeSide::Sell => OrderSize::SellPercent {
            percent: amount.round() as u64,
        },
    };
    size.validate()?;
    let app_user = state.db.get_user_by_telegram_id(telegram_id).await?;
    let settings = load_user_trade_settings(state, app_user.id).await?;
    let execution = ExecutionParams {
        wallet_id: None,
        mode: ExecutionMode::from_db(&settings.default_execution_mode, true),
        method: ExecutionMethod::from_db(&settings.default_execution_method),
        slippage_bps: settings.default_slippage_bps,
        tip_sol: settings.default_tip_sol,
        mev_mode: if settings.use_mev {
            settings.default_mev_provider.clone()
        } else {
            "none".to_string()
        },
    };
    execution.validate()?;
    let resolving = bot
        .send_message(chat_id, "正在解析 Mint 和 Pool...")
        .await?;
    let route = match resolve_pool_route(state, mint, false).await {
        Ok(route) => route,
        Err(err) => {
            bot.edit_message_text(chat_id, resolving.id, pool_resolution_error_text(&err))
                .reply_markup(trade_menu_keyboard())
                .await?;
            return Ok(());
        }
    };
    if matches!(side, TradeSide::Buy) && custom_quote_route(&route) {
        return open_trade_panel_at_message(
            bot,
            chat_id,
            resolving.id,
            state,
            app_user.id,
            telegram_id,
            mint,
            PanelTab::Quick,
        )
        .await;
    }
    let draft = TradeDraft {
        side: side.clone(),
        mint: mint.to_string(),
        amount,
        execution,
        route,
        quote_amount: None,
        sol_hop_pool_id: None,
    };
    state.pending.lock().await.insert(
        telegram_id,
        PendingAction::ConfirmTrade {
            draft: draft.clone(),
        },
    );

    let side_label = match side {
        TradeSide::Buy => "买入",
        TradeSide::Sell => "卖出",
    };
    let unit = match side {
        TradeSide::Buy => "SOL",
        TradeSide::Sell => "%",
    };
    bot.edit_message_text(
        chat_id,
        resolving.id,
        format!(
            "确认交易\n动作: {}\nMint: <code>{}</code>\nPool: {}{}\n数量: {} {}\n模式: {}\n滑点: {}%\n小费: {} SOL\nMEV: {}",
            side_label,
            mint,
            pool_display_name(&draft.route.pool),
            pool_id_suffix(draft.route.pool_id.as_deref()),
            amount,
            unit,
            execution_mode_label(draft.execution.mode),
            draft.execution.slippage_bps as f64 / 100.0,
            draft.execution.tip_sol,
            draft.execution.mev_mode,
        ),
    )
    .reply_markup(trade_confirm_keyboard())
    .await?;
    Ok(())
}

async fn create_limit_order_from_text(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    side: TradeSide,
    direction: TriggerDirection,
    args: &str,
) -> Result<()> {
    let mut parts = args.split_whitespace();
    let Some(mint) = parts.next() else {
        bot.send_message(chat_id, "格式错误，请发送 mint、触发价格和数量。")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    let Some(price_text) = parts.next() else {
        bot.send_message(chat_id, "缺少触发价格。")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    let Some(amount_text) = parts.next() else {
        bot.send_message(chat_id, "缺少数量。买入填 SOL，卖出填百分比。")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    };
    let trigger_price = price_text.parse::<f64>()?;
    let amount = amount_text.parse::<f64>()?;
    if !is_probable_wallet_address(mint) || trigger_price <= 0.0 || amount <= 0.0 {
        bot.send_message(chat_id, "mint、触发价格或数量格式不正确。")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    }
    if matches!(side, TradeSide::Sell)
        && (amount > 100.0 || (amount - amount.round()).abs() > f64::EPSILON)
    {
        bot.send_message(chat_id, "卖出百分比必须是 1 到 100 的整数。")
            .reply_markup(back_to_main_keyboard())
            .await?;
        return Ok(());
    }

    if matches!(side, TradeSide::Buy) {
        let route = resolve_pool_route(state, mint, false).await?;
        if custom_quote_route(&route) {
            bot.send_message(
                chat_id,
                "该池以自定义 Quote 计价，请在快速交易面板设置 Quote 金额。",
            )
            .reply_markup(back_to_main_keyboard())
            .await?;
            return Ok(());
        }
    }

    let wallet = state.db.active_wallet(user_id).await?;
    let settings = load_user_trade_settings(state, user_id).await?;
    let action = direction.action(&side);
    let execution = ExecutionParams {
        wallet_id: wallet.as_ref().map(|item| item.id),
        mode: ExecutionMode::from_db(&settings.default_execution_mode, true),
        method: ExecutionMethod::from_db(&settings.default_execution_method),
        slippage_bps: settings.default_slippage_bps,
        tip_sol: settings.default_tip_sol,
        mev_mode: if settings.use_mev {
            settings.default_mev_provider.clone()
        } else {
            "none".to_string()
        },
    };
    execution.validate()?;
    let order_id = state
        .db
        .insert_limit_order(
            user_id,
            execution.wallet_id,
            mint,
            action,
            "price_usd",
            trigger_price,
            matches!(side, TradeSide::Buy).then_some(amount),
            matches!(side, TradeSide::Sell).then_some(amount),
            &execution,
            None,
        )
        .await?;

    bot.send_message(
        chat_id,
        format!(
            "已创建限价单\n订单: <code>{}</code>\n动作: {}\n条件: {} {} USD\n数量: {}\n滑点: {}%\n小费: {} SOL\nMEV: {}\nMint: <code>{}</code>\n\n触发器会优先使用 PumpAPI USD 价格。",
            order_id,
            action.label(),
            direction.label(),
            trigger_price,
            if matches!(side, TradeSide::Buy) {
                format!("{amount} SOL")
            } else {
                format!("{amount}%")
            },
            execution.slippage_bps as f64 / 100.0,
            execution.tip_sol,
            execution.mev_mode,
            mint
        ),
    )
    .reply_markup(back_to_main_keyboard())
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn submit_quick_trade(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    draft: TradeDraft,
) -> Result<()> {
    let acquired = state.trades_in_flight.lock().await.insert(telegram_id);
    if !acquired {
        bot.edit_message_text(
            chat_id,
            message_id,
            "已有交易正在处理中，请等待结果；本次点击未重复提交。",
        )
        .await?;
        return Ok(());
    }
    let result =
        execute_trade_draft(bot, chat_id, message_id, state, user_id, telegram_id, draft).await;
    state.trades_in_flight.lock().await.remove(&telegram_id);
    if let Err(error_value) = result {
        bot.edit_message_text(
            chat_id,
            message_id,
            format!("交易未提交：{}", escape_html(&error_value.to_string())),
        )
        .reply_markup(trade_menu_keyboard())
        .await?;
    }
    Ok(())
}

async fn execute_pending_trade(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
) -> Result<()> {
    let acquired = state.trades_in_flight.lock().await.insert(telegram_id);
    if !acquired {
        return Ok(());
    }
    let Some(PendingAction::ConfirmTrade { draft }) =
        state.pending.lock().await.remove(&telegram_id)
    else {
        state.trades_in_flight.lock().await.remove(&telegram_id);
        bot.edit_message_text(chat_id, message_id, "没有待确认交易")
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    };

    let result =
        execute_trade_draft(bot, chat_id, message_id, state, user_id, telegram_id, draft).await;
    state.trades_in_flight.lock().await.remove(&telegram_id);
    result
}

#[allow(clippy::too_many_arguments)]
async fn execute_trade_draft(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    draft: TradeDraft,
) -> Result<()> {
    validate_trade_draft(&draft)?;
    let isolated_simulation = state.config.isolated_sdk_simulation_enabled
        && state.trade_client.isolated_sdk_simulation_available()
        && draft.execution.mode == ExecutionMode::Simulate
        && draft.execution.method == ExecutionMethod::Sdk;
    let submission_check = state.trade_client.ensure_submission_enabled();
    if !isolated_simulation && submission_check.is_err() {
        let err = submission_check.unwrap_err();
        let mut message = escape_html(&err.to_string());
        match read_only_trade_draft_decision(state, user_id, &draft).await {
            Ok(Some(decision)) => {
                let label = match decision.status.as_str() {
                    "sdk_verified" => "SDK 路线已核验",
                    "sdk_unsupported" => "SDK 不支持候选",
                    "unresolved_retryable" => "路线待核验",
                    _ => "策略阻止",
                };
                message.push_str(&format!(
                    "\n服务端只读判断：{}（{}）\n不会创建订单或提交交易。",
                    label,
                    escape_html(decision.reason.as_deref().unwrap_or("无补充原因"))
                ));
                if let Some(hint) = decision.read_only_route_hint() {
                    message.push_str(&format!("\n{}", escape_html(hint)));
                }
            }
            Ok(None) => {}
            Err(_) => message.push_str("\n服务端只读路由判断暂不可用，请稍后再查。"),
        }
        bot.edit_message_text(chat_id, message_id, message)
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    }
    bot.edit_message_text(
        chat_id,
        message_id,
        format!(
            "{}\nMint: <code>{}</code>\nPool: {}{}",
            if isolated_simulation {
                "正在模拟 SDK 交易..."
            } else {
                "正在提交交易..."
            },
            draft.mint,
            pool_display_name(&draft.route.pool),
            pool_id_suffix(draft.route.pool_id.as_deref())
        ),
    )
    .await?;

    let wallet = match draft.execution.wallet_id {
        Some(wallet_id) => Some(state.db.get_wallet(user_id, wallet_id).await?),
        None => state.db.active_wallet(user_id).await?,
    };
    let wallet_id = wallet.as_ref().map(|item| item.id);
    let mut execution = draft.execution.clone();
    execution.wallet_id = wallet_id;
    if !load_user_trade_settings(state, user_id).await?.use_mev {
        execution.mev_mode = "none".to_string();
    }
    execution.validate()?;
    let side = match draft.side {
        TradeSide::Buy => "buy",
        TradeSide::Sell => "sell",
    };
    let idempotency_key = format!("manual-{}-{}", side, Uuid::new_v4());

    if execution.mode.is_dry_run() {
        let dry_run_request = custom_quote_route(&draft.route).then(|| {
            serde_json::json!({
                "quote_mint": draft.route.quote_mint,
                "amount_quote": draft.quote_amount,
                "pool": draft.route.pool,
                "pool_id": draft.route.pool_id,
                "settlement_asset": if draft.sol_hop_pool_id.is_some() { "sol" } else { "quote" },
                "sol_hop_pool": draft.sol_hop_pool_id.as_ref().map(|_| "raydium-cpmm"),
                "sol_hop_pool_id": draft.sol_hop_pool_id,
                "amount_sol": (matches!(draft.side, TradeSide::Buy) && draft.sol_hop_pool_id.is_some()).then_some(draft.amount),
            })
            .to_string()
        });
        let order_id = state
            .db
            .insert_order(
                user_id,
                wallet_id,
                None,
                None,
                &draft.mint,
                side,
                (matches!(draft.side, TradeSide::Buy)
                    && (!custom_quote_route(&draft.route) || draft.sol_hop_pool_id.is_some()))
                .then_some(draft.amount),
                matches!(draft.side, TradeSide::Sell).then_some(draft.amount),
                "dry_run",
                &idempotency_key,
                dry_run_request.as_deref(),
                None,
                None,
                None,
            )
            .await?;
        show_opposite_trade_panel(
            bot,
            chat_id,
            message_id,
            state,
            user_id,
            telegram_id,
            &draft,
            format!(
                "Dry-run {}已记录\n订单: <code>{}</code>\n未调用 trade_service。",
                draft.side.label(),
                order_id
            ),
        )
        .await?;
        return Ok(());
    }

    let Some(copy_wallet) = wallet.as_ref() else {
        record_manual_blocked(
            &state.db,
            user_id,
            None,
            &draft,
            side,
            &idempotency_key,
            "没有选择执行钱包",
        )
        .await?;
        bot.edit_message_text(chat_id, message_id, "已拦截：没有选择执行钱包。")
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    };
    let ready_result = if isolated_simulation {
        state.trade_client.isolated_sdk_ready().await
    } else {
        state.trade_client.ready().await
    };
    let ready = match ready_result {
        Ok(ready) if ready.ready => ready,
        Ok(_) => {
            record_manual_blocked(
                &state.db,
                user_id,
                wallet_id,
                &draft,
                side,
                &idempotency_key,
                "trade_service is not ready",
            )
            .await?;
            bot.edit_message_text(chat_id, message_id, "已拦截：trade_service 尚未 ready。")
                .reply_markup(trade_menu_keyboard())
                .await?;
            return Ok(());
        }
        Err(err) => {
            let reason = format!("trade_service readiness failed: {err}");
            record_manual_blocked(
                &state.db,
                user_id,
                wallet_id,
                &draft,
                side,
                &idempotency_key,
                &reason,
            )
            .await?;
            bot.edit_message_text(
                chat_id,
                message_id,
                format!("已拦截：<code>{}</code>", escape_html(&reason)),
            )
            .reply_markup(trade_menu_keyboard())
            .await?;
            return Ok(());
        }
    };
    let Some(trade_service_wallet_id) = ready
        .wallets
        .iter()
        .find(|candidate| candidate.pubkey == copy_wallet.public_key)
        .map(|candidate| candidate.wallet_id.clone())
    else {
        let reason = "所选钱包未绑定到 trade_service 同公钥钱包";
        record_manual_blocked(
            &state.db,
            user_id,
            wallet_id,
            &draft,
            side,
            &idempotency_key,
            reason,
        )
        .await?;
        bot.edit_message_text(chat_id, message_id, format!("已拦截：{reason}。"))
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    };

    if isolated_simulation {
        anyhow::ensure!(
            isolated_sdk_simulation_supported(&draft),
            "隔离 SDK 模拟当前仅支持已核验单池或 SOL 两跳路线"
        );
    }

    if custom_quote_route(&draft.route) && execution.method != ExecutionMethod::Sdk {
        anyhow::bail!("自定义 Quote 交易请使用 SDK 执行方式");
    }
    if draft.sol_hop_pool_id.is_some()
        && !(isolated_simulation && isolated_sdk_bridge_sell_supported(&draft))
    {
        let current_route = resolve_pool_route(state, &draft.mint, true).await?;
        anyhow::ensure!(
            current_route == draft.route,
            "池或 Quote 已变化，请刷新交易面板"
        );
        let hop = bounded_panel_read(
            state.trade_client.resolve_sol_hop(&draft.route.quote_mint),
            std::time::Duration::from_secs(12),
            "SOL 两跳查询超时",
        )
        .await?;
        validate_sol_hop_refresh(&draft, &current_route, &hop)?;
    }
    let quote_units = if matches!(draft.side, TradeSide::Buy)
        && custom_quote_route(&draft.route)
        && draft.sol_hop_pool_id.is_none()
    {
        let current_route = resolve_pool_route(state, &draft.mint, true).await?;
        if current_route != draft.route {
            anyhow::bail!("池或 Quote 已变化，请刷新交易面板");
        }
        let quote = state
            .trade_client
            .quote_balance(user_id, &trade_service_wallet_id, &draft.route.quote_mint)
            .await?;
        if quote.quote_mint != draft.route.quote_mint {
            anyhow::bail!("Quote Mint 不匹配");
        }
        let units = quote_amount_units(
            draft
                .quote_amount
                .as_deref()
                .context("请输入 Quote 买入金额")?,
            quote.decimals,
        )?;
        if execution.mode == ExecutionMode::Live && quote.balance_units < units {
            anyhow::bail!(
                "Quote 余额不足：可用 {}，需要 {}",
                format_quote_units(quote.balance_units, quote.decimals),
                draft.quote_amount.as_deref().unwrap_or("-")
            );
        }
        if execution.mode == ExecutionMode::Live
            && quote.balance_lamports
                < 3_000_000 + (execution.tip_sol * 1_000_000_000.0).ceil() as u64
        {
            anyhow::bail!("SOL 余额不足以支付交易费用及可能的代币账户租金");
        }
        Some(units)
    } else {
        None
    };
    let request = manual_trade_request(
        &draft,
        &execution,
        trade_service_wallet_id,
        quote_units,
        &idempotency_key,
    )?;
    let request_json = serde_json::to_string(&request)?;

    let trade_result = if isolated_simulation {
        state
            .trade_client
            .submit_isolated_sdk_simulation(&request, &copy_wallet.public_key)
            .await
    } else {
        state
            .trade_client
            .submit_trade(&request)
            .await
            .map(|response| (response, request.clone()))
    };
    match trade_result {
        Ok((response, submitted_request)) => {
            let submitted_request_json = serde_json::to_string(&submitted_request)?;
            let response_json = serde_json::to_string(&response)?;
            let status = crate::executor::normalize_trade_status(&response);
            let order_id = state
                .db
                .insert_order_with_origin(
                    user_id,
                    wallet_id,
                    None,
                    None,
                    &draft.mint,
                    side,
                    request.amount_sol,
                    request.sell_percent.map(|value| value as f64),
                    status,
                    &idempotency_key,
                    Some(&submitted_request_json),
                    Some(&response_json),
                    response.signature.as_deref(),
                    response.error.as_deref(),
                    if isolated_simulation {
                        "isolated_sdk_simulation"
                    } else {
                        "primary"
                    },
                    response.request_id.as_deref(),
                )
                .await?;
            state
                .db
                .update_order_from_trade_service(
                    &order_id,
                    status,
                    response.request_id.as_deref(),
                    Some(&response_json),
                    response.signature.as_deref(),
                    response.error.as_deref(),
                )
                .await?;
            state
                .db
                .insert_order_attempt(
                    &order_id,
                    status,
                    Some(&submitted_request_json),
                    Some(&response_json),
                    response.error.as_deref(),
                )
                .await?;
            let result_text = manual_trade_result_text(
                draft.side.label(),
                request.wallet_id.as_deref(),
                status,
                &order_id,
                &response,
            );
            if isolated_simulation || status == "failed" || response.error.is_some() {
                bot.edit_message_text(chat_id, message_id, result_text)
                    .reply_markup(trade_menu_keyboard())
                    .await?;
            } else {
                show_opposite_trade_panel(
                    bot,
                    chat_id,
                    message_id,
                    state,
                    user_id,
                    telegram_id,
                    &draft,
                    result_text,
                )
                .await?;
            }
        }
        Err(err) => {
            let error = err.to_string();
            let order_id = state
                .db
                .insert_order_with_origin(
                    user_id,
                    wallet_id,
                    None,
                    None,
                    &draft.mint,
                    side,
                    request.amount_sol,
                    request.sell_percent.map(|value| value as f64),
                    "failed",
                    &idempotency_key,
                    Some(&request_json),
                    None,
                    None,
                    Some(&error),
                    if isolated_simulation {
                        "isolated_sdk_simulation"
                    } else {
                        "primary"
                    },
                    None,
                )
                .await?;
            state
                .db
                .insert_order_attempt(&order_id, "failed", Some(&request_json), None, Some(&error))
                .await?;
            bot.edit_message_text(
                chat_id,
                message_id,
                format!("下单失败\n<code>{}</code>", escape_html(&error)),
            )
            .reply_markup(trade_menu_keyboard())
            .await?;
        }
    }

    Ok(())
}

async fn read_only_trade_draft_decision(
    state: &BotState,
    user_id: i64,
    draft: &TradeDraft,
) -> Result<Option<crate::trade_client::RouteDecisionResponse>> {
    if draft.execution.mode.is_dry_run()
        || (custom_quote_route(&draft.route) && draft.sol_hop_pool_id.is_none())
    {
        return Ok(None);
    }
    let wallet = match draft.execution.wallet_id {
        Some(wallet_id) => Some(state.db.get_wallet(user_id, wallet_id).await?),
        None => state.db.active_wallet(user_id).await?,
    };
    let Some(wallet) = wallet else {
        return Ok(None);
    };
    let Some(remote_wallet_id) = wallet.external_wallet_id else {
        return Ok(None);
    };
    let mut execution = draft.execution.clone();
    execution.mode = ExecutionMode::Simulate;
    execution.validate()?;
    let request = manual_trade_request(
        draft,
        &execution,
        remote_wallet_id,
        None,
        &format!("read-only-route-{}", Uuid::new_v4()),
    )?;
    let decision = bounded_panel_read(
        state
            .trade_client
            .route_decision(&request, Some(&wallet.public_key)),
        std::time::Duration::from_secs(12),
        "read-only SDK route decision timeout",
    )
    .await?;
    Ok(Some(decision))
}

#[allow(clippy::too_many_arguments)]
fn manual_trade_result_text(
    side_label: &str,
    wallet_id: Option<&str>,
    status: &str,
    order_id: &str,
    response: &crate::trade_client::TradeResponse,
) -> String {
    let failed = status == "failed" || response.error.is_some();
    let heading = if failed { "失败" } else { "交易状态" };
    let mut text = format!(
        "上笔{}{heading}\n钱包 ID: <code>{}</code>\n状态: {}\n订单: <code>{}</code>\n远端请求: <code>{}</code>\n签名: <code>{}</code>",
        side_label,
        escape_html(wallet_id.unwrap_or("-")),
        escape_html(status),
        escape_html(order_id),
        escape_html(response.request_id.as_deref().unwrap_or("-")),
        escape_html(response.signature.as_deref().unwrap_or("-")),
    );
    if failed {
        let error = response
            .error
            .as_deref()
            .unwrap_or("交易服务未返回具体失败原因");
        text.push_str(&format!("\n错误: <code>{}</code>", escape_html(error)));
        if error.contains("sell fixed output amount must be greater than zero") {
            text.push_str("\n交易端卖出构建得到零输出金额，需要核对代币余额及池报价/SDK 参数；这条错误本身不能证明钱包余额为零。");
        }
    }
    text
}

async fn show_opposite_trade_panel(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    draft: &TradeDraft,
    notice: String,
) -> Result<()> {
    let completed_side = match draft.side {
        TradeSide::Buy => PanelSide::Buy,
        TradeSide::Sell => PanelSide::Sell,
    };
    let panel_updated = {
        let mut panels = state.panels.lock().await;
        match panels.get_mut(&telegram_id) {
            Some(session)
                if session.message_id == Some(message_id) && session.mint == draft.mint =>
            {
                if draft.sol_hop_pool_id.is_some()
                    && draft.execution.mode == ExecutionMode::Simulate
                {
                    session.notice = Some(notice.clone());
                } else {
                    session.show_opposite_quick_side(completed_side, notice.clone());
                }
                true
            }
            _ => false,
        }
    };
    if !panel_updated {
        bot.edit_message_text(chat_id, message_id, notice)
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    }
    render_trade_panel(bot, chat_id, message_id, state, user_id, telegram_id).await
}

async fn record_manual_blocked(
    db: &Database,
    user_id: i64,
    wallet_id: Option<i64>,
    draft: &TradeDraft,
    side: &str,
    idempotency_key: &str,
    reason: &str,
) -> Result<()> {
    db.insert_order(
        user_id,
        wallet_id,
        None,
        None,
        &draft.mint,
        side,
        matches!(draft.side, TradeSide::Buy).then_some(draft.amount),
        matches!(draft.side, TradeSide::Sell).then_some(draft.amount),
        "blocked",
        idempotency_key,
        None,
        None,
        None,
        Some(reason),
    )
    .await?;
    Ok(())
}

fn is_probable_wallet_address(text: &str) -> bool {
    text.len() >= 32 && text.len() <= 44 && bs58::decode(text).into_vec().is_ok()
}

fn split_first_arg<'a>(input: &'a str, default_label: &'a str) -> (&'a str, &'a str) {
    let mut parts = input.trim().splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("").trim();
    let second = parts.next().unwrap_or(default_label).trim();
    (
        first,
        if second.is_empty() {
            default_label
        } else {
            second
        },
    )
}

fn short_text(value: &str) -> String {
    if value.len() > 10 {
        format!("{}..{}", &value[..4], &value[value.len() - 4..])
    } else {
        value.to_string()
    }
}

fn pool_display_name(pool: &str) -> &str {
    match pool {
        "pump" => "PumpFun",
        "pump-amm" => "PumpSwap",
        "bonk" => "Bonk",
        "meteora-damm-v2" => "Meteora",
        "meteora-dbc" => "Meteora DBC",
        "raydium-amm-v4" => "Raydium AMM V4",
        "raydium-cpmm" => "Raydium CPMM",
        _ => pool,
    }
}

async fn resolve_pool_route(
    state: &BotState,
    mint: &str,
    force_refresh: bool,
) -> Result<ResolvedPoolRoute> {
    if state.config.isolated_sdk_simulation_enabled
        && mint == crate::trade_client::WSOL_MINT
        && state.trade_client.isolated_sdk_simulation_available()
    {
        return bounded_panel_read(
            state.trade_client.isolated_wsol_usdc_sell_pool(),
            std::time::Duration::from_secs(state.config.pool_resolve_timeout_secs),
            "isolated WSOL/USDC pool resolution timeout",
        )
        .await;
    }
    if state.config.isolated_sdk_simulation_enabled
        && state.trade_client.isolated_sdk_simulation_available()
    {
        if let Ok(Some(hint)) = state.pool_routes.observed_hint(mint).await {
            if let Ok(route) = bounded_panel_read(
                state.trade_client.isolated_sdk_verify_pool_hint(&hint),
                std::time::Duration::from_secs(state.config.pool_resolve_timeout_secs),
                "isolated SDK cached pool verification timeout",
            )
            .await
            {
                return Ok(route);
            }
        }
        let isolated_result = bounded_panel_read(
            state.trade_client.isolated_sdk_resolve_pool(mint),
            std::time::Duration::from_secs(state.config.pool_resolve_timeout_secs),
            "isolated SDK pool resolution timeout",
        )
        .await;
        if isolated_result.is_ok() {
            return isolated_result;
        }
        if let Ok(hint) = state.pool_routes.resolve(mint, true).await {
            return bounded_panel_read(
                state.trade_client.isolated_sdk_verify_pool_hint(&hint),
                std::time::Duration::from_secs(state.config.pool_resolve_timeout_secs),
                "isolated SDK pool verification timeout",
            )
            .await;
        }
        return isolated_result;
    }
    bounded_panel_read(
        state.pool_routes.resolve(mint, force_refresh),
        std::time::Duration::from_secs(state.config.pool_resolve_timeout_secs),
        "pool resolution timeout",
    )
    .await
}

fn pool_resolution_error_text(error: &anyhow::Error) -> &'static str {
    let lower = format!("{error:#}").to_ascii_lowercase();
    if lower.contains("pump 池") && lower.contains("报价币不受支持") {
        "该 Pump 池的报价币不受支持\n当前交易接口仅支持 WSOL 或 USDC 报价，尚未接入这个自定义 Quote 的买卖路径。尚未提交交易。"
    } else if lower.contains("raydium cpmm") && lower.contains("报价币不受支持") {
        "该 Raydium CPMM 池的报价币不受支持\n当前交易服务仅支持 WSOL 或 USDC 报价池，无法交易这个自定义 Quote 池。尚未提交交易。"
    } else if lower.contains("meteora damm v2") && lower.contains("报价币不受支持") {
        "该 Meteora 池的报价币不受支持\n当前仅支持 WSOL 或 USDC 报价池，请刷新或重新发送 Mint；不会自动替换报价币下单。"
    } else if lower.contains("timeout") || lower.contains("timed out") || lower.contains("connect")
    {
        "交易服务暂不可用\nMint 和 Pool 解析请求超时或连接失败，请稍后重新发送 Mint。"
    } else if lower.contains("rate limited")
        || lower.contains("rpc unavailable")
        || lower.contains("429")
    {
        "Pool 解析暂时不可用\nRPC 节点限流或响应异常，请稍后重新发送 Mint。"
    } else if lower.contains("pool not found") || lower.contains("no supported pool") {
        "未找到支持的 Pool\n该 Mint 可能尚未建池、已迁移到暂不支持的 DEX，或地址有误。"
    } else {
        "Pool 解析失败\n请稍后重新发送 Mint 重试。"
    }
}

fn pool_id_suffix(pool_id: Option<&str>) -> String {
    pool_id
        .map(|value| format!(" <code>{}</code>", short_text(value)))
        .unwrap_or_default()
}

async fn send_strategy_detail(
    bot: &HtmlBot,
    chat_id: ChatId,
    strategy: &StrategyConfig,
) -> Result<()> {
    bot.send_message(chat_id, format_strategy_detail(strategy))
        .reply_markup(strategy_detail_keyboard(strategy))
        .await?;
    Ok(())
}

async fn edit_strategy_detail(
    bot: &HtmlBot,
    message: &MaybeInaccessibleMessage,
    strategy: &StrategyConfig,
) -> Result<()> {
    bot.edit_message_text(
        message.chat().id,
        message.id(),
        format_strategy_detail(strategy),
    )
    .reply_markup(strategy_detail_keyboard(strategy))
    .await?;
    Ok(())
}

fn format_strategy_detail(strategy: &StrategyConfig) -> String {
    let buy_mode = if strategy.buy_mode == "percentage" {
        format!("比例买入 {}%", strategy.buy_ratio_percent)
    } else {
        format!("固定 {} SOL", strategy.buy_amount_sol)
    };
    let buy_amount_line = if strategy.buy_mode == "percentage" {
        format!("买入金额: {}%", strategy.buy_ratio_percent)
    } else {
        format!("买入金额: {} SOL", strategy.buy_amount_sol)
    };
    let pools = if strategy.allowed_pools.is_empty() {
        "全部".to_string()
    } else {
        strategy.allowed_pools.join(",")
    };
    format!(
        "跟单策略 #{}\n\n状态: {}\n模式: {}\n跟买: {}\n买入方式: {}\n跟卖: {}，卖出比例: {}%\n{}\nMint市值区间: {} - {}\n钱包买入金额区间: {} - {} SOL\n滑点: {}%\n小费: {} SOL\nMEV: {}\n执行方式: {}\n最大延迟: {}\nPool筛选: {}\n\n选择操作:",
        strategy.id,
        if strategy.enabled { "启用" } else { "暂停" },
        if strategy.dry_run { "Dry-run" } else { "Live" },
        if strategy.copy_buy { "开" } else { "关" },
        buy_mode,
        if strategy.copy_sell { "开" } else { "关" },
        strategy.sell_percent.unwrap_or(100.0),
        buy_amount_line,
        range_value(strategy.min_mcap),
        range_value(strategy.max_mcap),
        range_value(strategy.min_target_buy_sol),
        range_value(strategy.max_target_buy_sol),
        strategy.max_slippage_bps as f64 / 100.0,
        strategy.tip_sol,
        strategy.mev_mode,
        strategy.execution_method,
        strategy
            .max_signal_delay_ms
            .map(|value| format_delay(Some(value)))
            .unwrap_or_else(|| "不限".to_string()),
        pools
    )
}

fn range_value(value: f64) -> String {
    if value > 0.0 {
        value.to_string()
    } else {
        "不限".to_string()
    }
}

fn strategy_input_prompt(field: &str) -> &'static str {
    match field {
        "buy_amount_sol" => "发送固定买入 SOL 数量，例如 0.01",
        "buy_ratio_percent" => "发送比例买入百分比，例如 50",
        "sell_percent" => "发送跟卖比例百分比，例如 100 表示全卖，50 表示卖一半",
        "mcap_range" => "发送 mint 市值区间，格式：最小 最大。0 表示不限，例如：10000 500000",
        "target_buy_range" => {
            "发送目标钱包买入金额区间 SOL，格式：最小 最大。0 表示不限，例如：0.1 3"
        }
        "tip_sol" => "发送小费 SOL，例如 0.0001",
        "max_slippage_bps" => "发送滑点百分比，例如 15 表示 15%（1500 bps）",
        "max_signal_delay_ms" => "发送最大延迟毫秒，0 表示不限，例如 1000",
        "allowed_pools" => {
            "发送允许 Pool，逗号分隔；发送 all 表示全部。例如 pump,pump-amm,meteora-dbc"
        }
        _ => "发送新的数值",
    }
}

fn limit_input_prompt(field: &str) -> &'static str {
    match field {
        "slippage_bps" => "发送限价单滑点百分比，例如 15 表示 15%（1500 bps）",
        "tip_sol" => "发送限价单小费 SOL，例如 0.0001",
        _ => "发送新的执行参数",
    }
}

async fn apply_limit_input(
    db: &Database,
    user_id: i64,
    order_id: &str,
    field: &str,
    text: &str,
) -> Result<()> {
    let value = text.trim().parse::<f64>()?;
    let stored = match field {
        "slippage_bps" => {
            if !(0.0..=100.0).contains(&value) {
                anyhow::bail!("滑点必须在 0% 到 100% 之间");
            }
            (value * 100.0).round().to_string()
        }
        "tip_sol" => {
            if !value.is_finite() || value < 0.0 {
                anyhow::bail!("小费必须大于或等于 0");
            }
            value.to_string()
        }
        _ => anyhow::bail!("不支持的限价单参数"),
    };
    if !db
        .update_limit_order_execution_field(user_id, order_id, field, &stored)
        .await?
    {
        anyhow::bail!("只有 pending 限价单可以修改参数");
    }
    Ok(())
}

async fn apply_strategy_input(
    db: &Database,
    user_id: i64,
    strategy_id: i64,
    field: &str,
    text: &str,
) -> Result<()> {
    match field {
        "mcap_range" => {
            let (min, max) = parse_pair(text)?;
            db.update_strategy_field(user_id, strategy_id, "min_mcap", &min.to_string())
                .await?;
            db.update_strategy_field(user_id, strategy_id, "max_mcap", &max.to_string())
                .await?;
        }
        "target_buy_range" => {
            let (min, max) = parse_pair(text)?;
            db.update_strategy_field(user_id, strategy_id, "min_target_buy_sol", &min.to_string())
                .await?;
            db.update_strategy_field(user_id, strategy_id, "max_target_buy_sol", &max.to_string())
                .await?;
        }
        "allowed_pools" => {
            let value = if text.trim().eq_ignore_ascii_case("all") {
                ""
            } else {
                text.trim()
            };
            db.update_strategy_field(user_id, strategy_id, "allowed_pools", value)
                .await?;
        }
        "max_signal_delay_ms" => {
            let value = text.trim().parse::<i64>()?;
            db.update_strategy_field(
                user_id,
                strategy_id,
                "max_signal_delay_ms",
                if value <= 0 { "0" } else { text.trim() },
            )
            .await?;
        }
        "sell_percent" => {
            let value = text.trim().parse::<f64>()?;
            if !(1.0..=100.0).contains(&value) || (value - value.round()).abs() > f64::EPSILON {
                anyhow::bail!("卖出比例必须是 1 到 100 的整数");
            }
            db.update_strategy_field(user_id, strategy_id, field, &value.to_string())
                .await?;
        }
        "max_slippage_bps" => {
            let value = text.trim().parse::<f64>()?;
            if !(0.0..=100.0).contains(&value) {
                anyhow::bail!("滑点必须在 0% 到 100% 之间");
            }
            db.update_strategy_field(
                user_id,
                strategy_id,
                field,
                &(value * 100.0).round().to_string(),
            )
            .await?;
        }
        _ => {
            let value = text.trim().parse::<f64>()?;
            db.update_strategy_field(user_id, strategy_id, field, &value.to_string())
                .await?;
        }
    }
    Ok(())
}

fn parse_pair(text: &str) -> Result<(f64, f64)> {
    let values = text
        .split_whitespace()
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 2 {
        anyhow::bail!("请输入两个数字，例如：0.1 3");
    }
    Ok((values[0], values[1]))
}

fn format_optional_number(value: Option<f64>) -> String {
    value
        .map(|number| {
            if number.abs() >= 1_000_000.0 {
                format!("{number:.2}")
            } else if number.abs() >= 1.0 {
                format!("{number:.4}")
            } else {
                format!("{number:.8}")
            }
        })
        .unwrap_or_else(|| "-".to_string())
}

fn optional_i64(value: Option<i64>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_json_summary(value: &str) -> String {
    let compact = serde_json::from_str::<serde_json::Value>(value)
        .ok()
        .and_then(|json| serde_json::to_string(&json).ok())
        .unwrap_or_else(|| value.to_string());
    if compact.chars().count() > 900 {
        let mut summary = compact.chars().take(900).collect::<String>();
        summary.push_str("...");
        summary
    } else {
        compact
    }
}

fn format_limit_trigger_line(
    triggered_price: Option<f64>,
    triggered_currency: Option<&str>,
    source_event_id: Option<i64>,
    triggered_at: Option<&str>,
) -> String {
    match triggered_price {
        Some(price) => format!(
            "触发: {} {} | 事件: {} | {}",
            format_optional_number(Some(price)),
            triggered_currency.unwrap_or("-"),
            optional_i64(source_event_id),
            triggered_at.unwrap_or("-")
        ),
        None => "触发: -".to_string(),
    }
}

fn format_delay(value_ms: Option<i64>) -> String {
    match value_ms {
        Some(value) if value >= 0 => {
            if value < 1000 {
                format!("{value} ms")
            } else {
                format!("{:.3} s", value as f64 / 1000.0)
            }
        }
        Some(value) => format!("{value} ms"),
        None => "事件未提供时间".to_string(),
    }
}

fn format_preflight_report(report: &PreflightReport) -> String {
    let active_wallet = report
        .active_wallet
        .as_ref()
        .map(|wallet| {
            format!(
                "{} <code>{}</code>",
                escape_html(&wallet.label),
                escape_html(&wallet.public_key)
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let sol_usd = report
        .sol_usd_price
        .map(|value| format!("{} USD", format_optional_number(Some(value))))
        .unwrap_or_else(|| "-".to_string());
    let trade_wallet = report
        .trade_service_wallet
        .as_ref()
        .map(|wallet| {
            format!(
                "{} <code>{}</code>",
                escape_html(&wallet.wallet_id),
                escape_html(&wallet.pubkey)
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let live_ready = if report.is_live_ready() {
        "可准备 Live"
    } else {
        "不可 Live"
    };
    let reasons = report.blocking_reasons();
    let reason_text = if reasons.is_empty() {
        "无阻断项".to_string()
    } else {
        reasons
            .into_iter()
            .map(|reason| format!("- {reason}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "实盘检查\n状态: {live_ready}\n\n默认钱包: {}\n执行钱包绑定: {}\n钱包: {} 个\n跟踪钱包: {} 个，启用 {} 个\n策略: {} 个，Live {} 个\n限价单: pending {} 个，Live {} 个\ntrade_service health: {}\ntrade_service ready: {}\ntrade_service 信息: {}\nreadiness 信息: {}\naggregator Shadow: {}\nPumpAPI 数据流: {}\nPumpAPI 最近接收: {}\nSOL/USD: {}\n默认滑点: {} bps\n默认小费: {} SOL\n\n阻断项\n{}",
        active_wallet,
        trade_wallet,
        report.wallet_count,
        report.watch_wallet_count,
        report.enabled_watch_wallet_count,
        report.strategy_count,
        report.live_strategy_count,
        report.pending_limit_order_count,
        report.live_limit_order_count,
        ok_bad(report.trade_service_ok),
        ok_bad(report.trade_service_ready),
        escape_html(&report.trade_service_message),
        escape_html(&report.trade_service_ready_message),
        escape_html(&report.aggregator_shadow_message),
        ok_bad(report.pumpapi_stream_fresh),
        report.latest_pumpapi_event_at.as_deref().unwrap_or("-"),
        sol_usd,
        report.default_slippage_bps,
        report.default_tip_sol,
        reason_text
    )
}

fn execution_mode_label(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::DryRun => "Dry-run",
        ExecutionMode::Simulate => "Simulate",
        ExecutionMode::Live => "Live",
    }
}

fn ok_bad(value: bool) -> &'static str {
    if value {
        "OK"
    } else {
        "异常"
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn wallet_detail_text(
    wallet: &crate::db::models::WalletRecord,
    balance_lamports: Option<u64>,
) -> String {
    let external_wallet_id = wallet
        .external_wallet_id
        .as_deref()
        .map(escape_html)
        .unwrap_or_else(|| "未绑定（旧本地钱包）".to_string());
    format!(
        "钱包详情\n本地记录 ID: {}\nTrade API ID: <code>{}</code>\n标签: {}\n地址:\n<code>{}</code>\n余额: {}",
        wallet.id,
        external_wallet_id,
        escape_html(&wallet.label),
        escape_html(&wallet.public_key),
        format_sol_balance(balance_lamports)
    )
}

async fn wallet_balance_for(
    state: &BotState,
    user_id: i64,
    wallet: &crate::db::models::WalletRecord,
) -> Option<u64> {
    let external_id = wallet.external_wallet_id.as_deref()?;
    bounded_wallet_read(
        state.wallet_service.balances(user_id),
        std::time::Duration::from_secs(5),
    )
    .await?
    .wallets
    .into_iter()
    .find(|item| item.wallet_id == external_id)?
    .balance_lamports
}

fn format_sol_balance(balance_lamports: Option<u64>) -> String {
    balance_lamports
        .map(|value| format!("{} SOL", format_lamports(value)))
        .unwrap_or_else(|| "暂不可用".to_string())
}

fn format_lamports(lamports: u64) -> String {
    let mut value = format!("{:.9}", lamports as f64 / 1_000_000_000.0);
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    value
}

async fn render_vanity_job(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    job: &crate::trade_client::VanityJobResponse,
) -> Result<()> {
    let side = if job.match_side == "prefix" {
        "开头"
    } else {
        "结尾"
    };
    let status = match job.status.as_str() {
        "queued" => "排队中",
        "running" => "生成中",
        "cancel_requested" => "正在取消",
        "cancelled" => "已取消",
        "completed" => "已完成",
        "exhausted" => "已达到最大尝试次数",
        "interrupted" => "服务重启，任务已中断",
        "failed" => "失败",
        value => value,
    };
    let result = job
        .pubkey
        .as_deref()
        .map(|pubkey| format!("\n地址: <code>{}</code>", escape_html(pubkey)))
        .unwrap_or_default();
    let error = job
        .error
        .as_deref()
        .map(|value| format!("\n错误: {}", escape_html(value)))
        .unwrap_or_default();
    let active = matches!(
        job.status.as_str(),
        "queued" | "running" | "cancel_requested"
    );
    bot.edit_message_text(
        chat_id,
        message_id,
        format!(
            "自定义钱包\n\n匹配: {side} {}\n状态: {status}\n尝试: {} / {}{result}{error}",
            escape_html(&job.pattern),
            job.attempts,
            job.max_attempts
        ),
    )
    .reply_markup(vanity_job_keyboard(&job.job_id, active))
    .await?;
    Ok(())
}

async fn render_transfer_amount(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    draft: &TransferDraft,
) -> Result<()> {
    let source = state.db.get_wallet(user_id, draft.source_wallet_id).await?;
    let destination = transfer_destination_text(state, user_id, &draft.destination).await?;
    let balance = wallet_balance_for(state, user_id, &source).await;
    bot.edit_message_text(
        chat_id,
        draft.panel_message_id,
        format!(
            "SOL 转账\n\n来源: {}\n余额: {}\n目标: {}\n\n请选择转账金额。50% 会保留免租余额；100% 扣除精确手续费后清空来源 SOL 账户。",
            escape_html(&source.label),
            format_sol_balance(balance),
            destination
        ),
    )
    .reply_markup(wallet_transfer_amount_keyboard(draft.source_wallet_id))
    .await?;
    Ok(())
}

async fn render_transfer_confirm(
    bot: &HtmlBot,
    chat_id: ChatId,
    state: &BotState,
    user_id: i64,
    draft: &TransferDraft,
) -> Result<()> {
    let source = state.db.get_wallet(user_id, draft.source_wallet_id).await?;
    let destination = transfer_destination_text(state, user_id, &draft.destination).await?;
    let amount = match draft.amount.context("尚未选择转账金额")? {
        TransferAmount::Percent(100) => "100%（扣除手续费后清空来源账户）".to_string(),
        TransferAmount::Percent(value) => format!("{value}%（保留免租余额）"),
        TransferAmount::FixedLamports(value) => format!("{} SOL", format_lamports(value)),
    };
    bot.edit_message_text(
        chat_id,
        draft.panel_message_id,
        format!(
            "确认 SOL 转账\n\n来源: {}\n目标: {}\n金额: {}\n\n确认时会重新读取余额和网络手续费。该操作不可撤销。",
            escape_html(&source.label), destination, amount
        ),
    )
    .reply_markup(wallet_transfer_confirm_keyboard(draft.source_wallet_id))
    .await?;
    Ok(())
}

async fn transfer_destination_text(
    state: &BotState,
    user_id: i64,
    destination: &TransferDestination,
) -> Result<String> {
    Ok(match destination {
        TransferDestination::Wallet(id) => {
            let wallet = state.db.get_wallet(user_id, *id).await?;
            format!(
                "{} <code>{}</code>",
                escape_html(&wallet.label),
                escape_html(&wallet.public_key)
            )
        }
        TransferDestination::Address(address) => {
            format!("<code>{}</code>", escape_html(address))
        }
    })
}

fn ensure_transferable_wallet(wallet: &crate::db::models::WalletRecord) -> Result<()> {
    if wallet.external_wallet_id.is_none() {
        anyhow::bail!("钱包尚未绑定到 trade_service");
    }
    Ok(())
}

fn parse_two_i64(value: &str) -> Result<(i64, i64)> {
    let (left, right) = value.split_once(':').context("回调参数格式无效")?;
    Ok((left.parse()?, right.parse()?))
}

fn format_transfer_result(result: &crate::trade_client::SolTransferResponse) -> String {
    let amount = result
        .amount_lamports
        .map(format_lamports)
        .unwrap_or_else(|| "-".to_string());
    let fee = result
        .fee_lamports
        .map(format_lamports)
        .unwrap_or_else(|| "-".to_string());
    let signature = result
        .signature
        .as_deref()
        .map(|value| format!("\n签名: <code>{}</code>", escape_html(value)))
        .unwrap_or_default();
    let error = result
        .error
        .as_deref()
        .map(|value| format!("\n错误: {}", escape_html(value)))
        .unwrap_or_default();
    format!(
        "SOL 转账结果\n\n状态: {}\n金额: {} SOL\n手续费: {} SOL\n接收地址: <code>{}</code>{signature}{error}",
        escape_html(&result.status),
        amount,
        fee,
        escape_html(&result.to_pubkey)
    )
}

fn help_text() -> &'static str {
    "独立跟单系统\n\n命令：\n/start 主菜单\n/status 状态\n/wallets 钱包\n/watch 跟踪钱包\n/watch_add &lt;钱包&gt; [标签]\n/strategies 策略\n/signals 最近信号\n/positions Dry-run 持仓\n/limits 限价单\n/preflight 实盘检查\n/buy &lt;mint&gt; &lt;SOL&gt;\n/sell &lt;mint&gt; &lt;百分比&gt;\n/pool &lt;Mint&gt; &lt;Pool ID&gt; &lt;协议&gt; 隔离 SDK 模拟池线索\n/orders 最近订单\n/import &lt;私钥&gt; 导入钱包\n/panic 全局暂停\n\n默认 Dry-run；选择 Live 并通过执行检查与确认后提交交易。"
}

fn user_trade_settings_defaults(config: &AppConfig, user_id: i64) -> UserTradeSettings {
    let closest_slot = |values: &[f64; 5], target: f64| {
        values
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                (**left - target).abs().total_cmp(&(**right - target).abs())
            })
            .map(|(index, _)| index)
            .unwrap_or(0)
    };
    UserTradeSettings {
        user_id,
        use_mev: config.use_mev,
        default_mev_provider: config.default_mev_provider.clone(),
        default_execution_mode: if config.dry_run_default {
            "dry_run".to_string()
        } else {
            "live".to_string()
        },
        default_execution_method: "sdk".to_string(),
        default_slippage_bps: config.default_slippage_bps,
        default_tip_sol: config.default_tip_sol,
        default_buy_preset_slot: closest_slot(
            &config.quick_buy_presets_sol,
            config.default_buy_amount_sol,
        ),
        default_sell_preset_slot: closest_slot(&config.quick_sell_presets_percent, 100.0),
        quick_trade_confirmation_enabled: true,
        buy_presets_sol: config.quick_buy_presets_sol,
        sell_presets_percent: config.quick_sell_presets_percent,
    }
}

async fn load_user_trade_settings(state: &BotState, user_id: i64) -> Result<UserTradeSettings> {
    state
        .db
        .get_or_create_user_trade_settings(&user_trade_settings_defaults(&state.config, user_id))
        .await
}

async fn render_settings_page(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    page: SettingsPage,
) -> Result<()> {
    let settings = load_user_trade_settings(state, user_id).await?;
    let (text, keyboard) = match page {
        SettingsPage::Home => (
            global_settings_text(&state.config, &settings),
            global_settings_keyboard(&settings),
        ),
        SettingsPage::Execution => (
            format!(
                "执行参数\n\n默认模式: {}\n执行方式: {}\n滑点: {}%\n小费: {} SOL",
                execution_mode_setting_label(&settings.default_execution_mode),
                settings.default_execution_method.to_ascii_uppercase(),
                trim_float(settings.default_slippage_bps as f64 / 100.0),
                trim_float(settings.default_tip_sol),
            ),
            execution_settings_keyboard(&settings),
        ),
        SettingsPage::ExecutionMode => (
            "选择默认模式".to_string(),
            execution_mode_selection_keyboard(&settings),
        ),
        SettingsPage::ExecutionMethod => (
            "选择执行方式".to_string(),
            execution_method_selection_keyboard(&settings),
        ),
        SettingsPage::Mev => (
            format!(
                "MEV 设置\n\nUSE_MEV: {}\nMEV Provider: {}\n执行方式: {}",
                if settings.use_mev { "开启" } else { "关闭" },
                settings.default_mev_provider,
                settings.default_execution_method.to_ascii_uppercase(),
            ),
            mev_settings_keyboard(&settings),
        ),
        SettingsPage::MevProvider => (
            "选择 MEV Provider".to_string(),
            mev_provider_selection_keyboard(&settings, &MEV_PROVIDERS),
        ),
        SettingsPage::Buy => (
            preset_settings_text(&settings, "buy"),
            trade_preset_settings_keyboard(&settings, "buy"),
        ),
        SettingsPage::Sell => (
            preset_settings_text(&settings, "sell"),
            trade_preset_settings_keyboard(&settings, "sell"),
        ),
        SettingsPage::DefaultBuyPreset => (
            "选择默认买入档位".to_string(),
            default_preset_selection_keyboard(&settings, "buy"),
        ),
        SettingsPage::DefaultSellPreset => (
            "选择默认卖出档位".to_string(),
            default_preset_selection_keyboard(&settings, "sell"),
        ),
    };
    let result = bot
        .edit_message_text(chat_id, message_id, text)
        .reply_markup(keyboard)
        .await;
    if let Err(err) = result {
        if !err.to_string().contains("message is not modified") {
            return Err(err.into());
        }
    }
    Ok(())
}

async fn handle_settings_callback(
    bot: &HtmlBot,
    message: &MaybeInaccessibleMessage,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
    data: &str,
) -> Result<()> {
    let chat_id = message.chat().id;
    let message_id = message.id();
    let mut settings = load_user_trade_settings(state, user_id).await?;
    let page = match data {
        "settings:home" | "settings:refresh" => SettingsPage::Home,
        "settings:execution" => SettingsPage::Execution,
        "settings:mev" => SettingsPage::Mev,
        "settings:buy" => SettingsPage::Buy,
        "settings:sell" => SettingsPage::Sell,
        "settings:choose:mode" => SettingsPage::ExecutionMode,
        "settings:choose:method" => SettingsPage::ExecutionMethod,
        "settings:choose:provider" => SettingsPage::MevProvider,
        "settings:choose:default:buy" => SettingsPage::DefaultBuyPreset,
        "settings:choose:default:sell" => SettingsPage::DefaultSellPreset,
        value if value.starts_with("settings:select:mode:") => {
            let mode = value.trim_start_matches("settings:select:mode:");
            if !matches!(mode, "dry_run" | "live") {
                anyhow::bail!("不支持的默认模式");
            }
            state
                .db
                .update_user_trade_setting(user_id, "default_execution_mode", mode)
                .await?;
            SettingsPage::Execution
        }
        value if value.starts_with("settings:select:method:") => {
            let method = value.trim_start_matches("settings:select:method:");
            if !matches!(method, "sdk" | "pumpapi") {
                anyhow::bail!("不支持的执行方式");
            }
            state
                .db
                .update_user_trade_setting(user_id, "default_execution_method", method)
                .await?;
            if method == "pumpapi" && settings.use_mev {
                state
                    .db
                    .update_user_trade_setting(user_id, "use_mev", "0")
                    .await?;
            }
            SettingsPage::Execution
        }
        "settings:toggle_mev" => {
            let enabled = !settings.use_mev;
            state
                .db
                .update_user_trade_setting(user_id, "use_mev", if enabled { "1" } else { "0" })
                .await?;
            if enabled && settings.default_execution_method == "pumpapi" {
                state
                    .db
                    .update_user_trade_setting(user_id, "default_execution_method", "sdk")
                    .await?;
            }
            SettingsPage::Mev
        }
        "settings:toggle_quick_confirmation" => {
            state
                .db
                .update_user_trade_setting(
                    user_id,
                    "quick_trade_confirmation_enabled",
                    if settings.quick_trade_confirmation_enabled {
                        "0"
                    } else {
                        "1"
                    },
                )
                .await?;
            SettingsPage::Home
        }
        value if value.starts_with("settings:select:provider:") => {
            let provider = value.trim_start_matches("settings:select:provider:");
            if !MEV_PROVIDERS.contains(&provider) {
                anyhow::bail!("不支持的 MEV Provider");
            }
            state
                .db
                .update_user_trade_setting(user_id, "default_mev_provider", provider)
                .await?;
            SettingsPage::Mev
        }
        value if value.starts_with("settings:select:default:") => {
            let parts = value.split(':').collect::<Vec<_>>();
            let side = parts.get(3).copied().unwrap_or_default();
            let index = parts
                .get(4)
                .ok_or_else(|| anyhow::anyhow!("缺少默认档位"))?
                .parse::<usize>()?;
            if index >= 5 || !matches!(side, "buy" | "sell") {
                anyhow::bail!("默认档位无效");
            }
            let (field, page) = if side == "buy" {
                ("default_buy_preset_slot", SettingsPage::Buy)
            } else {
                ("default_sell_preset_slot", SettingsPage::Sell)
            };
            state
                .db
                .update_user_trade_setting(user_id, field, &(index + 1).to_string())
                .await?;
            page
        }
        value if value.starts_with("settings:input:") => {
            let field = if value == "settings:input:slippage" {
                SettingsInputField::Slippage
            } else if value == "settings:input:tip" {
                SettingsInputField::Tip
            } else {
                let parts = value.split(':').collect::<Vec<_>>();
                let index = parts
                    .get(3)
                    .ok_or_else(|| anyhow::anyhow!("缺少预设档位"))?
                    .parse::<usize>()?;
                if index >= 5 {
                    anyhow::bail!("预设档位无效");
                }
                if parts.get(2) == Some(&"buy") {
                    SettingsInputField::BuyPreset(index)
                } else {
                    SettingsInputField::SellPreset(index)
                }
            };
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::SettingsInput {
                    field,
                    settings_message_id: message_id,
                },
            );
            return Ok(());
        }
        _ => return Ok(()),
    };
    settings = load_user_trade_settings(state, user_id).await?;
    state.follow_strategy.refresh_cache().await?;
    state
        .db
        .audit(
            Some(user_id),
            "trade_settings.updated",
            Some("user_trade_settings"),
            Some(&user_id.to_string()),
            Some(&serde_json::json!({ "action": data, "settings": settings }).to_string()),
        )
        .await?;
    state.pending.lock().await.remove(&telegram_id);
    render_settings_page(bot, chat_id, message_id, state, user_id, page).await
}

fn preset_settings_text(settings: &UserTradeSettings, side: &str) -> String {
    let (title, values, default_slot, unit) = if side == "buy" {
        (
            "买入金额设置",
            settings.buy_presets_sol,
            settings.default_buy_preset_slot,
            "SOL",
        )
    } else {
        (
            "卖出比例设置",
            settings.sell_presets_percent,
            settings.default_sell_preset_slot,
            "%",
        )
    };
    let lines = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            format!(
                "{}{}: {} {}",
                if index == default_slot { "✅ " } else { "" },
                index + 1,
                trim_float(*value),
                unit
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("{title}\n\n默认档位: {}\n{}", default_slot + 1, lines)
}

fn global_settings_text(_config: &AppConfig, settings: &UserTradeSettings) -> String {
    let buy_presets = settings
        .buy_presets_sol
        .iter()
        .map(|value| trim_float(*value))
        .collect::<Vec<_>>()
        .join(" / ");
    let sell_presets = settings
        .sell_presets_percent
        .iter()
        .map(|value| trim_float(*value))
        .collect::<Vec<_>>()
        .join("% / ");

    format!(
        "全局设置\n\n默认模式: {}\n执行方式: {}\n滑点: {}%\n小费: {} SOL\nUSE_MEV: {}\nMEV Provider: {}\n快速交易确认: {}\n买入快捷金额: {} SOL\n卖出快捷比例: {}%",
        execution_mode_setting_label(&settings.default_execution_mode),
        settings.default_execution_method.to_ascii_uppercase(),
        trim_float(settings.default_slippage_bps as f64 / 100.0),
        trim_float(settings.default_tip_sol),
        if settings.use_mev { "开启" } else { "关闭" },
        settings.default_mev_provider,
        if settings.quick_trade_confirmation_enabled {
            "开启"
        } else {
            "关闭"
        },
        buy_presets,
        sell_presets,
    )
}

fn execution_mode_setting_label(value: &str) -> &'static str {
    if value == "live" {
        "Live"
    } else {
        "Dry-run"
    }
}

#[cfg(test)]
mod trade_message_routing_tests {
    use super::{starts_new_trade_panel, PendingAction, TradeSide};

    const MINT: &str = "39ahtL8ynzE4amH26J29C93PA5172V3ft9UuUcqQS8fz";

    #[test]
    fn resent_mint_reopens_panel_while_waiting_for_sell_input() {
        let sell = PendingAction::ManualTrade {
            side: TradeSide::Sell,
        };
        let amount = PendingAction::ManualTradeAmount {
            side: TradeSide::Sell,
            mint: MINT.to_string(),
        };
        assert!(starts_new_trade_panel(&sell, MINT));
        assert!(starts_new_trade_panel(&amount, MINT));
        assert!(!starts_new_trade_panel(&amount, "100"));
        assert!(!starts_new_trade_panel(&sell, &format!("{MINT} 100")));
    }

    #[test]
    fn wallet_operations_keep_their_address_input() {
        assert!(!starts_new_trade_panel(&PendingAction::AddWatch, MINT));
        assert!(!starts_new_trade_panel(&PendingAction::ImportWallet, MINT));
        assert!(!starts_new_trade_panel(
            &PendingAction::RenameWallet { wallet_id: 3 },
            MINT
        ));
    }
}

#[cfg(test)]
mod manual_trade_result_tests {
    use super::manual_trade_result_text;
    use crate::trade_client::TradeResponse;

    #[test]
    fn backend_failure_is_visible_without_claiming_acceptance() {
        let response: TradeResponse = serde_json::from_value(serde_json::json!({
            "status":"failed", "request_id":"tr_test", "signature":null,
            "error":"Temporal: sell fixed output amount must be greater than zero"
        }))
        .unwrap();
        let text = manual_trade_result_text("卖出", Some("2TE"), "failed", "order", &response);
        assert!(text.contains("上笔卖出失败"));
        assert!(text.contains(response.error.as_deref().unwrap()));
        assert!(text.contains("不能证明钱包余额为零"));
        assert!(!text.contains("已进入"));
    }

    #[test]
    fn errors_are_html_escaped_and_submitted_is_not_confirmed() {
        let mut response: TradeResponse = serde_json::from_value(serde_json::json!({
            "status":"submitted", "signature":"sig"
        }))
        .unwrap();
        let text = manual_trade_result_text("买入", Some("2TE"), "submitted", "order", &response);
        assert!(!text.contains("失败"));
        assert!(!text.contains("成功"));
        response.error = Some("bad <pool>&quote".to_string());
        let text = manual_trade_result_text("卖出", Some("2TE"), "failed", "order", &response);
        assert!(text.contains("bad &lt;pool&gt;&amp;quote"));
    }
}

#[cfg(test)]
mod quote_amount_tests {
    use super::{format_quote_units, quote_amount_units};

    #[test]
    fn converts_quote_decimal_exactly() {
        assert_eq!(quote_amount_units("9.03156134", 8).unwrap(), 903_156_134);
        assert_eq!(format_quote_units(903_156_134, 8), "9.03156134");
        assert_eq!(quote_amount_units("1", 6).unwrap(), 1_000_000);
    }

    #[test]
    fn rejects_precision_and_overflow() {
        assert!(quote_amount_units("0.000000001", 8).is_err());
        assert!(quote_amount_units("1e3", 8).is_err());
        assert!(quote_amount_units("18446744073709551616", 8).is_err());
        assert!(quote_amount_units("0", 8).is_err());
    }

    #[test]
    fn quote_order_amount_is_not_labeled_sol() {
        let request = r#"{"quote_mint":"Xsf9mBktVB9BSU5kf4nHxPq5hCBJ2j2ui3ecFGxPRGc","amount_quote_units":903156134}"#;
        assert!(super::quote_order_amount(Some(request))
            .unwrap()
            .contains("903156134 最小单位"));
    }
}

#[cfg(test)]
mod wallet_panel_tests {
    use super::bounded_wallet_read;
    use std::time::Duration;

    #[tokio::test]
    async fn stalled_wallet_server_returns_unavailable() {
        let started = std::time::Instant::now();
        let result = bounded_wallet_read(
            std::future::pending::<anyhow::Result<u64>>(),
            Duration::from_millis(20),
        )
        .await;
        assert!(result.is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn zero_balance_is_preserved_and_errors_are_unavailable() {
        let budget = Duration::from_millis(20);
        assert_eq!(
            bounded_wallet_read(async { Ok(0_u64) }, budget).await,
            Some(0)
        );
        assert!(bounded_wallet_read(
            async { Err::<u64, _>(anyhow::anyhow!("unavailable")) },
            budget
        )
        .await
        .is_none());
    }
}

#[cfg(test)]
mod trade_panel_read_tests {
    use super::{bounded_panel_read, pool_resolution_error_text};
    use std::time::Duration;

    #[tokio::test]
    async fn stalled_panel_read_times_out_with_context() {
        let result = bounded_panel_read(
            std::future::pending::<anyhow::Result<()>>(),
            Duration::from_millis(20),
            "pool resolution timeout",
        )
        .await;
        let error = result.unwrap_err();
        assert!(pool_resolution_error_text(&error).contains("交易服务暂不可用"));
    }

    #[test]
    fn connection_error_in_context_chain_is_reported_as_service_unavailable() {
        let error = anyhow::anyhow!("connection refused").context("failed to resolve mint pool");
        let text = pool_resolution_error_text(&error);
        assert!(text.contains("交易服务暂不可用"));
        assert!(!text.contains("备用节点"));
    }

    #[test]
    fn missing_pool_is_distinct_from_service_failure() {
        assert!(
            pool_resolution_error_text(&anyhow::anyhow!("pool not found"))
                .contains("未找到支持的 Pool")
        );
    }
}

#[cfg(test)]
mod via_sol_tests {
    use super::*;
    use crate::trade_client::SolHopResponse;

    fn hop() -> SolHopResponse {
        SolHopResponse {
            available: true,
            pool: Some("raydium-cpmm".into()),
            pool_id: Some("So11111111111111111111111111111111111111112".into()),
            reason: None,
        }
    }

    #[test]
    fn background_result_preserves_amount_and_rejects_old_panel_or_generation() {
        let (mut panel, _) = trade_panel::custom_quote_fixture();
        panel.message_id = Some(MessageId(1));
        panel.sol_hop_generation = 1;
        panel.sol_hop_pending = true;
        let mut completed = panel.clone();
        completed.sol_hop = Some(hop());
        panel.quick.buy_amount_sol = 0.005;
        assert!(apply_hop_result(&mut panel, &completed));
        assert_eq!(panel.quick.buy_amount_sol, 0.005);
        assert!(!panel.sol_hop_pending);
        assert_eq!(panel.quick.execution.mode, ExecutionMode::Live);
        panel.sol_hop_generation = 2;
        assert!(!apply_hop_result(&mut panel, &completed));
        panel.sol_hop_generation = 1;
        panel.message_id = Some(MessageId(2));
        assert!(!apply_hop_result(&mut panel, &completed));
        panel.message_id = Some(MessageId(1));
        panel.route.quote_mint = "new-quote".into();
        assert!(!apply_hop_result(&mut panel, &completed));
    }

    #[test]
    fn http_success_with_unavailable_timeout_does_not_allow_submission() {
        let (mut panel, settings) = trade_panel::custom_quote_fixture();
        panel.sol_hop_pending = true;
        let mut completed = panel.clone();
        completed.sol_hop = Some(SolHopResponse {
            available: false,
            pool: None,
            pool_id: None,
            reason: Some(
                "no verified WSOL/Quote Raydium CPMM pool: SOL hop discovery timed out".into(),
            ),
        });
        completed.sol_hop_error = completed.sol_hop.as_ref().unwrap().reason.clone();
        assert!(apply_hop_result(&mut panel, &completed));
        assert!(panel.sol_hop_status().contains("寻池超时"));
        assert!(quick_trade_draft(&panel, &settings).is_err());
    }

    #[test]
    fn pending_background_lookup_cannot_submit_and_shows_progress() {
        let (mut panel, settings) = trade_panel::custom_quote_fixture();
        panel.sol_hop_pending = true;
        assert!(panel.sol_hop_status().contains("后台寻池"));
        assert!(quick_trade_draft(&panel, &settings).is_err());
    }

    #[test]
    fn confirmation_buttons_must_match_current_side_and_settlement() {
        let (mut panel, _) = trade_panel::custom_quote_fixture();
        assert!(confirmation_matches(&panel, "panel:confirm:buy:sol"));
        assert!(!confirmation_matches(&panel, "panel:confirm:buy"));
        assert!(!confirmation_matches(&panel, "panel:confirm:buy:quote"));
        assert!(!confirmation_matches(&panel, "panel:confirm:sell:sol"));
        panel.select_settlement(false).unwrap();
        assert!(confirmation_matches(&panel, "panel:confirm:buy:quote"));
        assert!(!confirmation_matches(&panel, "panel:confirm:buy:sol"));
    }

    #[test]
    fn screenshot_mint_shows_sol_presets_but_rpc_failure_cannot_submit() {
        let (mut panel, mut settings) = trade_panel::custom_quote_fixture();
        panel.mint = "4FEYQw5d7a3A7JvD7gXnK37EiHVJcuTG3ssKGF9RfarS".into();
        panel.route.mint = panel.mint.clone();
        panel.route.pool = "stonkfun-swap".into();
        panel.route.quote_mint = "VVV4UdRywr7SLafPHzqXwRCw7myXkmgCtitXUvj8g8c".into();
        panel.sol_hop_error =
            Some("no verified WSOL/Quote Raydium CPMM pool: HTTP 429 Too Many Requests".into());
        settings.buy_presets_sol = [0.001, 1.0, 5.0, 10.0, 0.5];
        let keyboard = serde_json::to_string(&panel.keyboard(&settings, false)).unwrap();
        for label in [
            "0.001 SOL",
            "1 SOL",
            "5 SOL",
            "10 SOL",
            "0.5 SOL",
            "确认买入 0.001 SOL",
        ] {
            assert!(keyboard.contains(label), "missing preset: {label}");
        }
        assert!(panel.text("wallet", true).contains("RPC 限流"));
        assert!(!panel.text("wallet", true).contains("HTTP 429"));
        assert!(quick_trade_draft(&panel, &settings).is_err());
        panel.quick.buy_amount_sol = 1.0;
        assert!(quick_trade_draft(&panel, &settings).is_err());
    }

    #[test]
    fn via_sol_buy_and_sell_serialize_one_amount_and_exact_settlement() {
        let (mut panel, settings) = trade_panel::custom_quote_fixture();
        panel.sol_hop = Some(hop());
        panel.select_settlement(true).unwrap();
        let mut draft = quick_trade_draft(&panel, &settings).unwrap();
        draft.execution.mode = ExecutionMode::Simulate;
        let request = manual_trade_request(
            &draft,
            &draft.execution,
            "wallet".into(),
            None,
            "stable-key",
        )
        .unwrap();
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["settlement_asset"], "sol");
        assert_eq!(json["sol_hop_pool"], "raydium-cpmm");
        assert_eq!(json["amount_sol"], 0.001);
        assert!(json["amount_quote_units"].is_null());
        assert_eq!(json["simulate"], true);
        assert_eq!(json["idempotency_key"], "stable-key");
        assert!(manual_trade_request(
            &draft,
            &draft.execution,
            "wallet".into(),
            Some(1),
            "stable-key"
        )
        .is_err());
        panel.quick.side = PanelSide::Sell;
        let draft = quick_trade_draft(&panel, &settings).unwrap();
        let request =
            manual_trade_request(&draft, &draft.execution, "wallet".into(), None, "sell-key")
                .unwrap();
        assert_eq!(request.sell_percent, Some(25));
        assert!(request.amount_sol.is_none());
        assert!(request.amount_quote_units.is_none());
        assert_eq!(request.settlement_asset.as_deref(), Some("sol"));
    }

    #[test]
    fn route_failures_pumpapi_and_migration_block_without_quote_fallback() {
        let (mut panel, settings) = trade_panel::custom_quote_fixture();
        panel.sol_hop = Some(hop());
        panel.select_settlement(true).unwrap();
        let draft = quick_trade_draft(&panel, &settings).unwrap();
        assert!(validate_sol_hop_refresh(&draft, &draft.route, &hop()).is_ok());
        let mut route = draft.route.clone();
        route.pool = "stonkfun-swap".into();
        assert!(validate_sol_hop_refresh(&draft, &route, &hop()).is_err());
        let mut changed = hop();
        changed.pool_id = Some("11111111111111111111111111111111".into());
        assert!(validate_sol_hop_refresh(&draft, &draft.route, &changed).is_err());
        panel.sol_hop = None;
        assert!(quick_trade_draft(&panel, &settings).is_err());
        panel.sol_hop = Some(hop());
        panel.quick.execution.method = ExecutionMethod::PumpApi;
        assert!(quick_trade_draft(&panel, &settings).is_err());
        assert!(panel.quick.via_sol);
    }

    #[test]
    fn direct_quote_keeps_units_and_does_not_emit_sol_hop_fields() {
        let (mut panel, settings) = trade_panel::custom_quote_fixture();
        panel.select_settlement(false).unwrap();
        panel.quick.buy_amount_quote = Some("1.25".into());
        let draft = quick_trade_draft(&panel, &settings).unwrap();
        let request = manual_trade_request(
            &draft,
            &draft.execution,
            "wallet".into(),
            Some(1_250_000),
            "quote-key",
        )
        .unwrap();
        assert!(request.amount_sol.is_none());
        assert_eq!(request.amount_quote_units, Some(1_250_000));
        assert!(request.settlement_asset.is_none());
        assert!(request.sol_hop_pool_id.is_none());
    }
}

#[cfg(test)]
mod isolated_sdk_policy_tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn only_verified_shape_single_pool_sell_is_admitted() {
        let mut draft = TradeDraft {
            side: TradeSide::Sell,
            mint: crate::trade_client::WSOL_MINT.into(),
            amount: 10.0,
            execution: ExecutionParams {
                wallet_id: None,
                mode: ExecutionMode::Simulate,
                method: ExecutionMethod::Sdk,
                slippage_bps: 100,
                tip_sol: 0.0001,
                mev_mode: "none".into(),
            },
            route: ResolvedPoolRoute {
                mint: crate::trade_client::WSOL_MINT.into(),
                pool: "raydium-cpmm".into(),
                pool_id: Some("fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2".into()),
                quote_mint: crate::trade_client::USDC_MINT.into(),
            },
            quote_amount: None,
            sol_hop_pool_id: None,
        };
        assert!(isolated_sdk_single_pool_supported(&draft));
        let request = manual_trade_request(
            &draft,
            &draft.execution,
            "2TE".into(),
            None,
            "isolated-sell-test",
        )
        .unwrap();
        assert_eq!(request.execution_method, "sdk");
        assert_eq!(request.side, "sell");
        assert_eq!(request.pool, "raydium-cpmm");
        assert_eq!(request.mint, crate::trade_client::WSOL_MINT);
        assert_eq!(
            request.quote_mint.as_deref(),
            Some(crate::trade_client::USDC_MINT)
        );
        assert_eq!(request.sell_percent, Some(10));
        assert!(request.amount_sol.is_none() && request.simulate);
        draft.side = TradeSide::Buy;
        assert!(!isolated_sdk_single_pool_supported(&draft));
        draft.side = TradeSide::Sell;
        draft.route.pool_id = None;
        assert!(!isolated_sdk_single_pool_supported(&draft));
        draft.route.pool_id = Some("fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2".into());
        draft.mint = "other-mint".into();
        assert!(!isolated_sdk_single_pool_supported(&draft));
        draft.mint = crate::trade_client::WSOL_MINT.into();
        draft.sol_hop_pool_id = Some("bridge".into());
        assert!(!isolated_sdk_single_pool_supported(&draft));
    }

    #[test]
    fn isolated_bridge_buy_requires_explicit_non_wsol_target_pool() {
        let draft = TradeDraft {
            side: TradeSide::Buy,
            mint: "nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump".into(),
            amount: 0.001,
            execution: ExecutionParams {
                wallet_id: None,
                mode: ExecutionMode::Simulate,
                method: ExecutionMethod::Sdk,
                slippage_bps: 100,
                tip_sol: 0.0001,
                mev_mode: "none".into(),
            },
            route: ResolvedPoolRoute {
                mint: "nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump".into(),
                pool: "pump-amm".into(),
                pool_id: Some("G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b".into()),
                quote_mint: "TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo".into(),
            },
            quote_amount: None,
            sol_hop_pool_id: None,
        };
        assert!(isolated_sdk_bridge_buy_supported(&draft));
        assert!(isolated_sdk_simulation_supported(&draft));
        let request = manual_trade_request(
            &draft,
            &draft.execution,
            "w1".into(),
            None,
            "bridge-request",
        )
        .unwrap();
        let payload = serde_json::to_value(&request).unwrap();
        assert!(payload.get("tip_sol").is_none());
        assert!(payload.get("use_mev").is_none());
        assert_eq!(payload["amount_sol"], 0.001);
        assert_eq!(payload["execution_method"], "sdk");
        let mut changed = draft.clone();
        changed.side = TradeSide::Sell;
        assert!(!isolated_sdk_bridge_buy_supported(&changed));
        changed = draft.clone();
        changed.route.pool_id = None;
        assert!(!isolated_sdk_bridge_buy_supported(&changed));
        changed = draft.clone();
        changed.route.quote_mint = crate::trade_client::WSOL_MINT.into();
        assert!(!isolated_sdk_bridge_buy_supported(&changed));
        changed = draft;
        changed.route.mint = "different-mint".into();
        assert!(!isolated_sdk_bridge_buy_supported(&changed));
    }

    #[test]
    fn isolated_cpmm_usdt_sell_requires_two_explicit_pools_and_omits_mev_fields() {
        let mut draft = TradeDraft {
            side: TradeSide::Sell,
            mint: "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh".into(),
            amount: 100.0,
            execution: ExecutionParams {
                wallet_id: None,
                mode: ExecutionMode::Simulate,
                method: ExecutionMethod::Sdk,
                slippage_bps: 100,
                tip_sol: 0.0001,
                mev_mode: "none".into(),
            },
            route: ResolvedPoolRoute {
                mint: "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh".into(),
                pool: "raydium-cpmm".into(),
                pool_id: Some("bsitkqbNk5dPgyrVpHRjiAPNQk66xCfNs8wE981Cd8e".into()),
                quote_mint: crate::trade_client::USDT_MINT.into(),
            },
            quote_amount: None,
            sol_hop_pool_id: Some("AddZXoGEs6pR6bgp51vjNasKN8jFnizsu6j5P3PSphqk".into()),
        };
        assert!(isolated_sdk_bridge_sell_supported(&draft));
        let request =
            manual_trade_request(&draft, &draft.execution, "2TE".into(), None, "sell").unwrap();
        let payload = serde_json::to_value(request).unwrap();
        assert_eq!(payload["settlement_asset"], "sol");
        assert_eq!(payload["sol_hop_pool"], "raydium-cpmm");
        assert_eq!(payload["sell_percent"], 100);
        assert!(payload.get("tip_sol").is_none());
        assert!(payload.get("use_mev").is_none());
        draft.sol_hop_pool_id = None;
        assert!(!isolated_sdk_bridge_sell_supported(&draft));
        draft.sol_hop_pool_id = Some("AddZXoGEs6pR6bgp51vjNasKN8jFnizsu6j5P3PSphqk".into());
        draft.route.quote_mint = crate::trade_client::USDC_MINT.into();
        assert!(!isolated_sdk_bridge_sell_supported(&draft));
    }

    #[test]
    fn cpmm_usdt_sell_panel_requires_verified_hop_before_creating_sdk_draft() {
        let (mut panel, settings) = trade_panel::custom_quote_fixture();
        panel.mint = "3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh".into();
        panel.route = ResolvedPoolRoute {
            mint: panel.mint.clone(),
            pool: "raydium-cpmm".into(),
            pool_id: Some("bsitkqbNk5dPgyrVpHRjiAPNQk66xCfNs8wE981Cd8e".into()),
            quote_mint: crate::trade_client::USDT_MINT.into(),
        };
        panel.quick.side = PanelSide::Sell;
        panel.quick.execution.mode = ExecutionMode::Simulate;
        panel.isolated_sdk_simulation_enabled = true;
        assert!(quick_trade_draft(&panel, &settings).is_err());
        panel.sol_hop = Some(crate::trade_client::SolHopResponse {
            available: true,
            pool: Some("raydium-cpmm".into()),
            pool_id: Some("AddZXoGEs6pR6bgp51vjNasKN8jFnizsu6j5P3PSphqk".into()),
            reason: None,
        });
        let draft = quick_trade_draft(&panel, &settings).unwrap();
        assert!(isolated_sdk_bridge_sell_supported(&draft));
        assert_eq!(draft.sol_hop_pool_id, panel.sol_hop.unwrap().pool_id);
    }

    #[tokio::test]
    async fn manual_simulation_persists_isolated_order_without_primary_request() {
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let telegram = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated_url = format!("http://{}", isolated.local_addr().unwrap());
        let telegram_url = format!("http://{}/", telegram.local_addr().unwrap());
        let service = tokio::spawn(async move {
            let responses = [
                (
                    "GET /ready ",
                    r#"{"ready":true,"default_wallet_id":"w1","wallets":[{"wallet_id":"w1","pubkey":"test-pubkey"}]}"#,
                ),
                (
                    "GET /ready ",
                    r#"{"ready":true,"default_wallet_id":"w1","wallets":[{"wallet_id":"w1","pubkey":"test-pubkey"}]}"#,
                ),
                (
                    "POST /v1/routes/decide ",
                    r#"{"schema_version":1,"status":"sdk_verified","reason":null,"wallet_id":"w1","wallet_pubkey":"test-pubkey","decision_id":"decision-1","route_id":"route-1","expires_at_ms":9999999999999,"simulate_only":true}"#,
                ),
                (
                    "POST /v1/trades ",
                    r#"{"request_id":"sim-1","wallet_id":"w1","wallet_pubkey":"test-pubkey","status":"simulated","signature":null}"#,
                ),
            ];
            for (expected, body) in responses {
                let (mut socket, _) = isolated.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..count]);
                assert!(wire.starts_with(expected), "{wire}");
                if expected == "POST /v1/trades " {
                    assert!(wire.contains("\"simulate\":true"));
                    assert!(wire.contains("\"sdk_decision_id\":\"decision-1\""));
                }
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let telegram_server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = telegram.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..count]);
                assert!(wire.contains("EditMessageText"), "{wire}");
                let body = r#"{"ok":true,"result":{"message_id":1,"date":0,"chat":{"id":1,"type":"private","first_name":"Test"},"text":"ok"}}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let path = std::env::temp_dir().join(format!("bot-manual-sim-{}.sqlite", Uuid::new_v4()));
        let db_url = format!("sqlite://{}", path.display());
        let db = Database::connect(&db_url).await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(1, None).await.unwrap();
        let wallet_id = db
            .insert_external_wallet(user.id, "w1", "Mock", "test-pubkey")
            .await
            .unwrap();
        let mut config = test_config();
        config.trade_service_url = format!("http://{}", primary.local_addr().unwrap());
        config.route_decision_url = Some(isolated_url.clone());
        config.route_decision_token = Some("isolated-token".into());
        config.trade_submission_enabled = false;
        config.isolated_sdk_simulation_enabled = true;
        let client = TradeServiceClient::new(config.trade_service_url.clone(), None, 2)
            .unwrap()
            .with_route_decision_endpoint(Some(isolated_url), Some("isolated-token".into()))
            .unwrap();
        let secret_box = crate::crypto::SecretBox::from_hex(&config.master_key_hex).unwrap();
        let wallet_service = WalletService::new(db.clone(), secret_box, client.clone());
        let oracle = PriceOracle::new(None, "".into(), None, 0);
        let risk = crate::risk::RiskEngine::new();
        let executor =
            crate::executor::ExecutionRouter::new(db.clone(), risk.clone(), client.clone(), 1, 1);
        let strategy = StrategyEngine::new(config.clone(), db.clone(), risk, executor, None);
        let routes = PoolRouteCache::new(db.clone(), client.clone(), 1);
        let state = BotState::new(
            config,
            db.clone(),
            wallet_service,
            client,
            oracle,
            strategy,
            routes,
        );
        let bot = Bot::new("123:ABC")
            .set_api_url(reqwest::Url::parse(&telegram_url).unwrap())
            .parse_mode(ParseMode::Html);
        let draft = TradeDraft {
            side: TradeSide::Sell,
            mint: crate::trade_client::WSOL_MINT.into(),
            amount: 10.0,
            execution: ExecutionParams {
                wallet_id: Some(wallet_id),
                mode: ExecutionMode::Simulate,
                method: ExecutionMethod::Sdk,
                slippage_bps: 100,
                tip_sol: 0.0001,
                mev_mode: "none".into(),
            },
            route: ResolvedPoolRoute {
                mint: crate::trade_client::WSOL_MINT.into(),
                pool: "raydium-cpmm".into(),
                pool_id: Some("fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2".into()),
                quote_mint: crate::trade_client::USDC_MINT.into(),
            },
            quote_amount: None,
            sol_hop_pool_id: None,
        };
        execute_trade_draft(&bot, ChatId(1), MessageId(1), &state, user.id, 1, draft)
            .await
            .unwrap();
        service.await.unwrap();
        telegram_server.await.unwrap();
        let pool = sqlx::SqlitePool::connect(&db_url).await.unwrap();
        let (status, origin, request_json): (String, String, String) = sqlx::query_as(
            "SELECT status, trade_service_origin, trade_service_request_json FROM orders LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, "simulated");
        assert_eq!(origin, "isolated_sdk_simulation");
        assert!(request_json.contains("\"sdk_decision_id\":\"decision-1\""));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
        pool.close().await;
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    #[ignore = "requires a funded, disposable SDK 11 candidate and local HTTP access"]
    async fn telegram_panel_buy_and_sell_reconcile_against_real_sdk_candidate() {
        let isolated_url = std::env::var("BOT_SDK11_TEST_URL").unwrap();
        let isolated_token = std::env::var("BOT_SDK11_TEST_TOKEN").unwrap();
        let service_wallet_id = std::env::var("BOT_SDK11_TEST_WALLET_ID").unwrap();
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let telegram = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let telegram_url = format!("http://{}/", telegram.local_addr().unwrap());
        let telegram_server = tokio::spawn(async move {
            for (index, expected) in [
                "SendMessage",
                "EditMessageText",
                "EditMessageText",
                "AnswerCallbackQuery",
                "EditMessageText",
                "AnswerCallbackQuery",
                "EditMessageText",
                "EditMessageText",
                "SendMessage",
                "EditMessageText",
                "EditMessageText",
                "AnswerCallbackQuery",
                "EditMessageText",
                "EditMessageText",
            ]
            .into_iter()
            .enumerate()
            {
                let (mut socket, _) = telegram.accept().await.unwrap();
                let mut buffer = [0; 16384];
                let count = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..count]);
                assert!(wire.contains(expected), "{wire}");
                if index == 4 || index == 10 {
                    assert!(wire.contains("模拟本笔"), "{wire}");
                }
                let body = if expected == "AnswerCallbackQuery" {
                    r#"{"ok":true,"result":true}"#
                } else {
                    r#"{"ok":true,"result":{"message_id":1,"date":0,"chat":{"id":1,"type":"private","first_name":"Test"},"text":"ok"}}"#
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let primary_url = format!("http://{}", primary.local_addr().unwrap());
        let client = TradeServiceClient::new(primary_url.clone(), None, 60)
            .unwrap()
            .with_route_decision_endpoint(Some(isolated_url), Some(isolated_token))
            .unwrap();
        let ready = client.isolated_sdk_ready().await.unwrap();
        assert!(ready.ready);
        let wallet_pubkey = ready
            .wallets
            .iter()
            .find(|wallet| wallet.wallet_id == service_wallet_id)
            .unwrap()
            .pubkey
            .clone();
        let path = std::env::temp_dir().join(format!("bot-real-sdk11-{}.sqlite", Uuid::new_v4()));
        let db_url = format!("sqlite://{}", path.display());
        let db = Database::connect(&db_url).await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(1, None).await.unwrap();
        let wallet_id = db
            .insert_external_wallet(user.id, &service_wallet_id, "Isolated", &wallet_pubkey)
            .await
            .unwrap();
        let mut config = test_config();
        config.trade_service_url = primary_url;
        config.route_decision_url = Some(std::env::var("BOT_SDK11_TEST_URL").unwrap());
        config.route_decision_token = Some(std::env::var("BOT_SDK11_TEST_TOKEN").unwrap());
        config.trade_service_timeout_secs = 60;
        config.trade_submission_enabled = false;
        config.isolated_sdk_simulation_enabled = true;
        let secret_box = crate::crypto::SecretBox::from_hex(&config.master_key_hex).unwrap();
        let wallet_service = WalletService::new(db.clone(), secret_box, client.clone());
        let oracle = PriceOracle::new(None, "".into(), None, 0);
        let risk = crate::risk::RiskEngine::new();
        let executor =
            crate::executor::ExecutionRouter::new(db.clone(), risk.clone(), client.clone(), 1, 1);
        let strategy = StrategyEngine::new(config.clone(), db.clone(), risk, executor, None);
        let routes = PoolRouteCache::new(db.clone(), client.clone(), 10);
        let state = Arc::new(BotState::new(
            config,
            db.clone(),
            wallet_service,
            client.clone(),
            oracle,
            strategy,
            routes,
        ));
        let bot = Bot::new("123:ABC")
            .set_api_url(reqwest::Url::parse(&telegram_url).unwrap())
            .parse_mode(ParseMode::Html);
        let message: Message = serde_json::from_str(
            r#"{"message_id":10,"date":0,"chat":{"id":1,"type":"private","first_name":"Test"},"from":{"id":1,"is_bot":false,"first_name":"Test"},"text":"/pool So11111111111111111111111111111111111111112 fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2 raydium-cpmm"}"#,
        )
        .unwrap();
        handle_message(bot.clone(), message, state.clone())
            .await
            .unwrap();
        let callback_message: serde_json::Value = serde_json::from_str(
            r#"{"message_id":1,"date":0,"chat":{"id":1,"type":"private","first_name":"Test"},"text":"ok"}"#,
        )
        .unwrap();
        assert_eq!(
            state
                .panels
                .lock()
                .await
                .get(&1)
                .and_then(|panel| panel.quick.execution.wallet_id),
            Some(wallet_id)
        );
        for (id, data) in [
            ("callback-side", "panel:side:sell"),
            ("callback-sim", "panel:simulate"),
        ] {
            let callback: CallbackQuery = serde_json::from_value(serde_json::json!({
                "id": id,
                "from": {"id": 1, "is_bot": false, "first_name": "Test"},
                "chat_instance": "isolated-test",
                "message": callback_message,
                "data": data,
            }))
            .unwrap();
            handle_callback(bot.clone(), callback, state.clone())
                .await
                .unwrap();
        }
        let buy_mint = std::env::var("BOT_SDK11_TEST_BUY_MINT")
            .unwrap_or_else(|_| "3ZLekZYq2qkZiSpnSvabjit34tUkjSwD1JFuW9as9wBG".into());
        let buy_pool = std::env::var("BOT_SDK11_TEST_BUY_POOL_ID")
            .unwrap_or_else(|_| "gyYigqG8vdEMkDNTTVL6at2msbHdTDWyb6bccyR238U".into());
        let buy_protocol =
            std::env::var("BOT_SDK11_TEST_BUY_PROTOCOL").unwrap_or_else(|_| "raydium-cpmm".into());
        let buy_quote_mint = std::env::var("BOT_SDK11_TEST_BUY_EXPECTED_QUOTE_MINT")
            .unwrap_or_else(|_| crate::trade_client::USDC_MINT.into());
        assert!(matches!(buy_protocol.as_str(), "raydium-cpmm" | "pump-amm"));
        let bridge_message: Message = serde_json::from_value(serde_json::json!({
            "message_id": 11,
            "date": 0,
            "chat": {"id": 1, "type": "private", "first_name": "Test"},
            "from": {"id": 1, "is_bot": false, "first_name": "Test"},
            "text": format!("/pool {buy_mint} {buy_pool} {buy_protocol}"),
        }))
        .unwrap();
        handle_message(bot.clone(), bridge_message, state.clone())
            .await
            .unwrap();
        let resolved_buy_route = state
            .panels
            .lock()
            .await
            .get(&1)
            .expect("buy panel must exist")
            .route
            .clone();
        assert_eq!(resolved_buy_route.mint, buy_mint);
        assert_eq!(
            resolved_buy_route.pool_id.as_deref(),
            Some(buy_pool.as_str())
        );
        assert_eq!(resolved_buy_route.pool, buy_protocol);
        assert_eq!(resolved_buy_route.quote_mint, buy_quote_mint);
        let bridge_callback: CallbackQuery = serde_json::from_value(serde_json::json!({
            "id": "callback-bridge-sim",
            "from": {"id": 1, "is_bot": false, "first_name": "Test"},
            "chat_instance": "isolated-test",
            "message": callback_message,
            "data": "panel:simulate",
        }))
        .unwrap();
        handle_callback(bot.clone(), bridge_callback, state.clone())
            .await
            .unwrap();
        telegram_server.await.unwrap();
        crate::order::OrderReconciler::new(db.clone(), client, 1, 1).start();
        let pool = sqlx::SqlitePool::connect(&db_url).await.unwrap();
        let mut settled = None;
        for _ in 0..120 {
            let rows: Vec<(String, String, String, Option<String>, Option<String>, String, Option<String>)> = sqlx::query_as(
                "SELECT status, trade_service_origin, trade_service_request_json, trade_service_request_id, tx_signature, side, error FROM orders ORDER BY side",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(rows.len(), 2);
            if rows.iter().all(|row| row.0 == "simulated") {
                settled = Some(rows);
                break;
            }
            assert!(
                rows.iter().all(|row| matches!(
                    row.0.as_str(),
                    "simulated" | "accepted" | "warming_params" | "building_tx"
                )),
                "{:?}",
                rows.iter()
                    .map(|row| (&row.5, &row.0, &row.6))
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let rows = settled.expect("isolated SDK order did not reconcile to simulated");
        assert_eq!(rows[0].5, "buy");
        assert_eq!(rows[1].5, "sell");
        for (_, origin, request_json, request_id, signature, _, _) in rows {
            assert_eq!(origin, "isolated_sdk_simulation");
            assert!(request_json.contains("\"sdk_decision_id\""));
            assert!(request_id.is_some());
            assert!(signature.is_none());
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
        pool.close().await;
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    #[ignore = "requires a funded disposable SDK 11 candidate and local HTTP access"]
    async fn telegram_cpmm_usdt_sell_reconciles_against_real_sdk_candidate() {
        let isolated_url = std::env::var("BOT_SDK11_TEST_URL").unwrap();
        let isolated_token = std::env::var("BOT_SDK11_TEST_TOKEN").unwrap();
        let service_wallet_id = std::env::var("BOT_SDK11_TEST_WALLET_ID").unwrap();
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let telegram = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let telegram_url = format!("http://{}/", telegram.local_addr().unwrap());
        let simulation_button = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_button = simulation_button.clone();
        let telegram_server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = telegram.accept().await.unwrap();
                let mut buffer = [0; 16384];
                let count = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..count]);
                if wire.contains("模拟本笔") {
                    saw_button.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if wire.contains("交易未提交") || wire.contains("下单失败") {
                    eprintln!(
                        "Telegram mock trade failure: {}",
                        wire.split("\r\n\r\n").last().unwrap_or("")
                    );
                }
                let body = if wire.contains("AnswerCallbackQuery") {
                    r#"{"ok":true,"result":true}"#
                } else {
                    r#"{"ok":true,"result":{"message_id":1,"date":0,"chat":{"id":1,"type":"private","first_name":"Test"},"text":"ok"}}"#
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let primary_url = format!("http://{}", primary.local_addr().unwrap());
        let client = TradeServiceClient::new(primary_url.clone(), None, 90)
            .unwrap()
            .with_route_decision_endpoint(Some(isolated_url.clone()), Some(isolated_token))
            .unwrap();
        let ready = client.isolated_sdk_ready().await.unwrap();
        let wallet_pubkey = ready
            .wallets
            .iter()
            .find(|wallet| wallet.wallet_id == service_wallet_id)
            .unwrap()
            .pubkey
            .clone();
        let path =
            std::env::temp_dir().join(format!("bot-real-sdk11-sell-{}.sqlite", Uuid::new_v4()));
        let db_url = format!("sqlite://{}", path.display());
        let db = Database::connect(&db_url).await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(1, None).await.unwrap();
        db.insert_external_wallet(user.id, &service_wallet_id, "Isolated", &wallet_pubkey)
            .await
            .unwrap();
        let mut config = test_config();
        config.trade_service_url = primary_url;
        config.route_decision_url = Some(isolated_url);
        config.route_decision_token = Some(std::env::var("BOT_SDK11_TEST_TOKEN").unwrap());
        config.trade_service_timeout_secs = 90;
        config.trade_submission_enabled = false;
        config.isolated_sdk_simulation_enabled = true;
        let secret_box = crate::crypto::SecretBox::from_hex(&config.master_key_hex).unwrap();
        let wallet_service = WalletService::new(db.clone(), secret_box, client.clone());
        let oracle = PriceOracle::new(None, "".into(), None, 0);
        let risk = crate::risk::RiskEngine::new();
        let executor =
            crate::executor::ExecutionRouter::new(db.clone(), risk.clone(), client.clone(), 1, 1);
        let strategy = StrategyEngine::new(config.clone(), db.clone(), risk, executor, None);
        let routes = PoolRouteCache::new(db.clone(), client.clone(), 10);
        let state = Arc::new(BotState::new(
            config,
            db.clone(),
            wallet_service,
            client.clone(),
            oracle,
            strategy,
            routes,
        ));
        let bot = Bot::new("123:ABC")
            .set_api_url(reqwest::Url::parse(&telegram_url).unwrap())
            .parse_mode(ParseMode::Html);
        let message: Message = serde_json::from_value(serde_json::json!({
            "message_id":10,"date":0,
            "chat":{"id":1,"type":"private","first_name":"Test"},
            "from":{"id":1,"is_bot":false,"first_name":"Test"},
            "text":"/pool 3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh bsitkqbNk5dPgyrVpHRjiAPNQk66xCfNs8wE981Cd8e raydium-cpmm"
        })).unwrap();
        handle_message(bot.clone(), message, state.clone())
            .await
            .unwrap();
        let callback_message = serde_json::json!({"message_id":1,"date":0,"chat":{"id":1,"type":"private","first_name":"Test"},"text":"ok"});
        let sell_callback: CallbackQuery = serde_json::from_value(serde_json::json!({
            "id":"sell-side","from":{"id":1,"is_bot":false,"first_name":"Test"},
            "chat_instance":"isolated-test","message":callback_message,"data":"panel:side:sell"
        }))
        .unwrap();
        handle_callback(bot.clone(), sell_callback, state.clone())
            .await
            .unwrap();
        let mut hop_ready = false;
        for _ in 0..120 {
            if state.panels.lock().await.get(&1).is_some_and(|panel| {
                panel
                    .sol_hop
                    .as_ref()
                    .is_some_and(|hop| hop.verified_pool_id().is_ok())
            }) {
                hop_ready = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert!(
            hop_ready,
            "verified CPMM WSOL/USDT bridge did not reach the panel"
        );
        assert!(simulation_button.load(std::sync::atomic::Ordering::SeqCst));
        let simulate_callback: CallbackQuery = serde_json::from_value(serde_json::json!({
            "id":"sell-simulate","from":{"id":1,"is_bot":false,"first_name":"Test"},
            "chat_instance":"isolated-test","message":callback_message,"data":"panel:simulate"
        }))
        .unwrap();
        handle_callback(bot, simulate_callback, state.clone())
            .await
            .unwrap();
        crate::order::OrderReconciler::new(db.clone(), client, 1, 1).start();
        let pool = sqlx::SqlitePool::connect(&db_url).await.unwrap();
        let mut settled = None;
        let mut last_rows = Vec::new();
        for _ in 0..120 {
            let mut rows: Vec<(String, String, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT status, trade_service_origin, trade_service_request_json, trade_service_request_id, tx_signature, error FROM orders"
            ).fetch_all(&pool).await.unwrap();
            if rows.len() == 1 && rows[0].0 == "simulated" {
                settled = Some(rows.remove(0));
                break;
            }
            last_rows = rows.into_iter().map(|row| (row.0, row.5)).collect();
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        let (_, origin, request_json, request_id, signature, _) = settled.unwrap_or_else(|| {
            panic!("two-hop sell did not reconcile to simulated: {last_rows:?}")
        });
        assert_eq!(origin, "isolated_sdk_simulation");
        assert!(request_json.contains("\"sdk_decision_id\""));
        assert!(request_json.contains("\"sol_hop_pool_id\""));
        assert!(request_id.is_some() && signature.is_none());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
        telegram_server.abort();
        pool.close().await;
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn panel_prefers_candidate_verified_cached_pool_over_mint_lookup() {
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated_url = format!("http://{}", isolated.local_addr().unwrap());
        let mint = "3ZLekZYq2qkZiSpnSvabjit34tUkjSwD1JFuW9as9wBG";
        let pool_id = "gyYigqG8vdEMkDNTTVL6at2msbHdTDWyb6bccyR238U";
        let server = tokio::spawn(async move {
            let responses = [(
                "POST /v1/pools/resolve-candidates ",
                200,
                r#"{"chain_verified":true,"protocol_id":"raydium-cpmm","pool_id":"gyYigqG8vdEMkDNTTVL6at2msbHdTDWyb6bccyR238U","mint":"3ZLekZYq2qkZiSpnSvabjit34tUkjSwD1JFuW9as9wBG","quote_mint":"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"}"#,
            )];
            for (expected, status, body) in responses {
                let (mut socket, _) = isolated.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..count]);
                assert!(wire.starts_with(expected), "{wire}");
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        db.upsert_pool_route(
            mint,
            "raydium-cpmm",
            Some(pool_id),
            crate::trade_client::USDC_MINT,
            "pumpapi",
        )
        .await
        .unwrap();
        let mut config = test_config();
        config.trade_service_url = format!("http://{}", primary.local_addr().unwrap());
        config.route_decision_url = Some(isolated_url);
        config.route_decision_token = Some("isolated-token".into());
        let client = TradeServiceClient::new(config.trade_service_url.clone(), None, 2)
            .unwrap()
            .with_route_decision_endpoint(
                config.route_decision_url.clone(),
                config.route_decision_token.clone(),
            )
            .unwrap();
        let secret_box = crate::crypto::SecretBox::from_hex(&config.master_key_hex).unwrap();
        let wallet_service = WalletService::new(db.clone(), secret_box, client.clone());
        let oracle = PriceOracle::new(None, "".into(), None, 0);
        let risk = crate::risk::RiskEngine::new();
        let executor =
            crate::executor::ExecutionRouter::new(db.clone(), risk.clone(), client.clone(), 1, 1);
        let strategy = StrategyEngine::new(config.clone(), db.clone(), risk, executor, None);
        let routes = PoolRouteCache::new(db.clone(), client.clone(), 1);
        let state = BotState::new(config, db, wallet_service, client, oracle, strategy, routes);
        let route = resolve_pool_route(&state, mint, false).await.unwrap();
        assert_eq!(route.pool, "raydium-cpmm");
        assert_eq!(route.pool_id.as_deref(), Some(pool_id));
        server.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn explicit_pool_hint_opens_simulation_panel_without_primary_request() {
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let telegram = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated_url = format!("http://{}", isolated.local_addr().unwrap());
        let telegram_url = format!("http://{}/", telegram.local_addr().unwrap());
        let isolated_server = tokio::spawn(async move {
            let (mut socket, _) = isolated.accept().await.unwrap();
            let mut buffer = [0; 8192];
            let count = socket.read(&mut buffer).await.unwrap();
            let wire = String::from_utf8_lossy(&buffer[..count]);
            assert!(wire.starts_with("POST /v1/pools/resolve-candidates "));
            assert!(wire.contains("\"quote_mint\":null"));
            let body = r#"{"chain_verified":true,"protocol_id":"pump-amm","pool_id":"G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b","mint":"nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump","quote_mint":"TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo"}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let telegram_server = tokio::spawn(async move {
            for (index, expected) in ["SendMessage", "EditMessageText", "EditMessageText"]
                .into_iter()
                .enumerate()
            {
                let (mut socket, _) = telegram.accept().await.unwrap();
                let mut buffer = [0; 16384];
                let count = socket.read(&mut buffer).await.unwrap();
                let wire = String::from_utf8_lossy(&buffer[..count]);
                assert!(wire.contains(expected), "{wire}");
                if index == 2 {
                    assert!(wire.contains("模拟本笔"), "{wire}");
                }
                let body = r#"{"ok":true,"result":{"message_id":1,"date":0,"chat":{"id":1,"type":"private","first_name":"Test"},"text":"ok"}}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(1, None).await.unwrap();
        db.insert_external_wallet(user.id, "w1", "Mock", "test-pubkey")
            .await
            .unwrap();
        let mut config = test_config();
        config.trade_service_url = format!("http://{}", primary.local_addr().unwrap());
        config.route_decision_url = Some(isolated_url);
        config.route_decision_token = Some("isolated-token".into());
        let client = TradeServiceClient::new(config.trade_service_url.clone(), None, 2)
            .unwrap()
            .with_route_decision_endpoint(
                config.route_decision_url.clone(),
                config.route_decision_token.clone(),
            )
            .unwrap();
        let secret_box = crate::crypto::SecretBox::from_hex(&config.master_key_hex).unwrap();
        let wallet_service = WalletService::new(db.clone(), secret_box, client.clone());
        let oracle = PriceOracle::new(None, "".into(), None, 0);
        let risk = crate::risk::RiskEngine::new();
        let executor =
            crate::executor::ExecutionRouter::new(db.clone(), risk.clone(), client.clone(), 1, 1);
        let strategy = StrategyEngine::new(config.clone(), db.clone(), risk, executor, None);
        let routes = PoolRouteCache::new(db.clone(), client.clone(), 1);
        let state = BotState::new(config, db, wallet_service, client, oracle, strategy, routes);
        let bot = Bot::new("123:ABC")
            .set_api_url(reqwest::Url::parse(&telegram_url).unwrap())
            .parse_mode(ParseMode::Html);
        handle_explicit_pool_panel(
            &bot,
            ChatId(1),
            &state,
            user.id,
            1,
            "nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b pump-amm",
        ).await.unwrap();
        isolated_server.await.unwrap();
        telegram_server.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
    }

    fn test_config() -> AppConfig {
        AppConfig {
            telegram_bot_token: "test".into(),
            allowed_telegram_user_ids: HashSet::from([1]),
            database_url: "sqlite::memory:".into(),
            master_key_hex: "00".repeat(32),
            pumpapi_zmq_endpoint: "tcp://127.0.0.1:8939".into(),
            enable_stream_consumer: false,
            enable_telegram_bot: false,
            web_api_base_url: None,
            web_api_token: None,
            pnl_position_wallets: Vec::new(),
            trade_service_url: "http://127.0.0.1:1".into(),
            trade_service_token: None,
            route_decision_url: None,
            route_decision_token: None,
            trade_service_timeout_secs: 2,
            pool_resolve_timeout_secs: 2,
            trade_submission_enabled: false,
            isolated_sdk_simulation_enabled: true,
            trade_submit_max_attempts: 1,
            trade_submit_retry_delay_ms: 1,
            trade_reconcile_interval_secs: 1,
            fast_follow_enabled: false,
            fast_follow_max_inflight: 1,
            fast_follow_cache_refresh_ms: 1000,
            fast_follow_limit_queue_capacity: 1,
            migrate_local_wallets_to_trade_service: false,
            dry_run_default: true,
            default_buy_amount_sol: 0.001,
            quick_buy_presets_sol: [0.001; 5],
            quick_sell_presets_percent: [10.0; 5],
            default_slippage_bps: 100,
            default_tip_sol: 0.0001,
            use_mev: false,
            default_mev_provider: "auto".into(),
            sol_usd_price: None,
            sol_usd_price_api_url: "".into(),
            sol_usd_price_api_key: None,
            sol_usd_refresh_secs: 0,
            alpha_retention_enabled: false,
            alpha_retention_interval_secs: 900,
            alpha_observation_retention_hours: 72,
            alpha_paper_signal_retention_days: 7,
            alpha_observation_max_rows: 50_000,
            alpha_paper_signal_max_rows: 20_000,
            alpha_retention_batch_size: 1000,
        }
    }
}
