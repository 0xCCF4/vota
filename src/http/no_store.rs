//! `Cache-Control: no-store` on every response of the voter surfaces (the
//! voting page and the voter API): they carry a Ballot, its voter's
//! Response and its Ballot Token, so no cache may keep them, whatever the
//! status.

use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

pub(crate) const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");

/// Middleware: marks the response as never to be stored.
pub(crate) async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, NO_STORE);
    response
}
