#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use std::collections::HashSet;

use axum::body::Body;
use axum::http::Request;
use common::{TestApp, ballot_request, body_json};
use serde_json::json;

#[tokio::test]
async fn admin_creates_a_ballot_and_gets_its_public_url() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;

    let response = app
        .post_json("/admin/ballots", Some(&token), &ballot_request())
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    let ballot_token = body["token"].as_str().unwrap();
    assert_eq!(
        body["url"],
        format!("https://ballots.example.org/b/{ballot_token}")
    );
    assert!(body["id"].is_i64());
}

#[tokio::test]
async fn options_keep_their_order_and_get_distinct_server_generated_ids() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["options"] = json!([{ "label": "Pizza" }, { "label": "Sushi" }, { "label": "Salad" }]);

    let body = app.create_ballot(&request).await;

    let options = body["options"].as_array().unwrap();
    let labels: Vec<_> = options
        .iter()
        .map(|o| o["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Pizza", "Sushi", "Salad"]);
    let ids: HashSet<_> = options.iter().map(|o| o["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 3);
    for id in ids {
        assert_eq!(id.len(), 11, "{id}");
        assert!(is_base64url(id), "{id}");
    }
}

#[tokio::test]
async fn voting_deadline_with_an_offset_is_stored_in_utc() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["voting_deadline"] = json!("2030-01-22T13:30:00+01:00");

    let body = app.create_ballot(&request).await;

    assert_eq!(body["voting_deadline"], "2030-01-22T12:30:00Z");
}

#[tokio::test]
async fn purge_date_defaults_to_four_weeks_after_the_voting_deadline() {
    let app = TestApp::new().await;

    let body = app.create_ballot(&ballot_request()).await;

    // Deadline 2030-01-22T12:00:00Z + 28 days.
    assert_eq!(body["purge_date"], "2030-02-19T12:00:00Z");
}

#[tokio::test]
async fn purge_date_default_follows_the_configured_gap() {
    let app = TestApp::with_config(|config| {
        config.default_purge_gap = std::time::Duration::from_secs(3 * 24 * 60 * 60);
    })
    .await;

    let body = app.create_ballot(&ballot_request()).await;

    assert_eq!(body["purge_date"], "2030-01-25T12:00:00Z");
}

#[tokio::test]
async fn purge_date_can_be_given_with_any_offset_and_is_returned_in_utc() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["purge_date"] = json!("2030-03-01T09:00:00+02:00");

    let body = app.create_ballot(&request).await;

    assert_eq!(body["purge_date"], "2030-03-01T07:00:00Z");
}

#[tokio::test]
async fn purge_date_may_equal_the_voting_deadline() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["purge_date"] = json!("2030-01-22T12:00:00Z");

    let body = app.create_ballot(&request).await;

    assert_eq!(body["purge_date"], "2030-01-22T12:00:00Z");
}

#[tokio::test]
async fn ballot_tokens_are_22_base64url_characters_and_unique() {
    let app = TestApp::new().await;

    let mut tokens = HashSet::new();
    for _ in 0..20 {
        let body = app.create_ballot(&ballot_request()).await;
        let token = body["token"].as_str().unwrap().to_owned();
        assert_eq!(token.len(), 22, "{token}");
        assert!(is_base64url(&token), "{token}");
        tokens.insert(token);
    }

    assert_eq!(tokens.len(), 20);
}

#[tokio::test]
async fn each_ballot_gets_its_own_internal_id() {
    let app = TestApp::new().await;

    let first = app.create_ballot(&ballot_request()).await;
    let second = app.create_ballot(&ballot_request()).await;

    assert_ne!(first["id"], second["id"]);
}

#[tokio::test]
async fn ballot_url_comes_from_configuration_never_from_request_headers() {
    let app = TestApp::with_config(|config| {
        config.public_base_url = "https://example.org/polls/".parse().unwrap();
    })
    .await;
    let token = app.admin_token().await;
    // Under the base URL's path, as every route is.
    let request = Request::post("/polls/admin/ballots")
        .header("host", "evil.example")
        .header("x-forwarded-host", "evil.example")
        .header("x-forwarded-proto", "http")
        .header("forwarded", "host=evil.example;proto=http")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(ballot_request().to_string()))
        .unwrap();

    let body = body_json(app.send(request).await).await;

    let ballot_token = body["token"].as_str().unwrap();
    assert_eq!(
        body["url"],
        format!("https://example.org/polls/b/{ballot_token}")
    );
}

fn is_base64url(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
