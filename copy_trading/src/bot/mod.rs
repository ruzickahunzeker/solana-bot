pub mod keyboards;
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
use std::{collections::HashMap, sync::Arc};
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
    pending: Arc<Mutex<HashMap<i64, PendingAction>>>,
    panels: Arc<Mutex<HashMap<i64, TradePanelSession>>>,
    transfers: Arc<Mutex<HashMap<i64, TransferDraft>>>,
}

#[derive(Debug, Clone)]
enum PendingAction {
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
        Self {
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

    if let Some(action) = state.pending.lock().await.remove(&telegram_id) {
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
        bot.edit_message_text(chat_id, message.id(), wallet_detail_text(&wallet, balance))
            .reply_markup(wallet_detail_keyboard(&wallet))
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
    state
        .wallet_service
        .sync_trade_service_wallets(user_id)
        .await?;
    let wallets = state.db.list_wallets(user_id).await?;
    let balances = state.wallet_service.balances(user_id).await.ok();
    let mut lines = vec![format!("钱包列表：{} 个", wallets.len())];
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
    if let Some(message_id) = message_id {
        bot.edit_message_text(chat_id, message_id, text)
            .reply_markup(wallet_list_keyboard(&wallets))
            .await?;
    } else {
        bot.send_message(chat_id, text)
            .reply_markup(wallet_list_keyboard(&wallets))
            .await?;
    }
    Ok(())
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
            bot.edit_message_text(chat_id, resolving.id, pool_resolution_error_text(&err))
                .reply_markup(back_to_main_keyboard())
                .await?;
            return Ok(());
        }
    };
    state
        .wallet_service
        .sync_trade_service_wallets(user_id)
        .await?;
    let wallet = state.db.active_wallet(user_id).await?;
    let settings = load_user_trade_settings(state, user_id).await?;
    let mut session = TradePanelSession::new(
        mint.to_string(),
        wallet.as_ref().map(|wallet| wallet.id),
        &settings,
        tab,
        route,
    );
    let wallet_label =
        panel_wallet_label(state, user_id, session.active_execution().wallet_id).await;
    bot.edit_message_text(
        chat_id,
        resolving.id,
        session.text(&wallet_label, settings.use_mev),
    )
    .reply_markup(session.keyboard(&settings))
    .await?;
    session.message_id = Some(resolving.id);
    state.panels.lock().await.insert(telegram_id, session);
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
    state
        .wallet_service
        .sync_trade_service_wallets(user_id)
        .await?;
    let wallet = state.db.active_wallet(user_id).await?;
    let settings = load_user_trade_settings(state, user_id).await?;
    let mut session = TradePanelSession::new(
        mint.to_string(),
        wallet.as_ref().map(|wallet| wallet.id),
        &settings,
        tab,
        route,
    );
    session.message_id = Some(message_id);
    state.panels.lock().await.insert(telegram_id, session);
    render_trade_panel(bot, chat_id, message_id, state, user_id, telegram_id).await
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
    let result = bot
        .edit_message_text(
            chat_id,
            message_id,
            session.text(&wallet_label, settings.use_mev),
        )
        .reply_markup(session.keyboard(&settings))
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
        "panel:tab:limit" => session.tab = PanelTab::Limit,
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
            state
                .panels
                .lock()
                .await
                .insert(telegram_id, session.clone());
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
            state.panels.lock().await.insert(telegram_id, session);
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
            session.route = resolve_pool_route(state, &session.mint, true).await?;
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
            state.panels.lock().await.insert(telegram_id, session);
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::PanelInput {
                    field,
                    panel_message_id: message_id,
                },
            );
            return Ok(());
        }
        "panel:confirm" => {
            let side = match session.quick.side {
                PanelSide::Buy => TradeSide::Buy,
                PanelSide::Sell => TradeSide::Sell,
            };
            let amount = if session.quick.side == PanelSide::Buy {
                session.quick.buy_amount_sol
            } else {
                session.quick.sell_percent
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
            };
            validate_trade_draft(&draft)?;
            let wallet_label = panel_wallet_label(state, user_id, draft.execution.wallet_id).await;
            state.pending.lock().await.insert(
                telegram_id,
                PendingAction::ConfirmTrade {
                    draft: draft.clone(),
                },
            );
            bot.edit_message_text(
                chat_id,
                message_id,
                format!(
                    "确认交易\n钱包: {}\n动作: {}\nMint: <code>{}</code>\nPool: {}{}\n数量: {} {}\n模式: {}\n滑点: {}%\n小费: {} SOL\nMEV Provider: {}",
                    wallet_label,
                    draft.side.label(),
                    draft.mint,
                    pool_display_name(&draft.route.pool),
                    pool_id_suffix(draft.route.pool_id.as_deref()),
                    trade_panel::trim_float(draft.amount),
                    if matches!(draft.side, TradeSide::Buy) { "SOL" } else { "%" },
                    execution_mode_label(draft.execution.mode),
                    draft.execution.slippage_bps as f64 / 100.0,
                    draft.execution.tip_sol,
                    if settings.use_mev { &draft.execution.mev_mode } else { "全局关闭" },
                ),
            )
            .reply_markup(trade_panel::confirm_keyboard())
            .await?;
            return Ok(());
        }
        "panel:create_limit" => {
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
            let index: usize = data.trim_start_matches("panel:preset:").parse()?;
            let presets = if session.active_side() == PanelSide::Buy {
                settings.buy_presets_sol
            } else {
                settings.sell_presets_percent
            };
            let Some(value) = presets.get(index).copied() else {
                anyhow::bail!("invalid preset index");
            };
            session.set_active_amount(value);
        }
        _ => return Ok(()),
    }

    state.panels.lock().await.insert(telegram_id, session);
    render_trade_panel(bot, chat_id, message_id, state, user_id, telegram_id).await
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
    let mut panels = state.panels.lock().await;
    let Some(session) = panels.get_mut(&telegram_id) else {
        anyhow::bail!("交易面板已过期");
    };
    match field {
        PanelInputField::Amount => {
            if value <= 0.0
                || (session.active_side() == PanelSide::Sell
                    && (value > 100.0 || value.fract() != 0.0))
            {
                anyhow::bail!("买入金额必须大于 0；卖出比例必须是 1 到 100 的整数");
            }
            session.set_active_amount(value);
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
    drop(panels);
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
    let draft = TradeDraft {
        side: side.clone(),
        mint: mint.to_string(),
        amount,
        execution,
        route,
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

async fn execute_pending_trade(
    bot: &HtmlBot,
    chat_id: ChatId,
    message_id: MessageId,
    state: &BotState,
    user_id: i64,
    telegram_id: i64,
) -> Result<()> {
    let Some(PendingAction::ConfirmTrade { draft }) =
        state.pending.lock().await.remove(&telegram_id)
    else {
        bot.edit_message_text(chat_id, message_id, "没有待确认交易")
            .reply_markup(trade_menu_keyboard())
            .await?;
        return Ok(());
    };

    bot.edit_message_text(
        chat_id,
        message_id,
        format!(
            "正在提交交易...\nMint: <code>{}</code>\nPool: {}{}",
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
        let order_id = state
            .db
            .insert_order(
                user_id,
                wallet_id,
                None,
                None,
                &draft.mint,
                side,
                matches!(draft.side, TradeSide::Buy).then_some(draft.amount),
                matches!(draft.side, TradeSide::Sell).then_some(draft.amount),
                "dry_run",
                &idempotency_key,
                None,
                None,
                None,
                None,
            )
            .await?;
        bot.edit_message_text(
            chat_id,
            message_id,
            format!(
                "Dry-run 交易已记录\n订单: <code>{}</code>\n未调用 trade_service。",
                order_id
            ),
        )
        .reply_markup(trade_menu_keyboard())
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
    let ready = match state.trade_client.ready().await {
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

    let (use_mev, mev_provider) = manual_mev_settings(&execution.mev_mode)?;
    let request = TradeRequest {
        wallet_id: Some(trade_service_wallet_id),
        execution_method: execution.method.as_str().to_string(),
        side: side.to_string(),
        pool: draft.route.pool.clone(),
        mint: draft.mint.clone(),
        pool_id: draft.route.pool_id.clone(),
        quote_mint: Some(draft.route.quote_mint.clone()),
        amount_sol: matches!(draft.side, TradeSide::Buy).then_some(draft.amount),
        sell_percent: matches!(draft.side, TradeSide::Sell).then_some(draft.amount.round() as u64),
        slippage_percent: execution.slippage_bps as f64 / 100.0,
        tip_sol: execution.tip_sol,
        use_mev,
        mev_provider,
        simulate: execution.mode == ExecutionMode::Simulate,
        source: Some("copy-trading-manual".to_string()),
        source_signature: None,
        idempotency_key: idempotency_key.clone(),
    };
    let request_json = serde_json::to_string(&request)?;

    match state.trade_client.submit_trade(&request).await {
        Ok(response) => {
            let response_json = serde_json::to_string(&response)?;
            let status = crate::executor::normalize_trade_status(&response);
            let order_id = state
                .db
                .insert_order(
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
                    Some(&request_json),
                    Some(&response_json),
                    response.signature.as_deref(),
                    response.error.as_deref(),
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
                    Some(&request_json),
                    Some(&response_json),
                    None,
                )
                .await?;
            bot.edit_message_text(
                chat_id,
                message_id,
                format!(
                    "订单已进入 trade_service\n钱包 ID: <code>{}</code>\n状态: {}\n订单: <code>{}</code>\n远端请求: <code>{}</code>\n签名: <code>{}</code>",
                    request.wallet_id.as_deref().unwrap_or("-"),
                    status,
                    order_id,
                    response.request_id.as_deref().unwrap_or("-"),
                    response.signature.as_deref().unwrap_or("-")
                ),
            )
            .reply_markup(trade_menu_keyboard())
            .await?;
        }
        Err(err) => {
            let error = err.to_string();
            let order_id = state
                .db
                .insert_order(
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
    state.pool_routes.resolve(mint, force_refresh).await
}

fn pool_resolution_error_text(error: &anyhow::Error) -> &'static str {
    let lower = error.to_string().to_ascii_lowercase();
    if lower.contains("rate limited")
        || lower.contains("rpc unavailable")
        || lower.contains("timeout")
        || lower.contains("429")
    {
        "Pool 解析暂时不可用\nRPC 节点限流或响应异常，已尝试备用节点。请稍后重新发送 Mint。"
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
        "实盘检查\n状态: {live_ready}\n\n默认钱包: {}\n执行钱包绑定: {}\n钱包: {} 个\n跟踪钱包: {} 个，启用 {} 个\n策略: {} 个，Live {} 个\n限价单: pending {} 个，Live {} 个\ntrade_service health: {}\ntrade_service ready: {}\ntrade_service 信息: {}\nreadiness 信息: {}\nPumpAPI 数据流: {}\nPumpAPI 最近接收: {}\nSOL/USD: {}\n默认滑点: {} bps\n默认小费: {} SOL\n\n阻断项\n{}",
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
    state
        .wallet_service
        .balances(user_id)
        .await
        .ok()?
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
    "独立跟单系统\n\n命令：\n/start 主菜单\n/status 状态\n/wallets 钱包\n/watch 跟踪钱包\n/watch_add &lt;钱包&gt; [标签]\n/strategies 策略\n/signals 最近信号\n/positions Dry-run 持仓\n/limits 限价单\n/preflight 实盘检查\n/buy &lt;mint&gt; &lt;SOL&gt;\n/sell &lt;mint&gt; &lt;百分比&gt;\n/orders 最近订单\n/import &lt;私钥&gt; 导入钱包\n/panic 全局暂停\n\n默认 Dry-run；选择 Live 并通过执行检查与确认后提交交易。"
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
        "全局设置\n\n默认模式: {}\n执行方式: {}\n滑点: {}%\n小费: {} SOL\nUSE_MEV: {}\nMEV Provider: {}\n买入快捷金额: {} SOL\n卖出快捷比例: {}%",
        execution_mode_setting_label(&settings.default_execution_mode),
        settings.default_execution_method.to_ascii_uppercase(),
        trim_float(settings.default_slippage_bps as f64 / 100.0),
        trim_float(settings.default_tip_sol),
        if settings.use_mev { "开启" } else { "关闭" },
        settings.default_mev_provider,
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
