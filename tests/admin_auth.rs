#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use chrono::Duration;
use common::{TestApp, ballot_request, body_string, start_time};
use serde_json::json;
use vota::data::admin_tokens::NewAdminToken;

#[tokio::test]
async fn creating_a_ballot_without_an_admin_token_is_refused() {
    let app = TestApp::new().await;

    let response = app.post_json("/admin/ballots", None, &json!({})).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

async fn create_with(app: &TestApp, token: Option<&str>) -> axum::http::Response<axum::body::Body> {
    app.post_json("/admin/ballots", token, &ballot_request())
        .await
}

#[tokio::test]
async fn a_valid_admin_token_grants_access() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;

    let response = create_with(&app, Some(&token)).await;

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn an_unknown_admin_token_is_refused() {
    let app = TestApp::new().await;
    app.admin_token().await;

    let response = create_with(
        &app,
        Some("vta_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_admin_token_is_accepted_until_its_expiry_and_refused_from_then_on() {
    let app = TestApp::new().await;
    let token = app
        .admin_token_with(NewAdminToken {
            label: Some("ci".into()),
            expires_at: Some(start_time() + Duration::hours(1)),
        })
        .await;

    app.clock.advance(Duration::minutes(59));
    assert_eq!(
        create_with(&app, Some(&token)).await.status(),
        StatusCode::CREATED
    );

    app.clock.advance(Duration::minutes(1));
    assert_eq!(
        create_with(&app, Some(&token)).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn missing_unknown_malformed_and_expired_tokens_get_identical_refusals() {
    let app = TestApp::new().await;
    let expired = app
        .admin_token_with(NewAdminToken {
            label: None,
            expires_at: Some(start_time() + Duration::seconds(1)),
        })
        .await;
    let valid = app.admin_token().await;
    app.clock.advance(Duration::seconds(1));

    let mut refusals = Vec::new();
    for request in [
        Request::post("/admin/ballots").body(Body::empty()).unwrap(),
        bearer("vta_unknown"),
        bearer(&expired),
        // Right secret, wrong scheme.
        Request::post("/admin/ballots")
            .header("authorization", format!("Basic {valid}"))
            .body(Body::empty())
            .unwrap(),
        Request::post("/admin/ballots")
            .header("authorization", "Bearer")
            .body(Body::empty())
            .unwrap(),
    ] {
        let response = app.send(request).await;
        let status = response.status();
        let headers = format!("{:?}", response.headers());
        refusals.push((status, headers, body_string(response).await));
    }

    assert_eq!(refusals[0].0, StatusCode::UNAUTHORIZED);
    assert_eq!(refusals[0].2, "");
    for refusal in &refusals[1..] {
        assert_eq!(refusal, &refusals[0]);
    }
}

fn bearer(token: &str) -> Request<Body> {
    Request::post("/admin/ballots")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(ballot_request().to_string()))
        .unwrap()
}

#[tokio::test]
async fn every_admin_path_requires_an_admin_token() {
    let app = TestApp::new().await;

    for path in [
        "/admin/ballots",
        "/admin/does-not-exist",
        "/admin/",
        "/admin",
    ] {
        let response = app.get(path).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[tokio::test]
async fn the_bearer_scheme_is_matched_in_any_case() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;

    for scheme in ["bearer", "BEARER", "bEaReR"] {
        let request = Request::get("/admin/ballots")
            .header("authorization", format!("{scheme} {token}"))
            .body(Body::empty())
            .unwrap();

        assert_eq!(app.send(request).await.status(), StatusCode::OK, "{scheme}");
    }
}
