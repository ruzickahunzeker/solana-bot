use axum::{
    routing::{get, patch, post},
    Router,
};

use crate::{
    api::handlers::{
        conversions, pools, quotes, system, trades, transfers, vanity, wallet_balances, wallets,
    },
    AppState,
};

pub(crate) fn system_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(system::health))
        .route("/ready", get(system::ready))
        .route("/v1/capabilities", get(system::capabilities))
}

pub(crate) fn wallet_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/wallets",
            get(wallets::wallets).post(wallets::create_wallet),
        )
        .route("/v1/wallets/manage", get(wallets::managed_wallets))
        .route(
            "/v1/wallets/:wallet_id",
            patch(wallets::update_wallet).delete(wallets::delete_wallet),
        )
        .route(
            "/v1/wallets/:wallet_id/export",
            post(wallets::export_wallet),
        )
}

pub(crate) fn wallet_balance_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/internal/wallet-balances",
            get(wallet_balances::wallet_balances),
        )
        .route(
            "/internal/wallets/:wallet_id/quote-balance/:mint",
            get(wallet_balances::quote_balance),
        )
}

pub(crate) fn wallet_vanity_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/internal/wallets/vanity-jobs",
            post(vanity::create_vanity_job),
        )
        .route(
            "/internal/wallets/vanity-jobs/:job_id",
            get(vanity::get_vanity_job).delete(vanity::cancel_vanity_job),
        )
}

pub(crate) fn pool_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/cache/warmup", post(pools::warmup))
        .route("/v1/pools/resolve", post(pools::resolve_pool))
        .route("/v1/pools/sol-hop", post(pools::resolve_sol_hop))
}

pub(crate) fn trade_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/trades",
            post(trades::create_trade).get(trades::list_trades),
        )
        .route("/v1/batch/trades", post(trades::create_batch_trades))
        .route("/v1/trades/:request_id", get(trades::get_trade))
}

pub(crate) fn quote_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/quotes/sol-buy", post(quotes::sol_buy_quote))
        .route(
            "/v1/quotes/sol-buy/:quote_id",
            get(quotes::get_sol_buy_quote),
        )
        .route(
            "/v1/quotes/sol-buy/:quote_id/preflight",
            post(quotes::preflight_sol_buy_quote),
        )
        .route(
            "/v1/quotes/sol-buy/:quote_id/submit",
            post(quotes::submit_sol_buy_quote),
        )
        .route(
            "/v1/quotes/sol-buy/:quote_id/submission",
            get(quotes::reconcile_sol_buy_submission),
        )
}

pub(crate) fn sol_transfer_routes() -> Router<AppState> {
    Router::new().route(
        "/internal/wallets/:wallet_id/transfer-sol",
        post(transfers::transfer_sol),
    )
}

pub(crate) fn wsol_conversion_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/internal/conversions/wsol/quotes",
            post(conversions::wsol_quote),
        )
        .route(
            "/internal/conversions/wsol/simulate",
            post(conversions::wsol_simulate),
        )
        .route(
            "/internal/conversions/wsol/submit",
            post(conversions::wsol_submit),
        )
        .route(
            "/internal/conversions/wsol/submissions/by-key",
            get(conversions::wsol_recover),
        )
        .route(
            "/internal/conversions/wsol/submissions/:submission_id",
            get(conversions::wsol_status),
        )
}
