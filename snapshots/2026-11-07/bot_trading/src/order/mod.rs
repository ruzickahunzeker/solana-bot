#![allow(dead_code)]

use crate::{
    db::Database,
    executor::normalize_trade_status,
    trade_client::{TradeResponse, TradeServiceClient},
};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Created,
    Blocked,
    DryRun,
    Submitted,
    Confirmed,
    Failed,
    Unknown,
    Cancelled,
    Expired,
}

impl OrderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Blocked => "blocked",
            Self::DryRun => "dry_run",
            Self::Submitted => "submitted",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }
}

#[derive(Clone)]
pub struct OrderReconciler {
    db: Database,
    trade_client: TradeServiceClient,
    interval: Duration,
    submit_max_attempts: u32,
}

impl OrderReconciler {
    pub fn new(
        db: Database,
        trade_client: TradeServiceClient,
        interval_secs: u64,
        submit_max_attempts: u32,
    ) -> Self {
        Self {
            db,
            trade_client,
            interval: Duration::from_secs(interval_secs.max(1)),
            submit_max_attempts: submit_max_attempts.max(1),
        }
    }

    pub fn start(self) {
        tokio::spawn(async move {
            loop {
                if let Err(err) = self.reconcile_once().await {
                    warn!(?err, "failed to reconcile trade_service orders");
                }
                tokio::time::sleep(self.interval).await;
            }
        });
    }

    async fn reconcile_once(&self) -> anyhow::Result<()> {
        if self.trade_client.submission_enabled() {
            self.recover_orphaned_submissions().await?;
        }
        let orders = self.db.list_reconcilable_orders(100).await?;
        for (order_id, request_id, local_status, origin) in orders {
            let result = match origin.as_str() {
                "primary" => self.trade_client.get_trade(&request_id).await,
                "isolated_sdk_simulation" => {
                    self.trade_client
                        .get_isolated_sdk_simulation_trade(&request_id)
                        .await
                }
                _ => {
                    warn!(
                        order_id,
                        origin, "unknown trade service origin; order left pending"
                    );
                    continue;
                }
            };
            let response = match result {
                Ok(response) => response,
                Err(err) => {
                    warn!(?err, order_id, request_id, "trade status query failed");
                    continue;
                }
            };
            let remote_status = normalize_trade_status(&response);
            if remote_status == local_status {
                continue;
            }
            self.persist_response(&order_id, &response, remote_status)
                .await?;
            info!(
                order_id,
                request_id, local_status, remote_status, "trade order status reconciled"
            );
        }
        Ok(())
    }

    async fn recover_orphaned_submissions(&self) -> anyhow::Result<()> {
        let orders = self.db.list_orphaned_submissions(20).await?;
        for (order_id, request_json, attempts) in orders {
            if attempts >= i64::from(self.submit_max_attempts) {
                self.db
                    .update_order_from_trade_service(
                        &order_id,
                        "unknown",
                        None,
                        None,
                        None,
                        Some(
                            "submission interrupted without request_id; retry limit reached, inspect trade_service by idempotency key",
                        ),
                    )
                    .await?;
                continue;
            }
            let request = extract_trade_request(&request_json)
                .with_context(|| format!("invalid orphaned order request {order_id}"))?;
            self.db
                .update_order_from_trade_service(&order_id, "submitting", None, None, None, None)
                .await?;
            self.db
                .insert_order_attempt(
                    &order_id,
                    "submitting",
                    Some(&request_json),
                    None,
                    Some("recovering interrupted submission"),
                )
                .await?;
            match self.trade_client.submit_trade(&request).await {
                Ok(response) => {
                    let status = normalize_trade_status(&response);
                    self.persist_response(&order_id, &response, status).await?;
                    info!(order_id, status, "interrupted trade submission recovered");
                }
                Err(err) => {
                    self.db
                        .insert_order_attempt(
                            &order_id,
                            "submit_failed",
                            Some(&request_json),
                            None,
                            Some(&err.to_string()),
                        )
                        .await?;
                    warn!(?err, order_id, "interrupted trade resubmission failed");
                }
            }
        }
        Ok(())
    }

    async fn persist_response(
        &self,
        order_id: &str,
        response: &TradeResponse,
        status: &str,
    ) -> anyhow::Result<()> {
        let response_json = serde_json::to_string(response)?;
        self.db
            .update_order_from_trade_service(
                order_id,
                status,
                response.request_id.as_deref(),
                Some(&response_json),
                response.signature.as_deref(),
                response.error.as_deref(),
            )
            .await?;
        self.db
            .insert_order_attempt(
                order_id,
                status,
                None,
                Some(&response_json),
                response.error.as_deref(),
            )
            .await?;
        Ok(())
    }
}

fn extract_trade_request(request_json: &str) -> anyhow::Result<crate::trade_client::TradeRequest> {
    let value: serde_json::Value = serde_json::from_str(request_json)?;
    let trade_request = value
        .get("trade_request")
        .cloned()
        .context("trade_request is missing")?;
    Ok(serde_json::from_value(trade_request)?)
}

#[cfg(test)]
mod submission_pause_tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[tokio::test]
    async fn isolated_simulation_reconciles_only_from_isolated_origin() {
        let path =
            std::env::temp_dir().join(format!("isolated-order-{}.sqlite", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", path.display());
        let db = Database::connect(&url).await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(41, None).await.unwrap();
        let order_id = db
            .insert_order_with_origin(
                user.id,
                None,
                None,
                None,
                "test-mint",
                "buy",
                Some(0.001),
                None,
                "accepted",
                "isolated-order-test",
                None,
                None,
                None,
                None,
                "isolated_sdk_simulation",
                Some("sim-1"),
            )
            .await
            .unwrap();
        let orphan_id = db
            .insert_order_with_origin(
                user.id,
                None,
                None,
                None,
                "test-mint",
                "buy",
                Some(0.001),
                None,
                "submitting",
                "isolated-no-retry-test",
                Some("{}"),
                None,
                None,
                None,
                "isolated_sdk_simulation",
                None,
            )
            .await
            .unwrap();
        let primary = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let isolated_address = isolated.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = isolated.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let count = socket.read(&mut buffer).await.unwrap();
            let wire = String::from_utf8_lossy(&buffer[..count]);
            assert!(wire.starts_with("GET /v1/trades/sim-1 "));
            assert!(wire
                .to_ascii_lowercase()
                .contains("authorization: bearer isolated-token"));
            let body = r#"{"request_id":"sim-1","wallet_id":"w1","wallet_pubkey":"test-pubkey","status":"simulated","signature":null}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let client =
            TradeServiceClient::new(format!("http://{}", primary.local_addr().unwrap()), None, 1)
                .unwrap()
                .with_route_decision_endpoint(
                    Some(format!("http://{isolated_address}")),
                    Some("isolated-token".into()),
                )
                .unwrap();
        OrderReconciler::new(db.clone(), client, 1, 2)
            .reconcile_once()
            .await
            .unwrap();
        server.await.unwrap();
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        let status: String = sqlx::query_scalar("SELECT status FROM orders WHERE id = ?")
            .bind(&order_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "simulated");
        sqlx::query("UPDATE orders SET updated_at = datetime('now', '-60 seconds') WHERE id = ?")
            .bind(&orphan_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(db.list_orphaned_submissions(20).await.unwrap().is_empty());
        assert!(
            tokio::time::timeout(Duration::from_millis(30), primary.accept())
                .await
                .is_err()
        );
        pool.close().await;
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn paused_recovery_preserves_eligible_orphan_without_attempts() {
        let path =
            std::env::temp_dir().join(format!("paused-recovery-{}.sqlite", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}", path.display());
        let db = Database::connect(&url).await.unwrap();
        db.migrate().await.unwrap();
        let user = db.upsert_user(1, None).await.unwrap();
        let (order_id, _) = db
            .reserve_order(
                user.id,
                None,
                None,
                None,
                "test-mint",
                "buy",
                Some(0.001),
                None,
                "paused-orphan",
                "{}",
            )
            .await
            .unwrap();
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("UPDATE orders SET updated_at = datetime('now', '-60 seconds') WHERE id = ?")
            .bind(&order_id)
            .execute(&pool)
            .await
            .unwrap();
        let before = db.list_orphaned_submissions(20).await.unwrap();
        assert_eq!(before.len(), 1);
        let client = TradeServiceClient::new("http://127.0.0.1:1".into(), None, 1).unwrap();
        OrderReconciler::new(db.clone(), client, 1, 2)
            .reconcile_once()
            .await
            .unwrap();
        assert_eq!(db.list_orphaned_submissions(20).await.unwrap(), before);
        pool.close().await;
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
