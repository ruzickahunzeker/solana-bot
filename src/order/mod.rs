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
        self.recover_orphaned_submissions().await?;
        let orders = self.db.list_reconcilable_orders(100).await?;
        for (order_id, request_id, local_status) in orders {
            let response = match self.trade_client.get_trade(&request_id).await {
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
