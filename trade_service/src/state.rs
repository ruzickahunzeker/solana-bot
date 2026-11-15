use crate::application::trade_record::TradeRecordV1 as TradeRecord;
#[cfg(test)]
use crate::domain::trade::ExecutionMethod;
use crate::domain::trade::{Pool, TradeStatus};
#[cfg(test)]
use crate::models::TradeRequest;
use dashmap::DashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Default)]
pub struct TradeStore {
    records: DashMap<String, TradeRecord>,
}

impl TradeStore {
    pub(crate) fn insert_record(&self, record: &TradeRecord) {
        self.records
            .insert(record.request_id.clone(), record.clone());
    }
    #[cfg(test)]
    pub fn create(
        &self,
        request: &TradeRequest,
        wallet_id: String,
        wallet_pubkey: String,
    ) -> TradeRecord {
        self.create_with_id(new_request_id(), request, wallet_id, wallet_pubkey)
    }

    #[cfg(test)]
    pub fn create_with_id(
        &self,
        request_id: String,
        request: &TradeRequest,
        wallet_id: String,
        wallet_pubkey: String,
    ) -> TradeRecord {
        let now = now_ms();
        let record = TradeRecord {
            request_id: request_id.clone(),
            wallet_id,
            wallet_pubkey,
            side: request.side,
            pool: request.pool,
            mint: request.mint.clone(),
            pool_id: request.pool_id.clone(),
            execution_method: request.execution_method.unwrap_or(ExecutionMethod::Sdk),
            status: TradeStatus::Accepted,
            signature: None,
            error: None,
            duplicate: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        self.records.insert(request_id, record.clone());
        record
    }

    #[cfg(test)]
    pub fn rejected(
        &self,
        request: &TradeRequest,
        wallet_id: String,
        wallet_pubkey: String,
        error: impl Into<String>,
    ) -> TradeRecord {
        let mut record = self.create(request, wallet_id, wallet_pubkey);
        record.status = TradeStatus::Rejected;
        record.error = Some(error.into());
        record.updated_at_ms = now_ms();
        self.records
            .insert(record.request_id.clone(), record.clone());
        record
    }

    pub fn get(&self, request_id: &str) -> Option<TradeRecord> {
        self.records.get(request_id).map(|entry| entry.clone())
    }

    pub fn remove(&self, request_id: &str) {
        self.records.remove(request_id);
    }

    pub fn recent(&self, limit: usize) -> Vec<TradeRecord> {
        let mut records = self
            .records
            .iter()
            .map(|entry| entry.clone())
            .collect::<Vec<_>>();
        records.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        records.truncate(limit);
        records
    }

    pub fn update(
        &self,
        request_id: &str,
        status: TradeStatus,
        signature: Option<String>,
        error: Option<String>,
    ) -> Option<TradeRecord> {
        self.records.get_mut(request_id).map(|mut entry| {
            entry.status = status;
            if signature.is_some() {
                entry.signature = signature;
            }
            entry.error = error;
            entry.updated_at_ms = now_ms();
            entry.clone()
        })
    }
}

pub fn new_request_id() -> String {
    format!("tr_{}", Uuid::new_v4().simple())
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn cache_key(pool: Pool, mint: &str, pool_id: Option<&str>) -> String {
    match pool {
        Pool::Pump => format!("pump:{mint}"),
        Pool::PumpAmm => match pool_id {
            Some(pool_id) => format!("pump-amm:{pool_id}"),
            None => format!("pump-amm-mint:{mint}"),
        },
        Pool::Bonk => format!("bonk:{mint}"),
        Pool::StonkFun => format!("stonkfun:{}", pool_id.unwrap_or(mint)),
        Pool::StonkFunSwap => format!("stonkfun-swap:{}", pool_id.unwrap_or(mint)),
        Pool::MeteoraDammV2 => format!("meteora-damm-v2:{}", pool_id.unwrap_or(mint)),
        Pool::MeteoraDbc => format!("meteora-dbc:{}", pool_id.unwrap_or(mint)),
        Pool::RaydiumAmmV4 => format!("raydium-amm-v4:{}", pool_id.unwrap_or(mint)),
        Pool::RaydiumCpmm => format!("raydium-cpmm:{}", pool_id.unwrap_or(mint)),
    }
}
