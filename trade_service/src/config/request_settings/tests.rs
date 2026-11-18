use super::*;
use std::sync::Arc;

#[tokio::test]
async fn request_settings_outlive_full_config_without_borrowing_its_secrets() {
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.local_trade_api_token = "fixture-request-trade".into();
    let ips = [
        "192.0.2.17".parse::<IpAddr>().unwrap(),
        "2001:db8::19".parse().unwrap(),
    ];
    config.trade_allowed_client_ips = ips.to_vec();
    config.security.wallet_admin = WalletAdminAuth::Separate {
        token: "fixture-request-admin".into(),
    };
    let config = Arc::new(config);
    let original = Arc::downgrade(&config);
    let settings = Arc::new(RequestSettings::from_config(&config));
    drop(config);
    assert!(original.upgrade().is_none());
    for settings in [settings.clone(), settings] {
        let policy = settings.api_access_policy();
        assert_eq!(policy.trade_token, "fixture-request-trade");
        assert_eq!(policy.allowed_client_ips, ips);
        assert!(
            matches!(policy.wallet_admin, WalletAdminAuth::Separate { token } if token == "fixture-request-admin")
        );
        assert!(policy.trade_client_ip_allowed("127.0.0.1".parse().unwrap()));
        assert!(policy.trade_client_ip_allowed(ips[0]));
        assert!(!policy.trade_client_ip_allowed("192.0.2.18".parse().unwrap()));
    }
}

#[tokio::test]
async fn request_settings_project_all_existing_policy_values_without_coupling_feature_flags() {
    let state = crate::http_contract_tests::state().await;
    let mut config = (*state.config).clone();
    config.default_buy_sol = 0.013;
    config.default_slippage_percent = 4.25;
    config.default_tip_sol = 0.0009;
    config.stonkfun_via_sol.max_buy_sol = 0.25;
    config.pumpapi.default_priority_fee_sol = 0.0007;
    config.trade_response_wait_ms = 132;
    config.blockhash_max_age_ms = 321;
    config.mev_submit_verify_ms = 412;
    for bits in 0..64 {
        let has = |bit| (bits & (1 << bit)) != 0;
        config.stonkfun_via_sol.enabled = has(0);
        config.enable_simulate = has(1);
        config.allow_internal_system_wallet_transfer = has(2);
        config.conversion_live_enabled = has(3);
        config.wait_tx_confirmed = has(4);
        config.security.broadcast = if has(5) {
            BroadcastPolicy::Disabled
        } else {
            BroadcastPolicy::Enabled
        };
        let settings = RequestSettings::from_config(&config);
        let intake = settings.intake();
        let execution = settings.execution();
        assert_eq!(settings.trade_response_wait_ms(), 132);
        assert_eq!(settings.stonkfun_via_sol_enabled(), has(0));
        assert_eq!(intake.validation.default_buy_sol, 0.013);
        assert_eq!(intake.validation.default_slippage_percent, 4.25);
        assert_eq!(intake.validation.default_tip_sol, 0.0009);
        assert_eq!(intake.validation.via_sol_enabled, has(0));
        assert_eq!(intake.validation.via_sol_max_buy_sol, 0.25);
        assert_eq!(intake.command.default_simulate, has(1));
        assert_eq!(intake.command.pumpapi_priority_fee_sol, 0.0007);
        assert_eq!(intake.broadcast, config.security.broadcast);
        assert_eq!(execution.broadcast, config.security.broadcast);
        assert_eq!(execution.blockhash_max_age_ms, 321);
        assert_eq!(execution.mev_submit_verify_ms, 412);
        assert_eq!(execution.wait_tx_confirmed, has(4));
        assert_eq!(
            settings.sol_transfer_policy().broadcast,
            config.security.broadcast
        );
        assert_eq!(settings.sol_transfer_policy().allow_system_wallet, has(2));
        assert_eq!(
            settings.wsol_submit_policy().broadcast,
            config.security.broadcast
        );
        assert_eq!(settings.wsol_submit_policy().live_enabled, has(3));
    }
}
