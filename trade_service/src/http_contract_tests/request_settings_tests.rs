use super::*;

#[tokio::test]
async fn request_settings_router_uses_its_own_projected_access_and_capability_values() {
    let mut state = state().await;
    let original = http::router(state.clone());
    state.edit_fixture_config(|config| {
        config.local_trade_api_token = "fixture-req-v2".into();
        config.trade_allowed_client_ips = vec!["192.0.2.31".parse().unwrap()];
        config.stonkfun_via_sol.enabled = true;
    });
    let updated = http::router(state);
    let path = "/v1/capabilities";
    let old_token = Some(TOKEN);
    let new_token = Some("Bearer fixture-req-v2");
    for (app, good, bad, via_sol) in [
        (&original, old_token, new_token, false),
        (&updated, new_token, old_token, true),
    ] {
        let before = crate::state::now_ms();
        let body = json_body(
            send(app, "GET", path, LOCAL, good, None).await,
            StatusCode::OK,
        )
        .await;
        let after = crate::state::now_ms();
        let generated = u128::from(body["generated_at_ms"].as_u64().unwrap());
        assert!((before..=after).contains(&generated));
        let mut expected =
            serde_json::to_value(crate::protocol_capabilities::current(via_sol)).unwrap();
        expected["generated_at_ms"] = body["generated_at_ms"].clone();
        assert_eq!(body, expected);
        error_body(
            send(app, "GET", path, LOCAL, bad, None).await,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        )
        .await;
    }
    assert_eq!(
        send(&updated, "GET", path, "192.0.2.31:12345", new_token, None)
            .await
            .status(),
        StatusCode::OK
    );
    error_body(
        send(&updated, "GET", path, ALLOWED, None, None).await,
        StatusCode::FORBIDDEN,
        "client ip not allowed",
    )
    .await;
    assert_eq!(
        send(&original, "GET", path, ALLOWED, old_token, None)
            .await
            .status(),
        StatusCode::OK
    );
}
