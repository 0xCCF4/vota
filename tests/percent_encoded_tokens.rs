//! A Ballot Token has exactly one spelling in a URL: a percent-encoded
//! character, even one that decodes to the real token, is the surface's
//! uniform not-found, like any unknown token.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, snapshot};

/// `token` with its first character percent-encoded (e.g. `X…` as `%58…`).
fn percent_encoded(token: &str) -> String {
    format!("%{:02X}{}", token.as_bytes()[0], &token[1..])
}

/// `token` with a different first character: well-formed, never issued.
fn unknown(token: &str) -> String {
    let first = if token.starts_with('A') { "B" } else { "A" };
    format!("{first}{}", &token[1..])
}

#[tokio::test]
async fn a_percent_encoded_token_gets_the_voter_apis_uniform_not_found() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;

    let encoded = snapshot(
        app.get(&format!("/api/b/{}", percent_encoded(&token)))
            .await,
    )
    .await;
    let unknown = snapshot(app.get(&format!("/api/b/{}", unknown(&token))).await).await;

    assert_eq!(encoded.0, StatusCode::NOT_FOUND);
    assert_eq!(encoded, unknown);
}

#[tokio::test]
async fn a_percent_encoded_token_gets_the_voting_pages_uniform_not_found() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;

    let encoded = snapshot(app.get(&format!("/b/{}", percent_encoded(&token))).await).await;
    let unknown = snapshot(app.get(&format!("/b/{}", unknown(&token))).await).await;

    assert_eq!(encoded.0, StatusCode::NOT_FOUND);
    assert_eq!(encoded, unknown);
}

#[tokio::test]
async fn a_response_to_a_percent_encoded_token_is_not_found_and_not_recorded() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let encoded = percent_encoded(&token);

    let api = app
        .put_raw(
            &format!("/api/b/{encoded}/response"),
            "application/json",
            format!(r#"{{"option_ids":["{}"],"revision":0}}"#, options[0]),
        )
        .await;
    let form = app
        .post_raw(
            &format!("/b/{encoded}"),
            None,
            "application/x-www-form-urlencoded",
            format!("option={}&revision=0", options[0]),
        )
        .await;

    assert_eq!(api.status(), StatusCode::NOT_FOUND);
    assert_eq!(form.status(), StatusCode::NOT_FOUND);
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}
