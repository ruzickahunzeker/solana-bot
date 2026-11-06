//! Short-lived Telegram sessions for non-broadcast SOL / WSOL previews.
use super::*;
use crate::trade_client::conversions::{bind_wallet, ConversionWallet, WsolQuote};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

#[derive(Clone)]
pub(super) struct Session {
    wallet_id: i64,
    user_id: i64,
    chat_id: ChatId,
    message_id: MessageId,
    binding: ConversionWallet,
    nonce: String,
    expires_at_ms: u64,
    quote: Option<WsolQuote>,
}
fn now() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}
fn nonce() -> String {
    Uuid::new_v4().simple().to_string()[..12].into()
}
fn amount(n: u64) -> String {
    format!("{}.{:09}", n / 1_000_000_000, n % 1_000_000_000)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .into()
}
fn option(action: &str) -> Result<(&'static str, Option<u64>, bool)> {
    match action {
        "w1" => Ok(("wrap", Some(1_000_000), false)),
        "w10" => Ok(("wrap", Some(10_000_000), false)),
        "u1" => Ok(("unwrap", Some(1_000_000), false)),
        "all" => Ok(("unwrap", None, true)),
        _ => anyhow::bail!("invalid conversion callback"),
    }
}
fn valid(
    s: &Session,
    user: i64,
    chat: ChatId,
    message: MessageId,
    wallet: i64,
    token: &str,
    time: u64,
) -> bool {
    s.user_id == user
        && s.chat_id == chat
        && s.message_id == message
        && s.wallet_id == wallet
        && s.nonce == token
        && time < s.expires_at_ms
}
fn button(s: &Session, label: &str, action: &str) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(
        label,
        format!("wsol:{}:{}:{}", s.wallet_id, action, s.nonce),
    )
}
fn selection(s: &Session) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            button(s, "包装 0.001 SOL", "w1"),
            button(s, "包装 0.01 SOL", "w10"),
        ],
        vec![
            button(s, "解包 0.001 WSOL", "u1"),
            button(s, "全部解包", "all"),
        ],
        vec![InlineKeyboardButton::callback(
            "返回钱包",
            format!("wallet:detail:{}", s.wallet_id),
        )],
    ])
}
fn selection_text() -> &'static str {
    "SOL / WSOL 只读预览\n仅报价和非广播模拟，不能提交真实兑换。请选择数量。"
}

/// Probe the quote endpoint for this authorized wallet. Health alone is insufficient.
pub(super) async fn detail_keyboard(
    state: &BotState,
    user: i64,
    telegram: i64,
    chat: ChatId,
    message: MessageId,
    wallet: &crate::db::models::WalletRecord,
) -> (InlineKeyboardMarkup, bool) {
    state.conversions.lock().await.remove(&telegram);
    let mut keyboard = wallet_detail_keyboard(wallet);
    if wallet.external_wallet_id.is_none() {
        return (keyboard, false);
    }
    let probe = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        let binding = bind_wallet(wallet, user, &state.trade_client.managed_wallets().await?)?;
        let q = state
            .trade_client
            .wsol_quote(&binding, "wrap", Some(1_000_000), false)
            .await?;
        anyhow::ensure!(q.expires_at_ms > now(), "expired conversion quote");
        Ok::<_, anyhow::Error>(binding)
    })
    .await;
    if let Ok(Ok(binding)) = probe {
        let s = Session {
            wallet_id: wallet.id,
            user_id: user,
            chat_id: chat,
            message_id: message,
            binding,
            nonce: nonce(),
            expires_at_ms: now() + 300_000,
            quote: None,
        };
        keyboard
            .inline_keyboard
            .insert(0, vec![button(&s, "SOL / WSOL", "open")]);
        state.conversions.lock().await.insert(telegram, s);
        (keyboard, true)
    } else {
        (keyboard, false)
    }
}

pub(super) async fn handle(
    bot: &HtmlBot,
    message: &MaybeInaccessibleMessage,
    state: &BotState,
    user: i64,
    telegram: i64,
    data: &str,
) -> Result<()> {
    let chat = message.chat().id;
    let result:Result<()>=async {
        let mut parts=data.split(':');
        anyhow::ensure!(parts.next()==Some("wsol"),"invalid callback");
        let wallet=parts.next().context("missing wallet")?.parse::<i64>()?;
        let action=parts.next().context("missing action")?;
        let token=parts.next().context("missing session")?;
        anyhow::ensure!(parts.next().is_none(),"invalid callback");
        // Consume before external I/O, preventing duplicate simulation clicks.
        let mut s={
            let mut sessions=state.conversions.lock().await;
            let s=sessions.get(&telegram).context("missing quote")?;
            anyhow::ensure!(valid(s,user,chat,message.id(),wallet,token,now()),"stale quote");
            sessions.remove(&telegram).unwrap()
        };
        let local=state.db.get_wallet(user,wallet).await?;
        let current=bind_wallet(&local,user,&state.trade_client.managed_wallets().await?)?;
        anyhow::ensure!(current==s.binding,"wallet mapping changed");
        s.nonce=nonce();
        if action=="sim" {
            let q=s.quote.take().context("missing quote")?;
            anyhow::ensure!(q.expires_at_ms>now(),"expired quote");
            let r=state.trade_client.wsol_simulate(&s.binding,&q.quote_id).await?;
            let detail=r.compute_units_consumed.map(|n|format!("\nCU: {n}")).unwrap_or_default();
            let fee=r.estimated_network_fee_lamports.map(|n|format!("\n预计网络费: {} SOL",amount(n))).unwrap_or_default();
            s.expires_at_ms=now()+300_000;
            bot.edit_message_text(chat,message.id(),format!("模拟成功，未广播交易。{detail}{fee}\n如需继续预览，请重新选择数量。"))
                .reply_markup(selection(&s)).await?;
        } else if matches!(action,"open"|"back") {
            s.quote=None;s.expires_at_ms=now()+300_000;
            bot.edit_message_text(chat,message.id(),selection_text()).reply_markup(selection(&s)).await?;
        } else {
            let (direction,n,all)=option(action)?;
            let q=state.trade_client.wsol_quote(&s.binding,direction,n,all).await?;
            anyhow::ensure!(q.expires_at_ms>now(),"expired quote");
            let from=if direction=="wrap" {"SOL"} else {"WSOL"};
            let to=if direction=="wrap" {"WSOL"} else {"SOL"};
            let text=format!("SOL / WSOL 只读报价\n方向: {from} → {to}\n源钱包: <code>{}</code>\n数量: {} {from}\n源余额: {} {from}\n预计网络费: {} SOL\n临时账户租金: {} SOL\n账户租金返还: {} SOL（与兑换所得分开）\n报价有效约 {} 秒\n仅预览和非广播模拟，不能提交真实兑换。",escape_html(&s.binding.pubkey),amount(q.amount_lamports),amount(q.source_balance_lamports),amount(q.estimated_network_fee_lamports),amount(q.temporary_rent_lamports),amount(q.rent_refund_lamports),q.expires_at_ms.saturating_sub(now())/1000);
            s.expires_at_ms=q.expires_at_ms;s.quote=Some(q);
            let keyboard=InlineKeyboardMarkup::new(vec![vec![button(&s,"模拟本笔","sim")],vec![button(&s,"返回兑换","back")]]);
            bot.edit_message_text(chat,message.id(),text).reply_markup(keyboard).await?;
        }
        state.conversions.lock().await.insert(telegram,s);
        Ok(())
    }.await;
    if result.is_err() {
        // No backend body, RPC URL, wallet secret, or credentials in UI/logs.
        bot.edit_message_text(chat,message.id(),"兑换预览暂不可用，或报价已过期、面板已失效。交易端需启用报价接口；请返回钱包重新打开。未广播兑换交易。")
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback("返回钱包列表","menu:wallet")]])).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_options_are_fixed_integer_lamports() {
        assert_eq!(option("w1").unwrap(), ("wrap", Some(1_000_000), false));
        assert_eq!(option("w10").unwrap(), ("wrap", Some(10_000_000), false));
        assert_eq!(option("u1").unwrap(), ("unwrap", Some(1_000_000), false));
        assert_eq!(option("all").unwrap(), ("unwrap", None, true));
        for s in ["submit", "wrap:100000000", "0", "-1"] {
            assert!(option(s).is_err());
        }
        assert_eq!(amount(1_000_000), "0.001");
        assert_eq!(amount(0), "0");
    }
    #[test]
    fn stale_foreign_panel_and_wallet_quotes_are_rejected() {
        let s = Session {
            wallet_id: 9,
            user_id: 1,
            chat_id: ChatId(5),
            message_id: MessageId(6),
            binding: ConversionWallet {
                wallet_id: "2TE".into(),
                owner_type: "system".into(),
                owner_id: "global".into(),
                pubkey: "p".into(),
            },
            nonce: "token".into(),
            expires_at_ms: 100,
            quote: None,
        };
        assert!(valid(&s, 1, ChatId(5), MessageId(6), 9, "token", 99));
        assert!(!valid(&s, 1, ChatId(5), MessageId(6), 9, "token", 100));
        assert!(!valid(&s, 2, ChatId(5), MessageId(6), 9, "token", 99));
        assert!(!valid(&s, 1, ChatId(7), MessageId(6), 9, "token", 99));
        assert!(!valid(&s, 1, ChatId(5), MessageId(7), 9, "token", 99));
        assert!(!valid(&s, 1, ChatId(5), MessageId(6), 8, "token", 99));
        assert!(!valid(&s, 1, ChatId(5), MessageId(6), 9, "old", 99));
        assert!(!selection(&s)
            .inline_keyboard
            .iter()
            .flatten()
            .any(|b| format!("{b:?}").contains("submit")));
    }
}
