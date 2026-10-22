//! Admin API under `/admin`: its own router, guarded as a whole by the Admin
//! Token middleware (ADR 0001). Admins refer to Ballots by internal ID.

mod auth;
mod ballots;
mod token_files;

use axum::Router;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::routing::{any, get, post};

use crate::AppState;

pub(crate) use token_files::load as load_token_files;

/// Every path under `/admin`, including unknown ones, sits behind the token
/// check, so the admin routes can't even be probed without a valid token.
pub(crate) fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/admin/ballots", post(ballots::create).get(ballots::list))
        .route(
            "/admin/ballots/{id}",
            get(ballots::get_one)
                .patch(ballots::edit)
                .delete(ballots::delete),
        )
        .route("/admin/ballots/{id}/extend", post(ballots::extend))
        .route("/admin", any(not_found))
        .route("/admin/", any(not_found))
        .route("/admin/{*rest}", any(not_found))
        .layer(from_fn_with_state(state, auth::require_admin_token))
}

async fn not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}
