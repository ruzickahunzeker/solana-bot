use super::*;

#[tokio::test]
async fn standalone_quote_route_group_keeps_integrated_paths_methods_auth_and_missing_wallet_errors(
) {
    let state = state().await;
    let group = crate::api::routes::quote_routes().with_state(state.clone());
    let integrated = http::router(state.clone());
    let cases = [
        (
            "POST",
            "GET",
            "/v1/quotes/sol-buy",
            Some(json!({
                "wallet_id":"missing", "mint":"11111111111111111111111111111111",
                "amount_lamports":1_000_000, "slippage_bps":1500,
            })),
        ),
        (
            "GET",
            "POST",
            "/v1/quotes/sol-buy/missing?wallet_id=missing",
            None,
        ),
        (
            "POST",
            "GET",
            "/v1/quotes/sol-buy/missing/preflight?wallet_id=missing",
            None,
        ),
        (
            "POST",
            "GET",
            "/v1/quotes/sol-buy/missing/submit",
            Some(
                json!({"wallet_id":"missing", "idempotency_key":"key", "confirmed":true, "simulate":true}),
            ),
        ),
        (
            "GET",
            "POST",
            "/v1/quotes/sol-buy/missing/submission?wallet_id=missing&idempotency_key=key",
            None,
        ),
    ];
    for app in [&group, &integrated] {
        for (method, wrong_method, path, request_body) in &cases {
            assert_eq!(
                send(app, method, path, LOCAL, None, request_body.clone())
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED,
            );
            assert_eq!(
                send(app, method, path, DENIED, Some(TOKEN), request_body.clone())
                    .await
                    .status(),
                StatusCode::FORBIDDEN,
            );
            error_body(
                send(app, method, path, LOCAL, Some(TOKEN), request_body.clone()).await,
                StatusCode::BAD_REQUEST,
                "wallet_id not found",
            )
            .await;
            assert_eq!(
                send(
                    app,
                    wrong_method,
                    path,
                    LOCAL,
                    Some(TOKEN),
                    request_body.clone()
                )
                .await
                .status(),
                StatusCode::METHOD_NOT_ALLOWED,
            );
        }
        assert_eq!(
            send(app, "GET", "/v1/quotes/unknown", LOCAL, Some(TOKEN), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert!(state.trades.recent(10).is_empty());
    assert_eq!(state.wallets.wallet_count(), 0);
}
