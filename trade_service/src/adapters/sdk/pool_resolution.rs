use crate::{
    adapters::rpc::{
        discovery_reads::{DiscoveryProtocol, DiscoveryReadSession},
        read_http::ReadHttpTransport,
        read_runtime::SharedReadRuntime,
    },
    domain::{
        route::PoolRouteHint,
        trade::{Address, Pool},
    },
    sanitize::redact_urls,
};
use anyhow::{anyhow, bail, Result};
use ring::rand::{SecureRandom, SystemRandom};
use sol_trade_sdk::{
    find_pool_by_mint,
    trading::{
        core::params::{MeteoraDbcParams, PumpFunParams, PumpSwapParams},
        factory::DexType,
    },
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Read configuration only; never retains a wallet, execution client or signer.
pub(crate) struct DiscoveryLane {
    url: String,
    commitment: CommitmentConfig,
    helius: bool,
}
impl DiscoveryLane {
    pub(crate) fn new(url: String, commitment: CommitmentConfig, helius: bool) -> Self {
        Self {
            url,
            commitment,
            helius,
        }
    }
}

pub(crate) fn lane_order(helius_lanes: &[bool], swap_helius: bool) -> Vec<usize> {
    let mut order = (0..helius_lanes.len()).collect::<Vec<_>>();
    let helius = helius_lanes
        .iter()
        .enumerate()
        .filter_map(|(index, is_helius)| is_helius.then_some(index))
        .take(2)
        .collect::<Vec<_>>();
    if helius.len() == 2 {
        order.retain(|index| !helius.contains(index));
        let first = usize::from(swap_helius);
        order.insert(0, helius[1 - first]);
        order.insert(0, helius[first]);
    }
    order
}

pub(crate) async fn resolve<T: ReadHttpTransport + 'static>(
    runtime: Arc<SharedReadRuntime<T>>,
    lanes: &[DiscoveryLane],
    mint: Address,
) -> Result<PoolRouteHint> {
    if lanes.is_empty() {
        bail!("no Default RPC lane configured for pool resolution");
    }
    let helius = lanes.iter().map(|lane| lane.helius).collect::<Vec<_>>();
    let mut random_byte = [0u8; 1];
    if helius.iter().filter(|&&is_helius| is_helius).count() >= 2 {
        SystemRandom::new()
            .fill(&mut random_byte)
            .map_err(|_| anyhow!("could not choose Helius RPC lane"))?;
    }
    resolve_ordered(
        runtime,
        lanes,
        mint,
        &lane_order(&helius, random_byte[0] & 1 != 0),
    )
    .await
}

pub(super) async fn resolve_ordered<T: ReadHttpTransport + 'static>(
    runtime: Arc<SharedReadRuntime<T>>,
    lanes: &[DiscoveryLane],
    mint: Address,
    order: &[usize],
) -> Result<PoolRouteHint> {
    let mut failures = Vec::with_capacity(lanes.len());
    for &lane_index in order {
        let deadline = Instant::now() + Duration::from_secs(3);
        let result = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            on_lane(runtime.clone(), &lanes[lane_index], mint, deadline),
        )
        .await;
        let failure = match result {
            Ok(Ok(resolved)) => return Ok(resolved),
            Ok(Err(failure)) => failure,
            Err(_) => "pool resolution timed out".into(),
        };
        let redacted = redact_urls(&failure);
        tracing::warn!(mint = %mint, rpc_lane = lane_index + 1, error = %redacted, "pool resolution RPC lane failed");
        failures.push(failure_kind(&redacted));
    }
    bail!(
        "pool resolution failed across {} RPC lanes: {}",
        failures.len(),
        failures.join(", ")
    )
}

pub(super) async fn on_lane<T: ReadHttpTransport + 'static>(
    runtime: Arc<SharedReadRuntime<T>>,
    lane: &DiscoveryLane,
    mint: Address,
    deadline: Instant,
) -> std::result::Result<PoolRouteHint, String> {
    let key = Pubkey::new_from_array(mint.to_bytes());
    let session = |protocol| {
        DiscoveryReadSession::new(
            runtime.clone(),
            lane.url.clone(),
            lane.commitment,
            protocol,
            mint,
            deadline,
        )
    };
    let pump = session(DiscoveryProtocol::Pump);
    let dbc = session(DiscoveryProtocol::Dbc);
    let (pump_result, dbc_result) = tokio::join!(
        async {
            let result = PumpFunParams::from_mint_by_rpc(pump.rpc(), &key).await;
            pump.finish(result)
        },
        async {
            let result = MeteoraDbcParams::from_base_mint_by_rpc(dbc.rpc(), &key).await;
            dbc.finish(result)
        },
    );
    if let Ok(params) = &pump_result {
        if !params.bonding_curve.complete {
            return Ok(PoolRouteHint {
                mint,
                pool: Pool::Pump,
                pool_id: None,
                quote_mint: Address::from_bytes(params.quote_mint.to_bytes()),
            });
        }
    }
    if let Ok(params) = &dbc_result {
        return Ok(PoolRouteHint {
            mint,
            pool: Pool::MeteoraDbc,
            pool_id: Some(Address::from_bytes(params.pool.to_bytes())),
            quote_mint: Address::from_bytes(params.quote_mint.to_bytes()),
        });
    }
    let dbc_error = dbc_result
        .err()
        .expect("successful DBC resolution returned above");
    let swap = session(DiscoveryProtocol::PumpSwap);
    let result = match find_pool_by_mint(swap.rpc(), &key, DexType::PumpSwap).await {
        Ok(pool) => PumpSwapParams::from_pool_address_by_rpc(swap.rpc(), &pool).await,
        Err(error) => Err(error),
    };
    match swap.finish(result) {
        Ok(params) => Ok(PoolRouteHint { mint, pool: Pool::PumpAmm, pool_id: Some(Address::from_bytes(params.pool.to_bytes())), quote_mint: Address::from_bytes(params.quote_mint.to_bytes()) }),
        Err(error) => match pump_result {
            Ok(_) => Err(format!("pump bonding curve is complete but migrated pools are unavailable: pump-amm={error}; meteora-dbc={dbc_error}")),
            Err(pump_error) => Err(format!("no supported pool found for mint; pump={pump_error}; pump-amm={error}; meteora-dbc={dbc_error}")),
        }
    }
}

pub(crate) fn failure_kind(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("429") || lower.contains("too many requests") {
        return "rate limited".into();
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return "timeout".into();
    }
    if lower.contains("accountnotfound")
        || lower.contains("account not found")
        || lower.contains("no pool found")
    {
        return "pool not found".into();
    }
    "RPC unavailable".into()
}
