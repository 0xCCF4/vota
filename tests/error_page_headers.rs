//! The uniform not-found (404), too-many-requests (429) and internal error
//! (500) responses of the voting page and the voter API, and the admin API's
//! internal error (500), carry security headers: a strict Content Security
//! Policy without scripts, `nosniff` and no referrer. The database is broken
//! through the filesystem; `database_write_failure.rs` covers the admin
//! API's 500s past the Admin Token check.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, assert_hardened, ballot_request, body_json, write_file};
use serde_json::json;

/// Well-formed, but never issued.
const UNKNOWN: &str = "AAAAAAAAAAAAAAAAAAAAAA";

#[tokio::test]
async fn the_not_found_page_carries_security_headers() {
    let app = TestApp::new().await;

    let response = app.get(&format!("/b/{UNKNOWN}")).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_hardened(&response, "voting page 404");
}

#[tokio::test]
async fn the_voter_api_not_found_carries_security_headers() {
    let app = TestApp::new().await;

    let response = app.get(&format!("/api/b/{UNKNOWN}")).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_hardened(&response, "voter API 404");
}

#[tokio::test]
async fn too_many_requests_carries_security_headers_on_both_surfaces() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = 1.try_into().unwrap();
    })
    .await;
    app.get_from("192.0.2.1", None, &format!("/b/{UNKNOWN}"))
        .await;

    for prefix in ["/b/", "/api/b/"] {
        let response = app
            .get_from("192.0.2.1", None, &format!("{prefix}{UNKNOWN}"))
            .await;

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "{prefix}");
        assert_hardened(&response, prefix);
    }
}

#[tokio::test]
async fn the_neutral_error_page_carries_security_headers() {
    // A folder Theme that fails to render only for a title the startup
    // trial render doesn't use.
    let app = TestApp::with_config(|config| {
        let folder = config.theme_folder.clone().unwrap();
        write_file(&folder, "fragile/style.css", "");
        write_file(
            &folder,
            "fragile/templates/ballot.html",
            r#"{% extends "base.html" %}{% block content %}
{% if title == "Explode" %}{{ no_such_function() }}{% endif %}
<form method="post"></form>{% endblock %}"#,
        );
    })
    .await;
    let mut request = ballot_request();
    request["title"] = json!("Explode");
    request["theme"] = json!("fragile");
    let token = app.create_ballot(&request).await["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let response = app.get(&format!("/b/{token}")).await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_hardened(&response, "500 page");
}

#[tokio::test]
async fn a_voter_api_internal_error_carries_security_headers_and_no_details() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    // A broken database: every lookup fails on our side.
    app.wipe_database_files();

    let get = app.get(&format!("/api/b/{token}")).await;
    let put = app
        .put_json(
            &format!("/api/b/{token}/response"),
            // The revision the voter saw before the database broke.
            &json!({ "option_ids": [options[0]], "revision": 0 }),
        )
        .await;

    for (what, response) in [("GET", get), ("PUT", put)] {
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{what}"
        );
        assert_hardened(&response, what);
        assert_eq!(
            body_json(response).await,
            json!({ "error": "internal" }),
            "{what}"
        );
    }
}

#[tokio::test]
async fn an_admin_token_check_that_fails_internally_carries_security_headers_and_no_details() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    // A broken database: the Admin Token check itself fails.
    app.wipe_database_files();

    let response = app.admin_get("/admin/ballots", &admin).await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_hardened(&response, "admin token check");
    assert_eq!(body_json(response).await, json!({ "error": "internal" }));
}
