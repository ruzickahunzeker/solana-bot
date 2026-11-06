//! Quote and non-broadcast simulation only. No live conversion endpoint.
use super::{ManagedWalletResponse, TradeServiceClient};
use crate::db::models::WalletRecord;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversionWallet {
    pub wallet_id: String,
    pub owner_type: String,
    pub owner_id: String,
    #[serde(skip)]
    pub pubkey: String,
}

pub fn bind_wallet(
    local: &WalletRecord,
    user_id: i64,
    remote: &[ManagedWalletResponse],
) -> Result<ConversionWallet> {
    ensure!(
        local.user_id == user_id && !local.is_deleted,
        "wallet unavailable"
    );
    let id = local
        .external_wallet_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("unmapped wallet"))?;
    let matches: Vec<_> = remote.iter().filter(|w| w.wallet_id == id).collect();
    ensure!(matches.len() == 1, "ambiguous wallet mapping");
    let w = matches[0];
    ensure!(
        w.enabled && w.pubkey == local.public_key,
        "wallet mapping changed"
    );
    ensure!(
        (w.owner_type == "copy_trading" && w.owner_id == user_id.to_string())
            || (w.owner_type == "system" && w.owner_id == "global"),
        "wallet owner mismatch"
    );
    Ok(ConversionWallet {
        wallet_id: w.wallet_id.clone(),
        owner_type: w.owner_type.clone(),
        owner_id: w.owner_id.clone(),
        pubkey: w.pubkey.clone(),
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct WsolQuote {
    pub quote_id: String,
    pub wallet_id: String,
    pub wallet_pubkey: String,
    pub owner_type: String,
    pub owner_id: String,
    pub direction: String,
    pub amount_lamports: u64,
    pub source_balance_lamports: u64,
    pub estimated_network_fee_lamports: u64,
    pub temporary_rent_lamports: u64,
    pub rent_refund_lamports: u64,
    pub expires_at_ms: u64,
}
#[derive(Deserialize)]
struct QuoteResponse {
    quote: WsolQuote,
    executable: bool,
}
#[derive(Debug, Deserialize)]
pub struct WsolSimulation {
    pub status: String,
    pub quote_id: String,
    #[serde(alias = "network_fee_lamports")]
    pub estimated_network_fee_lamports: Option<u64>,
    #[serde(alias = "compute_units", alias = "units_consumed")]
    pub compute_units_consumed: Option<u64>,
}

impl TradeServiceClient {
    pub async fn wsol_quote(
        &self,
        wallet: &ConversionWallet,
        direction: &str,
        amount: Option<u64>,
        all: bool,
    ) -> Result<WsolQuote> {
        ensure!(
            matches!(direction, "wrap" | "unwrap")
                && ((all && direction == "unwrap" && amount.is_none())
                    || (!all && amount.is_some_and(|n| n > 0))),
            "invalid conversion option"
        );
        let mut body = serde_json::to_value(wallet)?;
        body["direction"] = direction.into();
        body["all"] = all.into();
        if let Some(n) = amount {
            body["amount_lamports"] = n.into();
        }
        let response = self
            .authorized(self.client.post(format!(
                "{}/internal/conversions/wsol/quotes",
                self.base_url
            )))
            .timeout(Duration::from_secs(45))
            .json(&body)
            .send()
            .await?;
        let parsed: QuoteResponse = self.decode_response(response, "conversion quote").await?;
        let q = parsed.quote;
        ensure!(
            !q.quote_id.is_empty()
                && q.wallet_id == wallet.wallet_id
                && q.wallet_pubkey == wallet.pubkey
                && q.owner_type == wallet.owner_type
                && q.owner_id == wallet.owner_id
                && q.direction == direction
                && q.amount_lamports > 0
                && amount.is_none_or(|n| q.amount_lamports == n),
            "conversion quote mismatch"
        );
        Ok(q)
    }
    pub async fn wsol_simulate(
        &self,
        wallet: &ConversionWallet,
        quote_id: &str,
    ) -> Result<WsolSimulation> {
        let mut body = serde_json::to_value(wallet)?;
        body["quote_id"] = quote_id.into();
        body["simulate"] = true.into();
        body["idempotency_key"] = format!("wsol-preview-{}", uuid::Uuid::new_v4()).into();
        let response = self
            .authorized(self.client.post(format!(
                "{}/internal/conversions/wsol/simulate",
                self.base_url
            )))
            .timeout(Duration::from_secs(60))
            .json(&body)
            .send()
            .await?;
        let result: WsolSimulation = self
            .decode_response(response, "conversion simulation")
            .await?;
        ensure!(
            result.status == "simulated" && result.quote_id == quote_id,
            "conversion simulation mismatch"
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local() -> WalletRecord {
        WalletRecord {
            id: 9,
            user_id: 1,
            external_wallet_id: Some("2TE".into()),
            label: "test".into(),
            public_key: "pubkey".into(),
            is_active: true,
            is_deleted: false,
        }
    }
    fn remote(owner: &str, id: &str) -> ManagedWalletResponse {
        serde_json::from_value(serde_json::json!({"wallet_id":"2TE","owner_type":owner,"owner_id":id,"label":"test","pubkey":"pubkey","is_default":true,"enabled":true,"created":false})).unwrap()
    }
    #[test]
    fn owner_and_external_id_are_verified() {
        let l = local();
        assert_eq!(
            bind_wallet(&l, 1, &[remote("copy_trading", "1")])
                .unwrap()
                .wallet_id,
            "2TE"
        );
        assert_eq!(
            bind_wallet(&l, 1, &[remote("system", "global")])
                .unwrap()
                .owner_id,
            "global"
        );
        assert!(bind_wallet(&l, 2, &[remote("copy_trading", "1")]).is_err());
        assert!(bind_wallet(&l, 1, &[remote("copy_trading", "2")]).is_err());
        let mut r = remote("system", "global");
        r.pubkey = "changed".into();
        assert!(bind_wallet(&l, 1, &[r]).is_err());
        let mut r = remote("system", "global");
        r.enabled = false;
        assert!(bind_wallet(&l, 1, &[r]).is_err());
        let mut l = local();
        l.external_wallet_id = None;
        assert!(bind_wallet(&l, 1, &[]).is_err());
    }
    #[test]
    fn parses_quote_and_simulation_without_live_status() {
        let q:QuoteResponse=serde_json::from_value(serde_json::json!({"executable":false,"quote":{"quote_id":"q","wallet_id":"2TE","wallet_pubkey":"pubkey","owner_type":"system","owner_id":"global","direction":"wrap","amount_lamports":1000000,"source_balance_lamports":5000000,"estimated_network_fee_lamports":5000,"temporary_rent_lamports":2039280,"rent_refund_lamports":0,"expires_at_ms":1800000000000u64,"executable":false}})).unwrap();
        assert_eq!(q.quote.amount_lamports, 1000000);
        assert!(!q.executable);
        let r: WsolSimulation =
            serde_json::from_str(r#"{"quote_id":"q","status":"simulated","network_fee_lamports":5000,"units_consumed":351}"#)
                .unwrap();
        assert_eq!(r.compute_units_consumed, Some(351));
        assert_eq!(r.estimated_network_fee_lamports, Some(5000));
        assert!(
            serde_json::from_str::<QuoteResponse>(r#"{"quote":{"amount_lamports":0.001}}"#)
                .is_err()
        );
    }
    #[tokio::test]
    async fn quote_and_simulation_only_use_readonly_paths_and_registered_owner() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (index, path) in [
                "/internal/conversions/wsol/quotes",
                "/internal/conversions/wsol/simulate",
            ]
            .iter()
            .enumerate()
            {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = Vec::new();
                let (headers, body) = loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    buffer.extend_from_slice(&chunk[..n]);
                    if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&buffer[..end]).to_string();
                        let length = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if buffer.len() >= end + 4 + length {
                            break (
                                headers,
                                serde_json::from_slice::<serde_json::Value>(
                                    &buffer[end + 4..end + 4 + length],
                                )
                                .unwrap(),
                            );
                        }
                    }
                };
                assert!(headers.starts_with(&format!("POST {path} ")));
                assert_eq!(body["wallet_id"], "2TE");
                assert_eq!(body["owner_type"], "system");
                assert_eq!(body["owner_id"], "global");
                assert!(body.get("pubkey").is_none());
                let response=if index==0 {
                    assert_eq!(body["direction"],"unwrap");assert_eq!(body["all"],true);assert!(body.get("amount_lamports").is_none());
                    serde_json::json!({"executable":false,"quote":{"quote_id":"q","wallet_id":"2TE","wallet_pubkey":"pubkey","owner_type":"system","owner_id":"global","direction":"unwrap","amount_lamports":1000000,"source_balance_lamports":1000000,"estimated_network_fee_lamports":5000,"temporary_rent_lamports":2039280,"rent_refund_lamports":2039280,"expires_at_ms":1800000000000u64,"executable":false}})
                } else {
                    assert_eq!(body["simulate"],true);assert_eq!(body["quote_id"],"q");assert!(body["idempotency_key"].as_str().unwrap().starts_with("wsol-preview-"));
                    serde_json::json!({"status":"simulated","quote_id":"q","estimated_network_fee_lamports":5000,"compute_units":118})
                }.to_string();
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            response.len(),
                            response
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(30), listener.accept())
                    .await
                    .is_err()
            );
        });
        let client = TradeServiceClient::new(format!("http://{address}"), None, 1).unwrap();
        let binding = bind_wallet(&local(), 1, &[remote("system", "global")]).unwrap();
        assert!(client
            .wsol_quote(&binding, "wrap", None, true)
            .await
            .is_err());
        assert!(client
            .wsol_quote(&binding, "unwrap", Some(100), true)
            .await
            .is_err());
        assert!(client
            .wsol_quote(&binding, "wrap", Some(0), false)
            .await
            .is_err());
        let quote = client
            .wsol_quote(&binding, "unwrap", None, true)
            .await
            .unwrap();
        let simulated = client
            .wsol_simulate(&binding, &quote.quote_id)
            .await
            .unwrap();
        assert_eq!(simulated.quote_id, quote.quote_id);
        server.await.unwrap();
    }
}
