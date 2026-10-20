//! Failed Admin Token checks are rate-limited per client IP (security audit
//! I-3), with their own budget (`rate_limit.admin_failed_auth` per
//! `rate_limit.admin_window`): once a client has spent it, every admin
//! request from it gets 429 until the window ends, without a token check.
//! Requests that pass the check never spend it.

#![cfg(feature = "server")]

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use common::{TestApp, assert_hardened, body_json, from_peer};
use serde_json::json;

const WRONG: &str = "vta_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

async fn app_with_admin_limit(failures: u32, window: Duration) -> TestApp {
    TestApp::with_config(|config| {
        config.rate_limit.admin_failed_auth = failures.try_into().unwrap();
        config.rate_limit.admin_window = window;
    })
    .await
}

/// `GET /admin/ballots` from the TCP peer `peer` with this Admin Token.
async fn list_from(app: &TestApp, peer: &str, token: &str) -> Response<Body> {
    let request = Request::get("/admin/ballots")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    app.send(from_peer(peer, request)).await
}

#[tokio::test]
async fn a_client_that_spent_its_budget_of_failed_admin_checks_gets_429_even_with_a_valid_token() {
    let app = app_with_admin_limit(3, Duration::from_secs(600)).await;
    let valid = app.admin_token().await;

    for _ in 0..3 {
        let response = list_from(&app, "192.0.2.1", WRONG).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let blocked = list_from(&app, "192.0.2.1", &valid).await;

    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(blocked.headers()[header::RETRY_AFTER], "600");
    assert_hardened(&blocked, "admin 429");
    assert_eq!(
        body_json(blocked).await,
        json!({ "error": "too_many_requests" })
    );
    let other_client = list_from(&app, "192.0.2.2", &valid).await;
    assert_eq!(other_client.status(), StatusCode::OK);
}

#[tokio::test]
async fn requests_that_pass_the_token_check_spend_nothing() {
    let app = app_with_admin_limit(2, Duration::from_secs(600)).await;
    let valid = app.admin_token().await;

    for _ in 0..10 {
        assert_eq!(
            list_from(&app, "192.0.2.1", &valid).await.status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        list_from(&app, "192.0.2.1", WRONG).await.status(),
        StatusCode::UNAUTHORIZED
    );

    assert_eq!(
        list_from(&app, "192.0.2.1", &valid).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_block_ends_with_the_admin_window() {
    let app = app_with_admin_limit(1, Duration::from_secs(60)).await;
    let valid = app.admin_token().await;
    list_from(&app, "192.0.2.1", WRONG).await;
    assert_eq!(
        list_from(&app, "192.0.2.1", &valid).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );

    app.clock.advance(chrono::Duration::seconds(60));

    assert_eq!(
        list_from(&app, "192.0.2.1", &valid).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_admin_budget_and_the_ballot_token_budget_are_separate() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = 1.try_into().unwrap();
        config.rate_limit.admin_failed_auth = 1.try_into().unwrap();
    })
    .await;
    let valid = app.admin_token().await;
    let unknown = "A".repeat(22);

    // A failed Ballot Token lookup doesn't spend the admin budget ...
    app.get_from("192.0.2.1", None, &format!("/api/b/{unknown}"))
        .await;
    assert_eq!(
        list_from(&app, "192.0.2.1", &valid).await.status(),
        StatusCode::OK
    );
    // ... and a failed admin check doesn't spend the lookup budget.
    list_from(&app, "192.0.2.2", WRONG).await;
    let lookup = app
        .get_from("192.0.2.2", None, &format!("/api/b/{unknown}"))
        .await;
    assert_eq!(lookup.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn behind_a_trusted_proxy_admin_clients_are_told_apart_by_x_forwarded_for() {
    let app = TestApp::with_config(|config| {
        config.trusted_proxies = vec!["10.0.0.0/8".parse().unwrap()];
        config.rate_limit.admin_failed_auth = 1.try_into().unwrap();
    })
    .await;
    let valid = app.admin_token().await;
    let via_proxy = |client: &str, token: &str| {
        let request = Request::get("/admin/ballots")
            .header("authorization", format!("Bearer {token}"))
            .header("x-forwarded-for", client)
            .body(Body::empty())
            .unwrap();
        from_peer("10.0.0.1", request)
    };

    app.send(via_proxy("203.0.113.5", WRONG)).await;
    let blocked = app.send(via_proxy("203.0.113.5", &valid)).await;
    let other = app.send(via_proxy("203.0.113.6", &valid)).await;
    let unreadable = app.send(via_proxy("junk", &valid)).await;

    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(other.status(), StatusCode::OK);
    assert_eq!(unreadable.status(), StatusCode::BAD_REQUEST);
    assert_hardened(&unreadable, "admin 400");
}
