//! Headers every response carries (security audit I-13 and I-15):
//! `X-Content-Type-Options: nosniff` on all of them, whatever their type
//! and wherever they come from (handlers, the router, the body limit), and
//! on HTML also `X-Frame-Options`, `Permissions-Policy`,
//! `Cross-Origin-Opener-Policy` and `Cross-Origin-Resource-Policy`.

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use common::{TestApp, from_peer};

fn assert_nosniff(response: &Response<Body>, what: &str) {
    assert_eq!(
        response.headers().get(header::X_CONTENT_TYPE_OPTIONS),
        Some(&"nosniff".parse().unwrap()),
        "{what}: {:?}",
        response.headers()
    );
}

#[tokio::test]
async fn every_kind_of_response_says_nosniff() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let (token, _) = app.create_standard_ballot().await;
    let too_large = "option=".to_owned() + &"x".repeat(70 * 1024);

    let cases = [
        ("voter API JSON", app.get(&format!("/api/b/{token}")).await),
        (
            "voter API 422",
            app.put_raw(
                &format!("/api/b/{token}/response"),
                "application/json",
                "{}",
            )
            .await,
        ),
        (
            "admin API JSON",
            app.admin_get("/admin/ballots", &admin).await,
        ),
        ("admin 401", app.get("/admin/ballots").await),
        (
            "empty 405",
            app.send(
                Request::delete(format!("/api/b/{token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await,
        ),
        (
            "empty 415",
            app.put_raw(&format!("/api/b/{token}/response"), "text/plain", "x")
                .await,
        ),
        (
            "413 text",
            app.post_raw(
                &format!("/b/{token}"),
                None,
                "application/x-www-form-urlencoded",
                too_large,
            )
            .await,
        ),
        (
            "/healthz",
            app.get_from("127.0.0.1", None, "/healthz").await,
        ),
        ("plain 404", app.get("/no-such-path").await),
    ];

    for (what, response) in cases {
        assert_nosniff(&response, what);
    }
}

/// Asserts the headers every HTML response carries besides its CSP.
fn assert_html_headers(response: &Response<Body>, what: &str) {
    let headers = response.headers();
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html"),
        "{what}"
    );
    assert_nosniff(response, what);
    assert_eq!(headers["x-frame-options"], "DENY", "{what}");
    assert_eq!(
        headers["cross-origin-opener-policy"], "same-origin",
        "{what}"
    );
    assert_eq!(
        headers["cross-origin-resource-policy"], "same-origin",
        "{what}"
    );
    let permissions = headers["permissions-policy"].to_str().unwrap();
    for feature in ["camera=()", "microphone=()", "geolocation=()", "payment=()"] {
        assert!(permissions.contains(feature), "{what}: {permissions}");
    }
}

#[tokio::test]
async fn html_responses_also_refuse_framing_powerful_features_and_cross_origin_use() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = 1.try_into().unwrap();
    })
    .await;
    let (token, options) = app.create_standard_ballot().await;

    let page = app.get(&format!("/b/{token}")).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert_html_headers(&page, "voting page");
    let confirm = app
        .post_form(&format!("/b/{token}"), &format!("option={}", options[0]))
        .await;
    assert_eq!(confirm.status(), StatusCode::OK);
    assert_html_headers(&confirm, "confirmation page");
    let unknown = "A".repeat(22);
    let not_found = app
        .send(from_peer(
            "192.0.2.1",
            Request::get(format!("/b/{unknown}"))
                .body(Body::empty())
                .unwrap(),
        ))
        .await;
    assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
    assert_html_headers(&not_found, "not-found page");
    let too_many = app
        .get_from("192.0.2.1", None, &format!("/b/{token}"))
        .await;
    assert_eq!(too_many.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_html_headers(&too_many, "too-many-requests page");
}

#[tokio::test]
async fn json_responses_get_only_nosniff_of_the_html_headers() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;

    let response = app.get(&format!("/api/b/{token}")).await;

    assert_nosniff(&response, "voter API");
    for name in [
        "x-frame-options",
        "permissions-policy",
        "cross-origin-opener-policy",
    ] {
        assert!(!response.headers().contains_key(name), "{name}");
    }
}
