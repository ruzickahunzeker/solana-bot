use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::models::ErrorResponse;

pub(crate) fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}
