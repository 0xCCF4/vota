//! JSON HTTP handlers: the admin API (`/admin`, ADR 0001) and the voter API
//! (`/api/b/{token}`). No SQL here; persistence goes through [`crate::data`].

pub(crate) mod admin;
mod ballot_json;
mod conflict;
pub(crate) mod voter;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::json;

use crate::http::error_page;

/// RFC 3339 in UTC with a `Z` suffix; fractional seconds only when present.
pub(crate) fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// Logs an internal error (never request data: no Admin Token, Ballot Token
/// or Voter Reference) and answers 500 `{"error":"internal"}`, without
/// details, with the security headers of the voter API's not-found. Shared
/// by the admin API (handlers and the Admin Token check) and the voter API.
pub(crate) fn internal_error(context: &str, error: impl std::fmt::Display) -> Response {
    tracing::error!(%error, "{context}");
    error_page::harden(
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "internal" })),
        )
            .into_response(),
    )
}
