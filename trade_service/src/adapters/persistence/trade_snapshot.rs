//! Frozen V1 ordinary-trade storage codec and compatibility bridge.
#[cfg(test)]
use crate::{
    application::ports::ordinary_journal::OrdinaryTradeJournal,
    config::TradePersistenceMode,
    persistence::{DurableReserve, TradePersistence},
};
use crate::{
    application::trade_persistence_input::{
        RejectedTradeInput, StoredRequestV1, ValidatedTradeInput,
    },
    application::trade_record::TradeRecordV1 as TradeRecord,
    application::validated_trade::ValidatedTrade as NormalizedTrade,
    models::{MevProvider, TradeRequest},
    persistence::{execution_name, pool_name, side_name},
    validation::replay_uses_sol_aggregator,
};
use anyhow::{bail, Context};
use serde_json::json;
use sha2::{Digest, Sha256};

pub(crate) fn validated(
    wallet_id: &str,
    request: &TradeRequest,
    normalized: &NormalizedTrade,
) -> anyhow::Result<ValidatedTradeInput> {
    Ok(ValidatedTradeInput::new(
        wallet_id.into(),
        request.idempotency_key.clone(),
        normalized.aggregator_sol,
        StoredRequestV1::new(
            request_hash(wallet_id, request, normalized)?,
            serde_json::to_string(request)?,
        ),
    ))
}

pub(crate) fn rejected(
    wallet_id: &str,
    request_id: &str,
    request: &TradeRequest,
) -> anyhow::Result<RejectedTradeInput> {
    Ok(RejectedTradeInput::new(
        wallet_id.into(),
        request_id.into(),
        StoredRequestV1::new(
            hash_json(json!({"wallet_id":wallet_id,"request":request}))?,
            serde_json::to_string(request)?,
        ),
    ))
}

pub(crate) fn verify_stored_replay(
    record: &TradeRecord,
    request_json: &str,
    incoming: &ValidatedTradeInput,
) -> anyhow::Result<bool> {
    verify_stored_identity(
        record,
        request_json,
        incoming.idempotency_key(),
        incoming.aggregator_sol(),
    )
}

pub(crate) fn verify_stored_identity(
    record: &TradeRecord,
    request_json: &str,
    idempotency_key: &str,
    aggregator_sol: bool,
) -> anyhow::Result<bool> {
    let stored: TradeRequest =
        serde_json::from_str(request_json).context("stored trade request cannot be verified")?;
    if stored.idempotency_key != idempotency_key
        || stored
            .wallet_id
            .as_deref()
            .is_some_and(|id| id != record.wallet_id)
        || stored.side != record.side
        || stored.pool != record.pool
        || stored.mint != record.mint
    {
        bail!("stored trade request identity does not match reservation");
    }
    let stored_aggregator = replay_uses_sol_aggregator(&stored)
        .context("stored trade input path cannot be verified")?;
    Ok(stored_aggregator == aggregator_sol)
}

// Legacy entry points remain test oracles; production callers now use prepared project inputs.
#[cfg(test)]
impl TradePersistence {
    pub async fn reserve(
        &self,
        record: &TradeRecord,
        request: &TradeRequest,
        normalized: &NormalizedTrade,
    ) -> anyhow::Result<DurableReserve> {
        if !self.is_durable() {
            return Ok(DurableReserve::Created);
        }
        self.require_storage("durable database is unavailable")?;
        self.reserve_record(record, &validated(&record.wallet_id, request, normalized)?)
            .await
    }

    pub async fn mirror(
        &self,
        record: &TradeRecord,
        request: &TradeRequest,
        normalized: Option<&NormalizedTrade>,
    ) -> anyhow::Result<()> {
        if self.mode() == TradePersistenceMode::Memory {
            return Ok(());
        }
        self.require_storage("trade database is unavailable")?;
        match normalized {
            Some(normalized) => {
                self.mirror_accepted(record, validated(&record.wallet_id, request, normalized)?)
                    .await
            }
            None => {
                self.mirror_rejection(
                    record,
                    rejected(&record.wallet_id, &record.request_id, request)?,
                )
                .await
            }
        }
    }
}

pub fn request_hash(
    wallet_id: &str,
    request: &TradeRequest,
    value: &NormalizedTrade,
) -> anyhow::Result<String> {
    hash_json(json!({
        "wallet_id": wallet_id, "side": side_name(request.side), "pool": pool_name(request.pool),
        "mint": value.mint.to_string(), "quote_mint": value.quote_mint.to_string(),
        "pool_id": value.pool_id.map(|v| v.to_string()), "amount_units": value.amount_lamports,
        "settlement_asset": format!("{:?}", value.settlement_asset),
        "sol_hop_pool": value.sol_hop_pool.map(pool_name),
        "sol_hop_pool_id": value.sol_hop_pool_id.map(|v| v.to_string()),
        "sell_percent": value.sell_percent, "slippage_bps": value.slippage_bps,
        "tip_sol_bits": value.tip_sol.to_bits(), "execution_method": execution_name(value.execution_method),
        "simulate": value.simulate, "use_mev": value.use_mev,
        "mev_provider": value.mev_provider.map(mev_name),
    }))
}

fn hash_json(value: serde_json::Value) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(&value)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn mev_name(v: MevProvider) -> &'static str {
    match v {
        MevProvider::Auto => "auto",
        MevProvider::Default => "default",
        MevProvider::Jito => "jito",
        MevProvider::Temporal => "temporal",
        MevProvider::FlashBlock => "flashblock",
        MevProvider::BlockRazor => "blockrazor",
        MevProvider::Astralane => "astralane",
        MevProvider::Speedlanding => "speedlanding",
    }
}
