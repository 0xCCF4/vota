//! The security headers of the neutral error responses shared by the voter
//! API and the voting page: the uniform not-found (404), too many requests
//! (429), and the internal error (500: the neutral error page, or the voter
//! and admin APIs' `{"error":"internal"}`).
//!
//! They are never stored by a cache (`Cache-Control: no-store`, as every
//! response of the voter surfaces).
//!
//! They are static, so their policy is strict: nothing may load or run, no
//! scripts at all; only the error pages' own inline `<style>`, allowed by
//! its hash. The headers are the same on every such response, so the
//! uniform not-found stays byte-identical whatever the token was.

use std::sync::LazyLock;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest as _, Sha256};

/// The not-found page: static, neutral and unthemed, so it never depends on
/// the request (or on any Ballot's Theme).
pub(crate) const NOT_FOUND_PAGE: &str = include_str!("../../templates/not_found.html");

/// The page for anything that goes wrong on our side: static, neutral and
/// unthemed like the not-found page, never showing the error.
pub(crate) const SERVER_ERROR_PAGE: &str = include_str!("../../templates/server_error.html");

/// Every static error page, whose inline styles the policy allows.
const PAGES: [&str; 2] = [NOT_FOUND_PAGE, SERVER_ERROR_PAGE];

/// `default-src 'none'`, plus the hashes of the error pages' inline styles.
static CSP: LazyLock<HeaderValue> = LazyLock::new(|| {
    let mut hashes: Vec<String> = PAGES.iter().flat_map(|page| style_hashes(page)).collect();
    hashes.sort();
    hashes.dedup();
    let styles = hashes
        .iter()
        .map(|hash| format!("'sha256-{hash}'"))
        .collect::<Vec<_>>()
        .join(" ");
    let csp = format!(
        "default-src 'none'; style-src {styles}; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'none'"
    );
    HeaderValue::from_str(&csp).expect("a CSP of hashes is a valid header value")
});

/// The CSP hashes (base64 SHA-256) of the `<style>` elements' contents.
fn style_hashes(page: &str) -> Vec<String> {
    page.split("<style>")
        .skip(1)
        .filter_map(|rest| rest.split_once("</style>"))
        .map(|(style, _)| STANDARD.encode(Sha256::digest(style.as_bytes())))
        .collect()
}

/// A static error page with the error headers.
pub(crate) fn page(status: StatusCode, html: &'static str) -> Response {
    harden((status, Html(html)).into_response())
}

/// Adds the error headers to an error response.
pub(crate) fn harden(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, CSP.clone());
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, super::no_store::NO_STORE);
    response
}
