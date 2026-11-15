use crate::{
    application::{trade_input::WarmupInput, validated_trade::ValidatedWarmup},
    domain::{route::PoolRouteHint, trade::Pool},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct WarmupRequest {
    pub pool: Pool,
    pub mint: String,
    pub pool_id: Option<String>,
    pub quote_mint: Option<String>,
}

impl WarmupRequest {
    pub(crate) fn validation_input(&self) -> WarmupInput<'_> {
        WarmupInput {
            pool: self.pool,
            mint: &self.mint,
            pool_id: self.pool_id.as_deref(),
            quote_mint: self.quote_mint.as_deref(),
        }
    }

    pub(crate) fn to_route_hint(&self, n: &ValidatedWarmup) -> PoolRouteHint {
        PoolRouteHint {
            pool: self.pool,
            mint: n.mint,
            quote_mint: n.quote_mint,
            pool_id: n.pool_id,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ResolvePoolRequest {
    pub mint: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveSolHopRequest {
    pub quote_mint: String,
}

#[derive(Debug, Serialize)]
pub struct ResolveSolHopResponse {
    pub available: bool,
    pub pool: Option<Pool>,
    pub pool_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvePoolResponse {
    pub mint: String,
    pub pool: Pool,
    pub pool_id: Option<String>,
    pub quote_mint: String,
}

impl From<PoolRouteHint> for ResolvePoolResponse {
    fn from(hint: PoolRouteHint) -> Self {
        Self {
            mint: hint.mint.to_string(),
            pool: hint.pool,
            pool_id: hint.pool_id.map(|id| id.to_string()),
            quote_mint: hint.quote_mint.to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WarmupResponse {
    pub ok: bool,
    pub cache_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::trade::Address;

    #[test]
    fn warmup_mapping_borrows_literal_input_and_copies_project_result_without_sdk_conversion() {
        let request = WarmupRequest {
            pool: Pool::RaydiumCpmm,
            mint: " literal mint ".into(),
            pool_id: Some("literal pool".into()),
            quote_mint: Some("literal quote".into()),
        };
        let input = request.validation_input();
        assert_eq!(input.pool, request.pool);
        assert_eq!(input.mint, request.mint);
        assert_eq!(input.mint.as_ptr(), request.mint.as_ptr());
        assert_eq!(input.pool_id, request.pool_id.as_deref());
        assert_eq!(
            input.pool_id.unwrap().as_ptr(),
            request.pool_id.as_ref().unwrap().as_ptr()
        );
        assert_eq!(input.quote_mint, request.quote_mint.as_deref());
        assert_eq!(
            input.quote_mint.unwrap().as_ptr(),
            request.quote_mint.as_ref().unwrap().as_ptr()
        );
        let result = ValidatedWarmup {
            mint: Address::from_bytes([1; 32]),
            quote_mint: Address::from_bytes([2; 32]),
            pool_id: Some(Address::from_bytes([3; 32])),
        };
        let hint = request.to_route_hint(&result);
        assert_eq!(hint.pool, request.pool);
        assert_eq!(hint.mint, result.mint);
        assert_eq!(hint.quote_mint, result.quote_mint);
        assert_eq!(hint.pool_id, result.pool_id);
        let absent = WarmupRequest {
            pool: Pool::Pump,
            mint: "mint".into(),
            pool_id: None,
            quote_mint: None,
        };
        let input = absent.validation_input();
        assert!(input.pool_id.is_none());
        assert!(input.quote_mint.is_none());
        let result = ValidatedWarmup {
            pool_id: None,
            ..result
        };
        assert!(absent.to_route_hint(&result).pool_id.is_none());
    }
}
