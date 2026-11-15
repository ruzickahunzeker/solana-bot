use axum::Router;

use crate::{api::routes, AppState};

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(routes::system_routes())
        .merge(routes::wallet_routes())
        .merge(routes::pool_routes())
        .merge(routes::quote_routes())
        .merge(routes::wsol_conversion_routes())
        .merge(routes::trade_routes())
        .merge(routes::wallet_balance_routes())
        .merge(routes::wallet_vanity_routes())
        .merge(routes::sol_transfer_routes())
        .with_state(state)
}
