//! Wallet reports are read-only and always recheck wallet mapping/ownership.
use super::*;
use crate::wallet_pnl::{date, day, number, Report};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Target {
    Managed(i64),
    Position(String),
}
impl Target {
    fn key(&self) -> String {
        match self {
            Self::Managed(id) => format!("pnl:{id}"),
            Self::Position(tag) => format!("ppnl:{tag}"),
        }
    }
}
fn position_wallet<'a>(wallets: &'a [String], tag: &str) -> Result<&'a str> {
    let matches: Vec<_> = wallets.iter().filter(|w| w.get(..8) == Some(tag)).collect();
    anyhow::ensure!(
        matches.len() == 1,
        "position wallet is not configured or ambiguous"
    );
    Ok(matches[0])
}

pub(super) async fn select_positions(
    bot: &HtmlBot,
    message: &MaybeInaccessibleMessage,
    state: &BotState,
) -> Result<()> {
    let mut text = String::from(
        "仓位钱包盈亏 · 只读查询\n选择仓位订阅地址，查询 SOL 开仓日闭合收益（北京时间）。\n",
    );
    let mut rows = Vec::new();
    for wallet in &state.config.pnl_position_wallets {
        let tag = &wallet[..8];
        text.push_str(&format!(
            "\n{}: <code>{}</code>\n",
            &wallet[..3],
            escape_html(wallet)
        ));
        rows.push(vec![button(
            &format!("{} · 开仓日盈亏", &wallet[..3]),
            &Target::Position(tag.into()),
            "today",
            1,
        )]);
    }
    if rows.is_empty() {
        text.push_str("\n尚未配置仓位钱包地址。");
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "返回主页",
        "menu:main",
    )]);
    bot.edit_message_text(message.chat().id, message.id(), text)
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;
    Ok(())
}

fn callback(data: &str) -> Result<(Target, String, u64)> {
    let fields: Vec<_> = data.split(':').collect();
    anyhow::ensure!(fields.len() == 4, "invalid callback");
    let target = match fields[0] {
        "pnl" => {
            let id = fields[1].parse::<i64>()?;
            anyhow::ensure!(id > 0, "invalid wallet ID");
            Target::Managed(id)
        }
        "ppnl" => {
            anyhow::ensure!(
                fields[1].len() == 8 && bs58::decode(fields[1]).into_vec().is_ok(),
                "invalid position wallet tag"
            );
            Target::Position(fields[1].into())
        }
        _ => anyhow::bail!("invalid callback"),
    };
    let page = fields[3].parse::<u64>()?;
    anyhow::ensure!((1..=100_000).contains(&page), "invalid report page");
    let selected = match fields[2] {
        "today" => day(0).to_string(),
        "yesterday" => day(-1).to_string(),
        "date" => "date".into(),
        v => date(v)?.to_string(),
    };
    Ok((target, selected, page))
}
fn button(label: &str, target: &Target, selected: &str, page: u64) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(label, format!("{}:{selected}:{page}", target.key()))
}
fn keyboard(target: &Target, selected: &str, page: u64, pages: u64) -> InlineKeyboardMarkup {
    let mut rows = vec![vec![
        button("今天", target, "today", 1),
        button("昨天", target, "yesterday", 1),
        button("指定日期", target, "date", 1),
    ]];
    let mut paging = Vec::new();
    if page > 1 {
        paging.push(button("上一页", target, selected, page - 1));
    }
    if page < pages {
        paging.push(button("下一页", target, selected, page + 1));
    }
    if !paging.is_empty() {
        rows.push(paging);
    }
    let (label, callback) = match target {
        Target::Managed(id) => ("返回钱包", format!("wallet:detail:{id}")),
        Target::Position(_) => ("返回仓位钱包", "menu:position_pnl".into()),
    };
    rows.push(vec![InlineKeyboardButton::callback(label, callback)]);
    InlineKeyboardMarkup::new(rows)
}
fn count(n: Option<u64>) -> String {
    n.map(|n| n.to_string()).unwrap_or_else(|| "未知".into())
}
fn render(report: &Report, label: &str) -> String {
    let s = &report.summary;
    let time = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%m-%d %H:%M:%S");
    let mut text=format!("开仓日盈亏 · SOL\n钱包: {}\n地址: <code>{}</code>\n开仓日期: {}（北京时间）\n按开仓日期筛选，仅统计已闭合收益。\n闭合净收益: {} SOL\n盈利 / 亏损 Mint: {} / {}\n零收益或未闭合 Mint: {}\n闭合 / 未闭合轮次: {} / {}\n命中 Mint: {}\n查询时间: {}{}\n",escape_html(label),escape_html(&report.trader),report.buy_date.as_deref().unwrap_or("未知"),escape_html(&number(&s.net_pnl_sol)),count(s.profit_mints),count(s.loss_mints),count(s.flat_or_open_mints),count(s.closed_rounds),count(s.open_rounds),report.total,time,if report.cached {"（缓存结果）"} else {""});
    if report.total == 0 {
        text.push_str(
            "\n没有命中本次开仓日条件的记录，不代表钱包当日没有交易，也不证明历史已完整核查。\n",
        );
    } else {
        text.push_str(&format!(
            "\n第 {} / {} 页（分页结果可能随实时仓位变化）\n",
            report.page, report.pages
        ));
        if report.data.is_empty() {
            text.push_str("本页暂无记录，请刷新或返回上一页。\n");
        }
        for row in &report.data {
            text.push_str(&format!("\n<code>{}</code>\n闭合收益: {} SOL · 轮次 {}\n闭合 / 未闭合: {} / {}\n闭合收益率均值: {}%\n",escape_html(&row.mint),escape_html(&number(&row.pnl_sol)),count(row.rounds),count(row.closed_rounds),count(row.open_rounds),escape_html(&number(&row.avg_pnl_pct))));
            if let Some(details) = &row.round_details {
                let rounds: Vec<_> = details
                    .iter()
                    .take(5)
                    .filter_map(|d| {
                        d.get("round")
                            .and_then(|v| v.as_u64())
                            .map(|n| n.to_string())
                    })
                    .collect();
                if !rounds.is_empty() {
                    text.push_str(&format!("轮号（前5个）: {}\n", rounds.join(", ")));
                }
            }
        }
    }
    text.push_str("\n昨天开仓、今天加仓的旧轮次不会仅因加仓进入今天结果。后续平仓会更新历史日期收益；不是当日现金流或余额涨跌。不展示 Stonk、不回写仓位与币龄。");
    text
}
async fn show(
    bot: &HtmlBot,
    chat: ChatId,
    message: MessageId,
    state: &BotState,
    user: i64,
    target: Target,
    selected: &str,
    page: u64,
) -> Result<()> {
    let result: Result<(Report, String)> = async {
        let (address, label) = match &target {
            Target::Managed(id) => {
                let local = state.db.get_wallet(user, *id).await?;
                let remote = state.trade_client.managed_wallets().await?;
                let binding = crate::trade_client::conversions::bind_wallet(&local, user, &remote)?;
                (binding.pubkey, local.label)
            }
            Target::Position(tag) => {
                let address = position_wallet(&state.config.pnl_position_wallets, tag)?.to_string();
                let label = format!("仓位钱包 {}", &address[..3]);
                (address, label)
            }
        };
        let client = state.pnl_client.as_ref().context("WebAPI not configured")?;
        let report = client.fetch(&address, date(selected)?, page).await?;
        Ok((report, label))
    }
    .await;
    match result {
        Ok((r, label)) => {
            bot.edit_message_text(chat, message, render(&r, &label))
                .reply_markup(keyboard(&target, selected, r.page, r.pages))
                .await?;
        }
        Err(_) => {
            bot.edit_message_text(chat,message,"开仓日盈亏暂不可用：配置、鉴权、网络或钱包映射核验未通过。请稍后重试；不能将查询失败当作零收益。")
                .reply_markup(keyboard(&target,selected,1,0)).await?;
        }
    }
    Ok(())
}
pub(super) async fn handle(
    bot: &HtmlBot,
    message: &MaybeInaccessibleMessage,
    state: &BotState,
    user: i64,
    telegram: i64,
    data: &str,
) -> Result<()> {
    let Ok((target, selected, page)) = callback(data) else {
        return Ok(());
    };
    // Fetching reports never accepts a callback-supplied address or owner.
    match &target {
        Target::Managed(id) if state.db.get_wallet(user, *id).await.is_err() => return Ok(()),
        Target::Position(tag)
            if position_wallet(&state.config.pnl_position_wallets, tag).is_err() =>
        {
            return Ok(())
        }
        _ => (),
    }
    state.pending.lock().await.remove(&telegram);
    if selected == "date" {
        state.pending.lock().await.insert(
            telegram,
            PendingAction::PnlDate {
                target: target.clone(),
                panel_message_id: message.id(),
                panel_chat_id: message.chat().id,
            },
        );
        bot.edit_message_text(
            message.chat().id,
            message.id(),
            "请输入开仓日期，格式 YYYY-MM-DD，例如 2026-10-08。按北京时间查询 SOL 闭合轮次收益。",
        )
        .reply_markup(keyboard(&target, &day(0).to_string(), 1, 0))
        .await?;
    } else {
        bot.edit_message_text(message.chat().id, message.id(), "正在读取开仓日盈亏...")
            .await?;
        show(
            bot,
            message.chat().id,
            message.id(),
            state,
            user,
            target,
            &selected,
            page,
        )
        .await?;
    }
    Ok(())
}
pub(super) async fn input_date(
    bot: &HtmlBot,
    chat: ChatId,
    state: &BotState,
    user: i64,
    telegram: i64,
    target: Target,
    message: MessageId,
    text: &str,
) -> Result<()> {
    match date(text) {
        Ok(d) => show(bot, chat, message, state, user, target, &d.to_string(), 1).await?,
        Err(_) => {
            state.pending.lock().await.insert(
                telegram,
                PendingAction::PnlDate {
                    target: target.clone(),
                    panel_message_id: message,
                    panel_chat_id: chat,
                },
            );
            bot.edit_message_text(
                chat,
                message,
                "日期无效，请输入 YYYY-MM-DD，例如 2026-10-08。",
            )
            .reply_markup(keyboard(&target, &day(0).to_string(), 1, 0))
            .await?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_rejects_addresses_and_invalid_pages() {
        assert_eq!(
            callback("pnl:9:2026-10-08:2").unwrap(),
            (Target::Managed(9), "2026-10-08".into(), 2)
        );
        for v in [
            "pnl:9:2026-10-08:0",
            "pnl:0:today:1",
            "pnl:9:stonk:1",
            "pnl:9:today:1:wallet_address",
        ] {
            assert!(callback(v).is_err());
        }
    }
    #[test]
    fn empty_results_and_unknown_amounts_do_not_claim_no_trades_or_zero_pnl() {
        let r:Report=serde_json::from_value(serde_json::json!({"source":"sol","trader":"wallet","timezone":"utc8","buy_date":"2026-10-08","total":0,"page":1,"page_size":5,"pages":0,"summary":{},"data":[]})).unwrap();
        let t = render(&r, "<wallet>");
        assert!(t.contains("未知 SOL"));
        assert!(t.contains("不代表钱包当日没有交易"));
        assert!(t.contains("&lt;wallet&gt;"));
        assert!(!t.contains("净收益: 0"));
    }
    #[test]
    fn position_targets_use_only_configured_addresses_and_stable_tags() {
        let a = "5aLY85pyxiuX3fd4RgM3Yc1e3MAL6b7UgaZz6MS3JUfG".to_string();
        let b = "7BNaxx6KdUYrjACNQZ9He26NBFoFxujQMAfNLnArLGH5".to_string();
        assert_eq!(
            position_wallet(&[a.clone(), b.clone()], "5aLY85py").unwrap(),
            a
        );
        assert_eq!(position_wallet(&[b, a.clone()], "5aLY85py").unwrap(), a);
        assert!(position_wallet(&[a.clone()], "Ar2Y6o1Q").is_err());
        assert!(position_wallet(&[a.clone(), a.clone()], "5aLY85py").is_err());
        assert_eq!(
            callback("ppnl:5aLY85py:2026-10-08:2").unwrap(),
            (Target::Position("5aLY85py".into()), "2026-10-08".into(), 2)
        );
        assert!(callback(&format!("ppnl:{a}:today:1")).is_err());
        assert!(callback("pnl:5aLY85py:today:1").is_err());
    }
    #[test]
    fn position_navigation_never_enters_wallet_management_or_trade_callbacks() {
        let target = Target::Position("5aLY85py".into());
        let k = keyboard(&target, "2026-10-08", 2, 3);
        let debug = format!("{k:?}");
        assert!(debug.contains("menu:position_pnl"));
        assert!(!debug.contains("wallet:detail"));
        assert!(!debug.contains("trade:"));
        assert!(debug.contains("ppnl:5aLY85py:2026-10-08:3"));
        assert!(target.key().len() + ":2026-10-08:100000".len() < 64);
    }
}
