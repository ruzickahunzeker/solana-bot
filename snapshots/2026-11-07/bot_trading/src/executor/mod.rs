use crate::{
    db::{
        models::{PumpApiEvent, StrategyConfig, TradeWalletRef},
        Database,
    },
    execution_params::{ExecutionMode, ExecutionParams},
    risk::RiskEngine,
    strategy,
    trade_client::{TradeRequest, TradeResponse, TradeServiceClient},
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::time::{Duration, Instant};
use tracing::{info, warn};

#[derive(Clone)]
pub struct ExecutionRouter {
    dry_run: DryRunExecutor,
    live: LiveExecutor,
}

impl ExecutionRouter {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Database,
        risk: RiskEngine,
        trade_client: TradeServiceClient,
        submit_max_attempts: u32,
        retry_delay_ms: u64,
    ) -> Self {
        Self {
            dry_run: DryRunExecutor::new(db.clone()),
            live: LiveExecutor::new(db, risk, trade_client, submit_max_attempts, retry_delay_ms),
        }
    }

    pub async fn execute(&self, request: ExecutionRequest<'_>) -> Result<()> {
        self.live.trade_client.ensure_submission_enabled()?;
        let execution = request.strategy.execution_params();
        execution.validate()?;
        request.strategy.buy_size().validate()?;
        request.strategy.sell_size().validate()?;
        if execution.mode.is_dry_run() {
            self.dry_run.execute(request).await
        } else {
            self.live.execute(request).await
        }
    }

    pub async fn execute_limit_order(
        &self,
        request: LimitOrderExecutionRequest<'_>,
    ) -> Result<ExecutionOutcome> {
        self.live.trade_client.ensure_submission_enabled()?;
        request.execution.validate()?;
        if request.execution.mode.is_dry_run() {
            self.dry_run.execute_limit_order(request).await
        } else {
            self.live.record_limit_order(request).await
        }
    }

    pub async fn execute_fast_follow(&self, request: FastFollowExecutionRequest) -> Result<()> {
        self.live.trade_client.ensure_submission_enabled()?;
        self.live.execute_fast_follow(request).await
    }
}

pub struct ExecutionRequest<'a> {
    pub strategy: &'a StrategyConfig,
    pub signal_id: i64,
    pub source_event_id: i64,
    pub mint: &'a str,
    pub side: &'a str,
    pub event: &'a PumpApiEvent,
}

pub struct FastFollowExecutionRequest {
    pub strategy: StrategyConfig,
    pub event: PumpApiEvent,
    pub mint: String,
    pub side: String,
    pub trade_service_wallet_id: String,
    pub allow_mev: bool,
    pub signal_reason: String,
    pub signal_metadata_json: String,
}

pub struct LimitOrderExecutionRequest<'a> {
    pub user_id: i64,
    pub wallet_id: Option<i64>,
    pub limit_order_id: &'a str,
    pub source_event_id: i64,
    pub mint: &'a str,
    pub side: &'a str,
    pub amount_sol: Option<f64>,
    pub sell_percent: Option<f64>,
    pub execution: ExecutionParams,
    pub trigger_price: f64,
    pub trigger_currency: &'a str,
    pub observed_price: f64,
    pub observed_currency: &'a str,
    pub event: &'a PumpApiEvent,
}

pub struct ExecutionOutcome {
    pub order_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct LiveExecutionPlan {
    strategy_id: i64,
    signal_id: i64,
    wallet_ref: TradeWalletRef,
    trade_request: TradeRequest,
    mev_mode: String,
    note: String,
}

#[derive(Clone)]
struct DryRunExecutor {
    db: Database,
}

impl DryRunExecutor {
    fn new(db: Database) -> Self {
        Self { db }
    }

    async fn execute(&self, request: ExecutionRequest<'_>) -> Result<()> {
        match request.side {
            "buy" => self.create_buy_order(request).await,
            "sell" => self.create_sell_order(request).await,
            _ => Ok(()),
        }
    }

    async fn create_buy_order(&self, request: ExecutionRequest<'_>) -> Result<()> {
        let amount_sol = strategy::effective_buy_amount_sol(request.strategy, request.event);
        let idempotency_key = strategy_idempotency_key(
            "dryrun",
            request.strategy.id,
            request.source_event_id,
            request.side,
        );
        self.db
            .insert_order(
                request.strategy.user_id,
                request.strategy.wallet_id,
                Some(request.strategy.id),
                Some(request.signal_id),
                request.mint,
                "buy",
                Some(amount_sol),
                None,
                "dry_run",
                &idempotency_key,
                None,
                None,
                None,
                None,
            )
            .await?;
        self.db
            .add_dry_run_position_buy(
                request.strategy.user_id,
                request.strategy.id,
                request.strategy.wallet_id,
                request.mint,
                amount_sol,
                request.signal_id,
            )
            .await?;
        Ok(())
    }

    async fn create_sell_order(&self, request: ExecutionRequest<'_>) -> Result<()> {
        let sell_percent = request.strategy.sell_percent.unwrap_or(100.0);
        let idempotency_key = strategy_idempotency_key(
            "dryrun",
            request.strategy.id,
            request.source_event_id,
            request.side,
        );
        let position_amount = self
            .db
            .get_dry_run_position_amount(
                request.strategy.user_id,
                request.strategy.id,
                request.mint,
            )
            .await?
            .unwrap_or(0.0);

        if position_amount <= 0.0 {
            self.db
                .insert_order(
                    request.strategy.user_id,
                    request.strategy.wallet_id,
                    Some(request.strategy.id),
                    Some(request.signal_id),
                    request.mint,
                    "sell",
                    None,
                    Some(sell_percent),
                    "skipped",
                    &idempotency_key,
                    None,
                    None,
                    None,
                    Some("no dry-run position for mint"),
                )
                .await?;
            return Ok(());
        }

        self.db
            .insert_order(
                request.strategy.user_id,
                request.strategy.wallet_id,
                Some(request.strategy.id),
                Some(request.signal_id),
                request.mint,
                "sell",
                None,
                Some(sell_percent),
                "dry_run",
                &idempotency_key,
                None,
                None,
                None,
                None,
            )
            .await?;
        self.db
            .apply_dry_run_position_sell(
                request.strategy.user_id,
                request.strategy.id,
                request.mint,
                sell_percent,
                request.signal_id,
            )
            .await?;
        Ok(())
    }

    async fn execute_limit_order(
        &self,
        request: LimitOrderExecutionRequest<'_>,
    ) -> Result<ExecutionOutcome> {
        let idempotency_key = format!("limit-dryrun-{}", request.limit_order_id);
        let request_json = serde_json::json!({
            "limit_order_id": request.limit_order_id,
            "source_event_id": request.source_event_id,
            "trigger_price": request.trigger_price,
            "trigger_currency": request.trigger_currency,
            "observed_price": request.observed_price,
            "observed_currency": request.observed_currency,
            "event_signature": request.event.signature,
            "pool": request.event.pool,
            "pool_id": request.event.pool_id,
            "quote_mint": request.event.quote_mint,
        })
        .to_string();
        let order_id = self
            .db
            .insert_order(
                request.user_id,
                request.wallet_id,
                None,
                None,
                request.mint,
                request.side,
                request.amount_sol,
                request.sell_percent,
                "dry_run",
                &idempotency_key,
                Some(&request_json),
                None,
                None,
                None,
            )
            .await?;
        Ok(ExecutionOutcome {
            order_id: Some(order_id),
        })
    }
}

#[derive(Clone)]
struct LiveExecutor {
    db: Database,
    risk: RiskEngine,
    trade_client: TradeServiceClient,
    submit_max_attempts: u32,
    retry_delay_ms: u64,
}

impl LiveExecutor {
    #[allow(clippy::too_many_arguments)]
    fn new(
        db: Database,
        risk: RiskEngine,
        trade_client: TradeServiceClient,
        submit_max_attempts: u32,
        retry_delay_ms: u64,
    ) -> Self {
        Self {
            db,
            risk,
            trade_client,
            submit_max_attempts: submit_max_attempts.max(1),
            retry_delay_ms,
        }
    }

    async fn execute(&self, request: ExecutionRequest<'_>) -> Result<()> {
        let idempotency_key = strategy_idempotency_key(
            "live",
            request.strategy.id,
            request.source_event_id,
            request.side,
        );
        let decision = self.risk.check_live_trade_allowed(false);
        if !decision.allowed {
            return self
                .record_strategy_blocked(request, &idempotency_key, &decision.reason)
                .await;
        }

        let trade_service_wallet_id = match self
            .resolve_trade_service_wallet(request.strategy.user_id, request.strategy.wallet_id)
            .await
        {
            Ok(wallet_id) => wallet_id,
            Err(err) => {
                return self
                    .record_strategy_blocked(request, &idempotency_key, &err.to_string())
                    .await;
            }
        };
        let allow_mev = self
            .db
            .user_mev_enabled(request.strategy.user_id)
            .await
            .unwrap_or(false);
        let trade_request = match build_strategy_trade_request(
            &request,
            idempotency_key.clone(),
            trade_service_wallet_id,
            allow_mev,
        ) {
            Ok(trade_request) => trade_request,
            Err(err) => {
                return self
                    .record_strategy_blocked(request, &idempotency_key, &err.to_string())
                    .await;
            }
        };
        let amount_sol = trade_request.amount_sol;
        let sell_percent = trade_request.sell_percent.map(|value| value as f64);
        let plan = LiveExecutionPlan {
            strategy_id: request.strategy.id,
            signal_id: request.signal_id,
            wallet_ref: request
                .strategy
                .wallet_id
                .map(TradeWalletRef::CopyTradingWalletId)
                .unwrap_or(TradeWalletRef::TradeServiceDefault),
            trade_request,
            mev_mode: if allow_mev {
                request.strategy.mev_mode.clone()
            } else {
                "none".to_string()
            },
            note: "live trade submitted through trade_service".to_string(),
        };
        let request_json = serde_json::to_string(&plan)?;
        let trade_request = &plan.trade_request;
        let (order_id, created) = self
            .db
            .reserve_order(
                request.strategy.user_id,
                request.strategy.wallet_id,
                Some(request.strategy.id),
                Some(request.signal_id),
                request.mint,
                request.side,
                amount_sol,
                sell_percent,
                &idempotency_key,
                &request_json,
            )
            .await?;
        if created {
            self.submit_with_retry(&order_id, trade_request, &request_json)
                .await?;
        }
        Ok(())
    }

    async fn execute_fast_follow(&self, request: FastFollowExecutionRequest) -> Result<()> {
        let signature = request
            .event
            .signature
            .as_deref()
            .context("live fast follow requires a source signature")?;
        let idempotency_key =
            fast_strategy_idempotency_key(request.strategy.id, signature, request.side.as_str());
        let decision = self.risk.check_live_trade_allowed(false);
        if !decision.allowed {
            anyhow::bail!(decision.reason);
        }
        let trade_request = build_strategy_trade_request_parts(
            &request.strategy,
            &request.event,
            &request.mint,
            &request.side,
            idempotency_key.clone(),
            request.trade_service_wallet_id.clone(),
            request.allow_mev,
            true,
        )?;

        let event_to_submit_start_ms =
            chrono::Utc::now().timestamp_millis() - request.event.received_unix_ms;
        let submit_started = Instant::now();
        let mut response = None;
        let mut last_error = None;
        for attempt in 1..=self.submit_max_attempts {
            match self.trade_client.submit_trade(&trade_request).await {
                Ok(item) => {
                    response = Some(item);
                    break;
                }
                Err(err) => {
                    last_error = Some(format!("attempt {attempt}: {err}"));
                    if attempt < self.submit_max_attempts && self.retry_delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                    }
                }
            }
        }
        let submit_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
        info!(
            strategy_id = request.strategy.id,
            mint = request.mint,
            side = request.side,
            submit_ms,
            event_to_submit_start_ms,
            "fast follow trade request completed"
        );

        if let Err(err) = self
            .persist_fast_follow_result(
                &request,
                &trade_request,
                &idempotency_key,
                response.as_ref(),
                last_error.as_deref(),
            )
            .await
        {
            warn!(
                ?err,
                strategy_id = request.strategy.id,
                idempotency_key,
                "fast follow audit persistence failed after trade submission"
            );
        }

        if response.is_none() {
            anyhow::bail!(last_error.unwrap_or_else(|| "trade submission failed".to_string()));
        }
        Ok(())
    }

    async fn persist_fast_follow_result(
        &self,
        request: &FastFollowExecutionRequest,
        trade_request: &TradeRequest,
        idempotency_key: &str,
        response: Option<&TradeResponse>,
        submit_error: Option<&str>,
    ) -> Result<()> {
        let source_event_id = self.db.insert_or_get_pumpapi_event(&request.event).await?;
        let signal_id = self
            .db
            .insert_signal(
                request.strategy.user_id,
                Some(request.strategy.id),
                source_event_id,
                &request.mint,
                &request.side,
                &request.signal_reason,
                "live",
                Some(&request.signal_metadata_json),
            )
            .await?;
        let plan = LiveExecutionPlan {
            strategy_id: request.strategy.id,
            signal_id,
            wallet_ref: request
                .strategy
                .wallet_id
                .map(TradeWalletRef::CopyTradingWalletId)
                .unwrap_or(TradeWalletRef::TradeServiceDefault),
            trade_request: trade_request.clone(),
            mev_mode: if request.allow_mev {
                request.strategy.mev_mode.clone()
            } else {
                "none".to_string()
            },
            note: "fast live follow submitted through trade_service".to_string(),
        };
        let request_json = serde_json::to_string(&plan)?;
        let (order_id, created) = self
            .db
            .reserve_order(
                request.strategy.user_id,
                request.strategy.wallet_id,
                Some(request.strategy.id),
                Some(signal_id),
                &request.mint,
                &request.side,
                trade_request.amount_sol,
                trade_request.sell_percent.map(|value| value as f64),
                idempotency_key,
                &request_json,
            )
            .await?;
        if !created {
            return Ok(());
        }
        if let Some(response) = response {
            self.persist_trade_response(&order_id, response).await?;
        } else {
            let error = submit_error.unwrap_or("trade submission failed");
            self.db
                .update_order_from_trade_service(&order_id, "failed", None, None, None, Some(error))
                .await?;
            self.db
                .insert_order_attempt(
                    &order_id,
                    "submit_failed",
                    Some(&request_json),
                    None,
                    Some(error),
                )
                .await?;
        }
        Ok(())
    }

    async fn record_strategy_blocked(
        &self,
        request: ExecutionRequest<'_>,
        idempotency_key: &str,
        reason: &str,
    ) -> Result<()> {
        let amount_sol = (request.side == "buy")
            .then(|| strategy::effective_buy_amount_sol(request.strategy, request.event));
        let sell_percent =
            (request.side == "sell").then_some(request.strategy.sell_percent.unwrap_or(100.0));
        let request_json = serde_json::json!({
            "strategy_id": request.strategy.id,
            "signal_id": request.signal_id,
            "source_event_id": request.source_event_id,
            "reason": reason,
        })
        .to_string();
        let (order_id, created) = self
            .db
            .reserve_order(
                request.strategy.user_id,
                request.strategy.wallet_id,
                Some(request.strategy.id),
                Some(request.signal_id),
                request.mint,
                request.side,
                amount_sol,
                sell_percent,
                idempotency_key,
                &request_json,
            )
            .await?;
        if created {
            self.db
                .update_order_from_trade_service(
                    &order_id,
                    "blocked",
                    None,
                    None,
                    None,
                    Some(reason),
                )
                .await?;
        }
        Ok(())
    }

    async fn record_limit_order(
        &self,
        request: LimitOrderExecutionRequest<'_>,
    ) -> Result<ExecutionOutcome> {
        let idempotency_key = format!("limit-live-{}", request.limit_order_id);
        let decision = self.risk.check_live_trade_allowed(false);
        if !decision.allowed {
            return self
                .record_limit_blocked(request, &idempotency_key, &decision.reason)
                .await;
        }
        let trade_service_wallet_id = match self
            .resolve_trade_service_wallet(request.user_id, request.wallet_id)
            .await
        {
            Ok(wallet_id) => wallet_id,
            Err(err) => {
                return self
                    .record_limit_blocked(request, &idempotency_key, &err.to_string())
                    .await;
            }
        };
        let sell_percent = match request.sell_percent.map(percent_to_u64).transpose() {
            Ok(value) => value,
            Err(err) => {
                return self
                    .record_limit_blocked(request, &idempotency_key, &err.to_string())
                    .await;
            }
        };
        let pool = match normalize_pool(request.event.pool.as_deref()) {
            Ok(pool) => pool,
            Err(err) => {
                return self
                    .record_limit_blocked(request, &idempotency_key, &err.to_string())
                    .await;
            }
        };
        let pool_id = trade_service_pool_id(&pool, request.event.pool_id.as_deref());
        let allow_mev = self
            .db
            .user_mev_enabled(request.user_id)
            .await
            .unwrap_or(false);
        let mev_mode = if allow_mev {
            request.execution.mev_mode.as_str()
        } else {
            "none"
        };
        let (use_mev, mev_provider) = mev_settings(mev_mode)?;
        let trade_request = TradeRequest {
            wallet_id: Some(trade_service_wallet_id),
            execution_method: request.execution.method.as_str().to_string(),
            side: request.side.to_string(),
            pool,
            mint: request.mint.to_string(),
            pool_id,
            quote_mint: request.event.quote_mint.clone(),
            amount_quote_units: None,
            settlement_asset: None,
            sol_hop_pool: None,
            sol_hop_pool_id: None,
            sdk_decision_id: None,
            sdk_preview_id: None,
            amount_sol: request.amount_sol,
            sell_percent,
            slippage_percent: request.execution.slippage_bps as f64 / 100.0,
            tip_sol: Some(request.execution.tip_sol),
            use_mev: Some(use_mev),
            mev_provider,
            simulate: request.execution.mode == ExecutionMode::Simulate,
            source: Some("copy-trading-limit".to_string()),
            source_signature: request.event.signature.clone(),
            idempotency_key: idempotency_key.clone(),
        };
        let request_json = serde_json::json!({
            "limit_order_id": request.limit_order_id,
            "source_event_id": request.source_event_id,
            "trigger_price": request.trigger_price,
            "trigger_currency": request.trigger_currency,
            "observed_price": request.observed_price,
            "observed_currency": request.observed_currency,
            "trade_request": trade_request,
            "note": "live limit trade submitted through trade_service",
        })
        .to_string();
        let (order_id, created) = self
            .db
            .reserve_order(
                request.user_id,
                request.wallet_id,
                None,
                None,
                request.mint,
                request.side,
                request.amount_sol,
                request.sell_percent,
                &idempotency_key,
                &request_json,
            )
            .await?;
        if created {
            self.submit_with_retry(&order_id, &trade_request, &request_json)
                .await?;
        }
        Ok(ExecutionOutcome {
            order_id: Some(order_id),
        })
    }

    async fn record_limit_blocked(
        &self,
        request: LimitOrderExecutionRequest<'_>,
        idempotency_key: &str,
        reason: &str,
    ) -> Result<ExecutionOutcome> {
        let request_json = serde_json::json!({
            "limit_order_id": request.limit_order_id,
            "source_event_id": request.source_event_id,
            "reason": reason,
        })
        .to_string();
        let (order_id, created) = self
            .db
            .reserve_order(
                request.user_id,
                request.wallet_id,
                None,
                None,
                request.mint,
                request.side,
                request.amount_sol,
                request.sell_percent,
                idempotency_key,
                &request_json,
            )
            .await?;
        if created {
            self.db
                .update_order_from_trade_service(
                    &order_id,
                    "blocked",
                    None,
                    None,
                    None,
                    Some(reason),
                )
                .await?;
        }
        Ok(ExecutionOutcome {
            order_id: Some(order_id),
        })
    }

    async fn resolve_trade_service_wallet(
        &self,
        user_id: i64,
        wallet_id: Option<i64>,
    ) -> Result<String> {
        let wallet_id = wallet_id.context("copy-trading wallet is not selected")?;
        let wallet = self
            .db
            .get_wallet(user_id, wallet_id)
            .await
            .context("copy-trading wallet is unavailable")?;
        let ready = self.trade_client.ready().await?;
        if !ready.ready {
            anyhow::bail!("trade_service is not ready");
        }
        ready
            .wallets
            .into_iter()
            .find(|candidate| candidate.pubkey == wallet.public_key)
            .map(|candidate| candidate.wallet_id)
            .with_context(|| {
                format!(
                    "wallet {} is not configured in trade_service",
                    wallet.public_key
                )
            })
    }

    async fn submit_with_retry(
        &self,
        order_id: &str,
        request: &TradeRequest,
        request_json: &str,
    ) -> Result<()> {
        self.db
            .update_order_from_trade_service(order_id, "submitting", None, None, None, None)
            .await?;
        let mut last_error = None;
        for attempt in 1..=self.submit_max_attempts {
            self.db
                .insert_order_attempt(order_id, "submitting", Some(request_json), None, None)
                .await?;
            match self.trade_client.submit_trade(request).await {
                Ok(response) => {
                    self.persist_trade_response(order_id, &response).await?;
                    return Ok(());
                }
                Err(err) => {
                    let error = format!("attempt {attempt}: {err}");
                    self.db
                        .insert_order_attempt(
                            order_id,
                            "submit_failed",
                            Some(request_json),
                            None,
                            Some(&error),
                        )
                        .await?;
                    last_error = Some(error);
                    if attempt < self.submit_max_attempts && self.retry_delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(self.retry_delay_ms)).await;
                    }
                }
            }
        }

        let error = last_error.unwrap_or_else(|| "trade submission failed".to_string());
        self.db
            .update_order_from_trade_service(order_id, "failed", None, None, None, Some(&error))
            .await?;
        Ok(())
    }

    async fn persist_trade_response(&self, order_id: &str, response: &TradeResponse) -> Result<()> {
        let response_json = serde_json::to_string(response)?;
        let status = normalize_trade_status(response);
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

fn build_strategy_trade_request(
    request: &ExecutionRequest<'_>,
    idempotency_key: String,
    trade_service_wallet_id: String,
    allow_mev: bool,
) -> Result<TradeRequest> {
    build_strategy_trade_request_parts(
        request.strategy,
        request.event,
        request.mint,
        request.side,
        idempotency_key,
        trade_service_wallet_id,
        allow_mev,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_strategy_trade_request_parts(
    strategy: &StrategyConfig,
    event: &PumpApiEvent,
    mint: &str,
    side: &str,
    idempotency_key: String,
    trade_service_wallet_id: String,
    allow_mev: bool,
    require_event_route: bool,
) -> Result<TradeRequest> {
    let mut execution = strategy.execution_params();
    if !allow_mev {
        execution.mev_mode = "none".to_string();
    }
    execution.validate()?;
    let amount_sol = (side == "buy").then(|| strategy::effective_buy_amount_sol(strategy, event));
    if amount_sol.is_some_and(|amount| !amount.is_finite() || amount <= 0.0) {
        anyhow::bail!("buy amount must be greater than 0");
    }
    let sell_percent = if side == "sell" {
        Some(percent_to_u64(strategy.sell_percent.unwrap_or(100.0))?)
    } else {
        None
    };
    if require_event_route && event.pool.is_none() {
        anyhow::bail!("fast follow event is missing pool");
    }
    let pool = normalize_pool(event.pool.as_deref())?;
    let pool_id = trade_service_pool_id(&pool, event.pool_id.as_deref());
    if require_event_route && !matches!(pool.as_str(), "pump" | "bonk") && pool_id.is_none() {
        anyhow::bail!("fast follow event is missing pool_id for {pool}");
    }
    let (use_mev, mev_provider) = mev_settings(&execution.mev_mode)?;
    Ok(TradeRequest {
        wallet_id: Some(trade_service_wallet_id),
        execution_method: execution.method.as_str().to_string(),
        side: side.to_string(),
        pool,
        mint: mint.to_string(),
        pool_id,
        quote_mint: event.quote_mint.clone(),
        amount_quote_units: None,
        settlement_asset: None,
        sol_hop_pool: None,
        sol_hop_pool_id: None,
        sdk_decision_id: None,
        sdk_preview_id: None,
        amount_sol,
        sell_percent,
        slippage_percent: execution.slippage_bps as f64 / 100.0,
        tip_sol: Some(execution.tip_sol),
        use_mev: Some(use_mev),
        mev_provider,
        simulate: execution.mode == ExecutionMode::Simulate,
        source: Some("copy-trading-strategy".to_string()),
        source_signature: event.signature.clone(),
        idempotency_key,
    })
}

fn mev_settings(mev_mode: &str) -> Result<(bool, Option<String>)> {
    match mev_mode {
        "none" => Ok((false, None)),
        "auto" => Ok((true, None)),
        "default" | "rpc" | "jito" | "temporal" | "flashblock" | "blockrazor" | "astralane"
        | "speedlanding" => Ok((true, Some(mev_mode.to_string()))),
        value => anyhow::bail!("unsupported MEV mode {value}"),
    }
}

fn strategy_idempotency_key(
    prefix: &str,
    strategy_id: i64,
    source_event_id: i64,
    side: &str,
) -> String {
    format!("{prefix}-strategy-{strategy_id}-event-{source_event_id}-{side}")
}

fn fast_strategy_idempotency_key(strategy_id: i64, signature: &str, side: &str) -> String {
    format!("cf:{strategy_id}:{signature}:{side}")
}

fn percent_to_u64(value: f64) -> Result<u64> {
    if !value.is_finite() || !(1.0..=100.0).contains(&value) {
        anyhow::bail!("sell percent must be between 1 and 100");
    }
    let rounded = value.round();
    if (value - rounded).abs() > f64::EPSILON {
        anyhow::bail!("trade_service requires an integer sell percent");
    }
    Ok(rounded as u64)
}

fn normalize_pool(value: Option<&str>) -> Result<String> {
    let value = value.unwrap_or("pump").trim().to_ascii_lowercase();
    match value.as_str() {
        "pump" | "pump-amm" | "bonk" | "stonkfun" | "stonkfun-swap" | "meteora-damm-v2"
        | "meteora-dbc" | "raydium-amm-v4" | "raydium-cpmm" => Ok(value),
        _ => anyhow::bail!("unsupported trade_service pool {value}"),
    }
}

pub(crate) fn trade_service_pool_id(pool: &str, pool_id: Option<&str>) -> Option<String> {
    match pool {
        "pump" | "bonk" => None,
        _ => pool_id.map(ToString::to_string),
    }
}

pub fn normalize_trade_status(response: &TradeResponse) -> &str {
    match response.status.as_deref() {
        Some("accepted") => "accepted",
        Some("warming_params") => "warming_params",
        Some("building_tx") => "building_tx",
        Some("submitting") => "submitting",
        Some("simulated") => "simulated",
        Some("submitted") => "submitted",
        Some("confirmed") => "confirmed",
        Some("failed") | Some("rejected") => "failed",
        Some("unknown") => "unknown",
        _ if response.signature.is_some() => "submitted",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: Option<&str>, signature: Option<&str>) -> TradeResponse {
        TradeResponse {
            request_id: Some("req-1".to_string()),
            wallet_id: Some("main".to_string()),
            wallet_pubkey: Some("pubkey".to_string()),
            status: status.map(ToString::to_string),
            signature: signature.map(ToString::to_string),
            error: None,
            duplicate: false,
            extra: serde_json::Value::Null,
        }
    }

    #[test]
    fn stable_strategy_idempotency_key() {
        assert_eq!(
            strategy_idempotency_key("live", 7, 42, "buy"),
            "live-strategy-7-event-42-buy"
        );
    }

    #[test]
    fn fast_strategy_key_uses_the_source_signature() {
        assert_eq!(
            fast_strategy_idempotency_key(7, "signature-1", "buy"),
            "cf:7:signature-1:buy"
        );
    }

    #[test]
    fn sell_percent_must_be_whole_number() {
        assert_eq!(percent_to_u64(50.0).unwrap(), 50);
        assert!(percent_to_u64(50.5).is_err());
        assert!(percent_to_u64(0.0).is_err());
        assert!(percent_to_u64(101.0).is_err());
    }

    #[test]
    fn pump_pool_drops_pool_id() {
        assert_eq!(trade_service_pool_id("pump", Some("curve")), None);
        assert_eq!(
            trade_service_pool_id("pump-amm", Some("pool")),
            Some("pool".to_string())
        );
    }

    #[test]
    fn meteora_dbc_is_forwarded_with_pool_id() {
        assert_eq!(normalize_pool(Some("meteora-dbc")).unwrap(), "meteora-dbc");
        assert_eq!(
            trade_service_pool_id("meteora-dbc", Some("dbc-pool")),
            Some("dbc-pool".to_string())
        );
    }

    #[test]
    fn remote_status_is_normalized() {
        assert_eq!(
            normalize_trade_status(&response(Some("accepted"), None)),
            "accepted"
        );
        assert_eq!(
            normalize_trade_status(&response(Some("rejected"), None)),
            "failed"
        );
        assert_eq!(
            normalize_trade_status(&response(None, Some("sig"))),
            "submitted"
        );
    }
}
