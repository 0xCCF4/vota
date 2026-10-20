use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// A rejected input: 422 with the offending field (a JSON path such as
/// `options[1].label`, empty for the body as a whole) and a fixed reason. The
/// rejected value itself is never echoed, and neither is a name the request
/// made up: only field and parameter names the schema knows are reported;
/// an unknown one is `null` ([`InputError::unknown_field`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError {
    field: Option<String>,
    message: String,
}

/// The reason given for a field or parameter the schema doesn't have.
const UNKNOWN_FIELD: &str = "unknown field";

impl InputError {
    /// An error at `field`, a path of names the schema knows.
    pub(super) fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: Some(field.into()),
            message: message.into(),
        }
    }

    /// An error at a name the request chose, reported without it.
    pub(super) fn unnamed(message: impl Into<String>) -> Self {
        Self {
            field: None,
            message: message.into(),
        }
    }

    /// A field or parameter the schema doesn't have, unnamed.
    pub(super) fn unknown_field() -> Self {
        Self::unnamed(UNKNOWN_FIELD)
    }

    /// Whether this is an [`InputError::unknown_field`].
    pub(super) fn is_unknown_field(&self) -> bool {
        self.field.is_none() && self.message == UNKNOWN_FIELD
    }
}

impl IntoResponse for InputError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": "invalid_input",
            "field": self.field,
            "message": self.message,
        });
        (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
    }
}
