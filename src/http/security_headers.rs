//! Headers on every response of the app, whatever produced it (a handler,
//! the router's 404 and 405, the body limit's 413):
//!
//! - `X-Content-Type-Options: nosniff` on all of them, so no browser reads
//!   a JSON or plain-text body as anything else;
//! - on HTML responses also `X-Frame-Options: DENY` (for browsers that
//!   predate CSP's `frame-ancestors`), a `Permissions-Policy` that turns
//!   off every powerful feature, and `Cross-Origin-Opener-Policy` and
//!   `Cross-Origin-Resource-Policy: same-origin`.
//!
//! Each response's own headers (e.g. its Content Security Policy) stay as
//! they are. The headers are fixed, so the uniform not-found stays
//! byte-identical whatever the token was.

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

/// Features no page of the app uses.
const PERMISSIONS_POLICY: &str = "accelerometer=(), autoplay=(), camera=(), \
     display-capture=(), encrypted-media=(), fullscreen=(), geolocation=(), \
     gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), \
     picture-in-picture=(), publickey-credentials-get=(), screen-wake-lock=(), \
     usb=(), xr-spatial-tracking=()";

/// Middleware: adds the headers to the response.
pub(crate) async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    let is_html = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim_start().starts_with("text/html"));
    if is_html {
        for (name, value) in [
            (header::X_FRAME_OPTIONS, "DENY"),
            (
                HeaderName::from_static("permissions-policy"),
                PERMISSIONS_POLICY,
            ),
            (
                HeaderName::from_static("cross-origin-opener-policy"),
                "same-origin",
            ),
            (
                HeaderName::from_static("cross-origin-resource-policy"),
                "same-origin",
            ),
        ] {
            headers.insert(name, HeaderValue::from_static(value));
        }
    }
    response
}
