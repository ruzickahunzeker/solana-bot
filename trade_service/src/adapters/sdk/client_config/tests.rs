use super::*;
use sol_trade_sdk::{
    common::{InfrastructureConfig, TradeTransactionVersion},
    swqos::{SwqosRegion, SwqosTransport},
};

const RPC: &str = "https://rpc.example.invalid/?api-key=fixture-a";

fn temporal(token: &str) -> SwqosConfig {
    SwqosConfig::Temporal(
        token.into(),
        SwqosRegion::Frankfurt,
        Some("https://relay.example.invalid".into()),
        Some(0.0001),
    )
}

fn old(
    rpc: String,
    provider: SwqosConfig,
    commitment: CommitmentConfig,
    log: bool,
    mev: bool,
) -> TradeConfig {
    TradeConfig::builder(rpc, vec![provider], commitment)
        .create_wsol_ata_on_startup(false)
        .use_seed_optimize(false)
        .log_enabled(log)
        .mev_protection(mev)
        .build()
}

#[test]
fn sdk_lane_config_preserves_original_default_and_provider_construction_for_all_settings() {
    let mut cases = 0;
    for provider in [SwqosConfig::Default(RPC.into()), temporal("fixture-a")] {
        for commitment in [
            CommitmentConfig::processed(),
            CommitmentConfig::confirmed(),
            CommitmentConfig::finalized(),
        ] {
            for log in [false, true] {
                for mev in [false, true] {
                    let expected = old(RPC.into(), provider.clone(), commitment.clone(), log, mev);
                    let actual =
                        lane_config(RPC.into(), provider.clone(), commitment.clone(), log, mev);
                    assert!(
                        InfrastructureConfig::from_trade_config(&actual)
                            == InfrastructureConfig::from_trade_config(&expected)
                    );
                    assert_eq!(
                        actual.create_wsol_ata_on_startup,
                        expected.create_wsol_ata_on_startup
                    );
                    assert_eq!(actual.use_seed_optimize, expected.use_seed_optimize);
                    assert_eq!(actual.log_enabled, expected.log_enabled);
                    assert_eq!(actual.check_min_tip, expected.check_min_tip);
                    assert_eq!(actual.transaction_version, expected.transaction_version);
                    assert!(!actual.create_wsol_ata_on_startup && !actual.use_seed_optimize);
                    assert!(!actual.check_min_tip && !actual.swqos_cores_from_end);
                    assert_eq!(actual.transaction_version, TradeTransactionVersion::V0);
                    assert_eq!((actual.log_enabled, actual.mev_protection), (log, mev));
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 24);
}

#[test]
fn sdk_lane_config_full_infrastructure_identity_retains_auth_order_transport_and_protection() {
    let base = InfrastructureConfig::from_trade_config(&lane_config(
        RPC.into(),
        temporal("fixture-a"),
        CommitmentConfig::processed(),
        false,
        false,
    ));
    for change in 0..10 {
        let mut changed = base.clone();
        match change {
            0 => changed.rpc_url = "https://rpc.example.invalid/?api-key=fixture-b".into(),
            1 => changed.swqos_configs = vec![temporal("fixture-b")],
            2 => changed.commitment = CommitmentConfig::confirmed(),
            3 => changed.mev_protection = true,
            4 => changed.swqos_cores_from_end = true,
            5 => changed.swqos_configs.push(SwqosConfig::Default(RPC.into())),
            6 => {
                changed.swqos_configs = vec![SwqosConfig::Node1(
                    "fixture-a".into(),
                    SwqosRegion::Frankfurt,
                    None,
                    Some(SwqosTransport::Quic),
                    None,
                )]
            }
            7 => {
                changed.swqos_configs = vec![SwqosConfig::Temporal(
                    "fixture-a".into(),
                    SwqosRegion::Tokyo,
                    Some("https://relay.example.invalid".into()),
                    Some(0.0001),
                )]
            }
            8 => {
                changed.swqos_configs = vec![SwqosConfig::Temporal(
                    "fixture-a".into(),
                    SwqosRegion::Frankfurt,
                    Some("https://other-relay.example.invalid".into()),
                    Some(0.0001),
                )]
            }
            _ => {
                changed.swqos_configs = vec![SwqosConfig::Temporal(
                    "fixture-a".into(),
                    SwqosRegion::Frankfurt,
                    Some("https://relay.example.invalid".into()),
                    Some(0.0002),
                )]
            }
        }
        assert!(changed != base, "identity change{change} collapsed");
    }
    let mut ordered = base.clone();
    ordered.swqos_configs.push(SwqosConfig::Default(RPC.into()));
    let mut reversed = ordered.clone();
    reversed.swqos_configs.reverse();
    assert!(ordered != reversed);
    let http = InfrastructureConfig::from_trade_config(&lane_config(
        RPC.into(),
        SwqosConfig::Node1("fixture-a".into(), SwqosRegion::Frankfurt, None, None, None),
        CommitmentConfig::processed(),
        false,
        false,
    ));
    let quic = InfrastructureConfig::from_trade_config(&lane_config(
        RPC.into(),
        SwqosConfig::Node1(
            "fixture-a".into(),
            SwqosRegion::Frankfurt,
            None,
            Some(SwqosTransport::Quic),
            None,
        ),
        CommitmentConfig::processed(),
        false,
        false,
    ));
    assert!(http != quic);
}

#[test]
fn sdk_lane_config_separates_client_settings_and_owns_configuration_without_initializing_sdk() {
    let mut provider = temporal("fixture-a");
    let first = lane_config(
        RPC.into(),
        provider.clone(),
        CommitmentConfig::processed(),
        false,
        false,
    );
    let logged = lane_config(
        RPC.into(),
        provider.clone(),
        CommitmentConfig::processed(),
        true,
        false,
    );
    assert!(
        InfrastructureConfig::from_trade_config(&first)
            == InfrastructureConfig::from_trade_config(&logged)
    );
    assert_ne!(first.log_enabled, logged.log_enabled);
    if let SwqosConfig::Temporal(token, _, _, _) = &mut provider {
        token.clear();
    }
    assert!(first.swqos_configs == vec![temporal("fixture-a")]);
    assert_eq!(first.rpc_url, RPC);
    // Sharing only the infrastructure must not silently reset these per-client fields.
    let mut different = first.clone();
    different.check_min_tip = true;
    different.transaction_version = TradeTransactionVersion::V1;
    assert!(
        InfrastructureConfig::from_trade_config(&first)
            == InfrastructureConfig::from_trade_config(&different)
    );
    assert_ne!(first.check_min_tip, different.check_min_tip);
    assert_ne!(first.transaction_version, different.transaction_version);
}
