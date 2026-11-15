use super::*;
use axum::{body::to_bytes, http::HeaderValue};
use serde_json::json;

const TRADE_TOKEN: &str = "access-policy-trade-fixture";
const ADMIN_TOKEN: &str = "access-policy-admin-fixture";

fn headers(value: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(value) = value {
        headers.insert("authorization", value.parse().unwrap());
    }
    headers
}

async fn denied(result: Result<(), Response>, status: StatusCode, message: &str) {
    let response = result.unwrap_err();
    assert_eq!(response.status(), status);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        json!({"error":message})
    );
}

#[tokio::test]
async fn api_access_policy_trade_preserves_ip_before_token_and_bearer_parsing() {
    let allowed = [
        "198.51.100.1".parse().unwrap(),
        "2001:db8::1".parse().unwrap(),
    ];
    let mode = WalletAdminAuth::Legacy;
    let policy = ApiAccessPolicy {
        trade_token: TRADE_TOKEN,
        allowed_client_ips: &allowed,
        wallet_admin: &mode,
    };
    for (peer, permitted) in [
        ("127.0.0.1:1234", true),
        ("[::1]:1234", true),
        ("198.51.100.1:1234", true),
        ("[2001:db8::1]:1234", true),
        ("198.51.100.2:1234", false),
        ("[2001:db8::2]:1234", false),
        ("[::ffff:127.0.0.1]:1234", false),
    ] {
        for (header, correct) in [
            (Some("Bearer access-policy-trade-fixture"), true),
            (Some("bearer   access-policy-trade-fixture"), true),
            (Some("Bearer wrong"), false),
            (Some("Basic access-policy-trade-fixture"), false),
            (Some("Bearer access-policy-trade-fixture extra"), false),
            (None, false),
        ] {
            let result = authorize_trade(policy, &headers(header), peer.parse().unwrap());
            if !permitted {
                denied(result, StatusCode::FORBIDDEN, "client ip not allowed").await;
            } else if !correct {
                denied(result, StatusCode::UNAUTHORIZED, "unauthorized").await;
            } else {
                assert!(result.is_ok());
            }
        }
    }
    let mut binary = HeaderMap::new();
    binary.insert(
        "authorization",
        HeaderValue::from_bytes(b"Bearer \xff").unwrap(),
    );
    denied(
        authorize_trade(policy, &binary, "127.0.0.1:1".parse().unwrap()),
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
    let empty = ApiAccessPolicy {
        trade_token: " \t ",
        ..policy
    };
    denied(
        authorize_trade(
            empty,
            &headers(Some("Bearer access-policy-trade-fixture")),
            "127.0.0.1:1".parse().unwrap(),
        ),
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn api_access_policy_internal_keeps_loopback_only_even_for_allowlisted_clients() {
    let allowed = ["198.51.100.1".parse().unwrap()];
    let mode = WalletAdminAuth::Legacy;
    let policy = ApiAccessPolicy {
        trade_token: TRADE_TOKEN,
        allowed_client_ips: &allowed,
        wallet_admin: &mode,
    };
    for peer in ["127.0.0.1:1234", "[::1]:1234"] {
        assert!(authorize_internal(
            policy,
            &headers(Some("Bearer access-policy-trade-fixture")),
            peer.parse().unwrap()
        )
        .is_ok());
        denied(
            authorize_internal(policy, &HeaderMap::new(), peer.parse().unwrap()),
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
    let mut spoofed = headers(Some("Bearer access-policy-trade-fixture"));
    spoofed.insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
    for peer in ["198.51.100.1:1234", "198.51.100.2:1234"] {
        for headers in [&spoofed, &HeaderMap::new()] {
            denied(
                authorize_internal(policy, headers, peer.parse().unwrap()),
                StatusCode::FORBIDDEN,
                "internal wallet API is loopback-only",
            )
            .await;
        }
    }
}

#[tokio::test]
async fn api_access_policy_admin_keeps_legacy_and_separate_credentials_and_error_order() {
    let allowed = ["198.51.100.1".parse().unwrap()];
    let legacy = WalletAdminAuth::Legacy;
    let separate = WalletAdminAuth::Separate {
        token: ADMIN_TOKEN.into(),
    };
    let policy = ApiAccessPolicy {
        trade_token: TRADE_TOKEN,
        allowed_client_ips: &allowed,
        wallet_admin: &legacy,
    };
    let trade = headers(Some("Bearer access-policy-trade-fixture"));
    let admin = headers(Some("Bearer access-policy-admin-fixture"));
    let remote = "198.51.100.1:1234".parse().unwrap();
    assert!(authorize_wallet_admin(policy, &trade, remote, false).is_ok());
    denied(
        authorize_wallet_admin(policy, &admin, remote, false),
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
    denied(
        authorize_wallet_admin(policy, &trade, remote, true),
        StatusCode::FORBIDDEN,
        "internal wallet API is loopback-only",
    )
    .await;
    let policy = ApiAccessPolicy {
        wallet_admin: &separate,
        ..policy
    };
    for internal in [false, true] {
        for peer in ["127.0.0.1:1234", "[::1]:1234"] {
            assert!(
                authorize_wallet_admin(policy, &admin, peer.parse().unwrap(), internal).is_ok()
            );
            for denied_headers in [&trade, &HeaderMap::new()] {
                denied(
                    authorize_wallet_admin(policy, denied_headers, peer.parse().unwrap(), internal),
                    StatusCode::UNAUTHORIZED,
                    "wallet admin authorization required",
                )
                .await;
            }
        }
        for denied_headers in [&trade, &admin, &HeaderMap::new()] {
            denied(
                authorize_wallet_admin(policy, denied_headers, remote, internal),
                StatusCode::FORBIDDEN,
                "wallet admin API is loopback-only",
            )
            .await;
        }
    }
}
