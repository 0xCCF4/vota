#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::json;

#[tokio::test]
async fn voter_api_returns_the_ballot_by_its_token() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&ballot_request()).await;
    let token = created["token"].as_str().unwrap();

    let response = app.get(&format!("/api/b/{token}")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await,
        json!({
            "title": "Lunch on Friday?",
            "revision": 0,
            "description": null,
            "options": created["options"],
            "choice_mode": "single",
            "allow_no_option": false,
            "submission_mode": "final",
            "voting_deadline": "2030-01-22T12:00:00Z",
            "voting_open": true,
            "status": "unanswered",
            "fallback": null,
            "response": null,
            "result": null,
            "theme": "classic",
            "accent_color": null,
            "logo_url": null,
        })
    );
}

#[tokio::test]
async fn unknown_and_malformed_tokens_get_byte_identical_not_found() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&ballot_request()).await;
    let real = created["token"].as_str().unwrap();
    // Same alphabet and length as a real token, but never issued.
    let unknown = if real.starts_with('A') { "B" } else { "A" }.to_owned() + &real[1..];

    let mut responses = Vec::new();
    for path in common::not_found_token_paths("/api/b/", &unknown) {
        responses.push(common::snapshot(app.get(&path).await).await);
    }

    assert_eq!(responses[0].0, StatusCode::NOT_FOUND);
    for response in &responses[1..] {
        assert_eq!(response, &responses[0]);
    }
}
