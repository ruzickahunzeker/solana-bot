//! Read-only WebAPI opening-day reports. Never imports or reconstructs positions.
use anyhow::{ensure, Result};
use chrono::{FixedOffset, NaiveDate, Utc};
use reqwest::{Client, Url};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::time::Duration;

pub const PAGE_SIZE: u64 = 5;
#[derive(Clone)]
pub struct WalletPnlClient {
    endpoint: Url,
    token: String,
    client: Client,
}
#[derive(Debug, Deserialize)]
pub struct Report {
    pub source: String,
    pub trader: String,
    pub timezone: String,
    pub buy_date: Option<String>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
    pub pages: u64,
    pub summary: Summary,
    pub data: Vec<MintRow>,
    #[serde(default)]
    pub cached: bool,
}
#[derive(Debug, Deserialize)]
pub struct Summary {
    pub total_mints: Option<u64>,
    pub profit_mints: Option<u64>,
    pub loss_mints: Option<u64>,
    pub flat_or_open_mints: Option<u64>,
    pub closed_rounds: Option<u64>,
    pub open_rounds: Option<u64>,
    pub net_pnl_sol: Option<Value>,
}
#[derive(Debug, Deserialize)]
pub struct MintRow {
    pub source: String,
    pub mint: String,
    pub pnl_sol: Option<Value>,
    pub closed_rounds: Option<u64>,
    pub open_rounds: Option<u64>,
    pub rounds: Option<u64>,
    pub avg_pnl_pct: Option<Value>,
    #[serde(default, deserialize_with = "round_details")]
    pub round_details: Option<Vec<Value>>,
}
fn round_details<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Vec<Value>>, D::Error> {
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(Value::Array(a)) => Ok(Some(a)),
        Some(Value::String(s)) => {
            serde_json::from_str::<Option<Vec<Value>>>(&s).map_err(serde::de::Error::custom)
        }
        _ => Err(serde::de::Error::custom("invalid round details")),
    }
}
pub fn day(offset: i64) -> NaiveDate {
    Utc::now()
        .with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
        .date_naive()
        + chrono::Duration::days(offset)
}
pub fn date(text: &str) -> Result<NaiveDate> {
    let parsed = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")?;
    ensure!(
        parsed.format("%Y-%m-%d").to_string() == text.trim(),
        "date must be YYYY-MM-DD"
    );
    Ok(parsed)
}
pub fn number(value: &Option<Value>) -> String {
    match value {
        Some(Value::Number(n)) if n.as_f64().is_some_and(f64::is_finite) => n.to_string(),
        Some(Value::String(s)) if s.parse::<f64>().is_ok_and(f64::is_finite) => s.clone(),
        _ => "未知".into(),
    }
}
impl WalletPnlClient {
    pub fn new(base: &str, token: String) -> Result<Self> {
        let mut url = Url::parse(base)?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && !token.trim().is_empty(),
            "invalid WebAPI configuration"
        );
        url.set_path(&format!(
            "{}/api/buy_day_age_pnl",
            url.path().trim_end_matches('/')
        ));
        Ok(Self {
            endpoint: url,
            token,
            client: Client::builder()
                .timeout(Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    pub async fn fetch(&self, trader: &str, date: NaiveDate, page: u64) -> Result<Report> {
        ensure!(
            page > 0 && bs58::decode(trader).into_vec()?.len() == 32,
            "invalid report query"
        );
        let day = date.format("%Y-%m-%d").to_string();
        let response = self
            .client
            .get(self.endpoint.clone())
            .header("X-API-Key", &self.token)
            .query(&[
                ("trader", trader),
                ("source", "sol"),
                ("buy_date", day.as_str()),
                ("timezone", "utc8"),
                ("page", &page.to_string()),
                ("page_size", &PAGE_SIZE.to_string()),
            ])
            .send()
            .await?;
        // Do not include backend response bodies or authenticated URLs in errors.
        ensure!(
            response.status().is_success(),
            "WebAPI report unavailable (HTTP {})",
            response.status().as_u16()
        );
        let report: Report = response.json().await?;
        ensure!(
            report.source == "sol"
                && report.trader == trader
                && report.timezone == "utc8"
                && report.buy_date.as_deref() == Some(day.as_str())
                && report.page == page
                && report.page_size == PAGE_SIZE
                && report.data.len() <= PAGE_SIZE as usize
                && report.data.iter().all(|r| r.source == "sol")
                && report.summary.total_mints.is_none_or(|n| n == report.total)
                && (report.total > 0 || report.data.is_empty()),
            "report response mismatch"
        );
        Ok(report)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_numbers_and_jsonb_text_are_preserved() {
        let raw = serde_json::json!({"source":"sol","mint":"mint","pnl_sol":null,"avg_pnl_pct":"0.125","round_details":"[{\"round\":2,\"pnl_sol\":null}]"});
        let row: MintRow = serde_json::from_value(raw).unwrap();
        assert_eq!(number(&row.pnl_sol), "未知");
        assert_eq!(number(&row.avg_pnl_pct), "0.125");
        assert_eq!(row.round_details.unwrap()[0]["pnl_sol"], Value::Null);
        assert_eq!(number(&Some(serde_json::json!(0))), "0");
        assert_eq!(number(&Some(serde_json::json!("NaN"))), "未知");
    }
    #[test]
    fn exact_dates_and_config_are_checked() {
        assert!(date("2026-10-08").is_ok());
        assert!(date("2026-2-1").is_err());
        assert!(date("2026-02-30").is_err());
        assert!(WalletPnlClient::new("http://user:secret@example.com", "token".into()).is_err());
        assert!(WalletPnlClient::new("http://example.com?api_key=secret", "token".into()).is_err());
    }
    #[tokio::test]
    async fn request_uses_header_auth_and_closed_round_summary_without_mutation() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let trader = "So11111111111111111111111111111111111111112";
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut b = [0; 8192];
            let n = socket.read(&mut b).await.unwrap();
            let text = String::from_utf8_lossy(&b[..n]);
            assert!(text.starts_with("GET /api/buy_day_age_pnl?"));
            assert!(text.contains("source=sol"));
            assert!(text.contains("timezone=utc8"));
            assert!(text.contains("buy_date=2026-10-08"));
            assert!(text.contains("page_size=5"));
            assert!(text.to_ascii_lowercase().contains("x-api-key: test-key"));
            assert!(!text.lines().next().unwrap().contains("test-key"));
            // A row opened on the requested day may close later. The API owns filtering and PnL; the Bot does not recompute it.
            let body=serde_json::json!({"source":"sol","trader":trader,"timezone":"utc8","buy_date":"2026-10-08","total":1,"page":1,"page_size":5,"pages":1,"summary":{"total_mints":1,"profit_mints":1,"closed_rounds":1,"open_rounds":0,"net_pnl_sol":"0.003"},"data":[{"source":"sol","mint":"mint","pnl_sol":"0.003","round_details":[{"open_ts":1791388800000u64,"close_ts":1791475200000u64,"buy_count":2}]}]}).to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let client = WalletPnlClient::new(&format!("http://{address}"), "test-key".into()).unwrap();
        let r = client
            .fetch(trader, date("2026-10-08").unwrap(), 1)
            .await
            .unwrap();
        assert_eq!(number(&r.summary.net_pnl_sol), "0.003");
        assert_eq!(r.data[0].round_details.as_ref().unwrap()[0]["buy_count"], 2);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn authentication_failure_does_not_expose_body_or_become_zero_pnl() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut b = [0; 4096];
            socket.read(&mut b).await.unwrap();
            let body = "private-backend-details";
            socket.write_all(format!("HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        });
        let client =
            WalletPnlClient::new(&format!("http://{address}"), "private-api-token".into()).unwrap();
        let error = client
            .fetch(
                "So11111111111111111111111111111111111111112",
                date("2026-10-08").unwrap(),
                1,
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("401"));
        assert!(!error.contains("private-backend-details"));
        assert!(!error.contains("private-api-token"));
        server.await.unwrap();
    }
}
