//! The Voting Deadline closes a Ballot: no Response can be submitted or
//! changed from that moment on (the deadline itself is already closed), on
//! the voter API and the no-JS form alike.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use common::{TestApp, body_json, body_string};
use serde_json::json;

/// The Voting Deadline of every Ballot built from `common::ballot_request`.
fn deadline() -> DateTime<Utc> {
    "2030-01-22T12:00:00Z".parse().unwrap()
}

#[tokio::test]
async fn an_api_submission_at_the_deadline_is_refused_as_voting_closed() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "final").await;
    let before = app.voter_view(&token).await;
    app.clock.set(deadline());

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[0]] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "voting_closed");
    assert_eq!(app.voter_view(&token).await["response"], before["response"]);
}

#[tokio::test]
async fn an_editable_response_can_be_changed_until_just_before_the_deadline_but_not_at_it() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "editable").await;
    let path = format!("/api/b/{token}/response");
    app.clock
        .set(deadline() - chrono::Duration::milliseconds(1));
    let last_moment = app
        .put_json(&path, &json!({ "option_ids": [options[0]] }))
        .await;
    assert_eq!(last_moment.status(), StatusCode::OK);
    app.clock.set(deadline());

    let response = app
        .put_json(&path, &json!({ "option_ids": [options[1], options[2]] }))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "voting_closed");
    assert_eq!(
        app.voter_view(&token).await["response"]["option_ids"],
        json!([options[0]])
    );
}

#[tokio::test]
async fn a_closed_ballot_refuses_even_an_invalid_response_as_voting_closed() {
    let app = TestApp::new().await;
    let (token, _) = app.create_ballot_with_modes("single", "editable").await;
    app.clock.set(deadline() + chrono::Duration::days(1));

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "voting_closed");
}

#[tokio::test]
async fn a_form_response_after_the_deadline_shows_the_closed_page_and_records_nothing() {
    let app = TestApp::new().await;
    for (submission_mode, body_suffix) in
        [("final", "&confirm=yes"), ("final", ""), ("editable", "")]
    {
        let (token, options) = app
            .create_ballot_with_modes("single", submission_mode)
            .await;
        app.clock.set(deadline());

        let response = app
            .post_form(
                &format!("/b/{token}"),
                &format!("option={}{body_suffix}", options[0]),
            )
            .await;

        assert_eq!(response.status(), StatusCode::CONFLICT, "{submission_mode}");
        let html = body_string(response).await;
        assert!(
            html.contains("Voting has closed. Your answer could not be accepted."),
            "{html}"
        );
        assert!(
            html.contains("Voting closed. No answer was given."),
            "{html}"
        );
        assert!(!html.contains("<form"), "{html}");
        assert_eq!(app.voter_view(&token).await["status"], "unanswered");
        app.clock.set(common::start_time());
    }
}

#[tokio::test]
async fn after_the_deadline_the_page_shows_the_voters_response_read_only() {
    let app = TestApp::new().await;
    for submission_mode in ["final", "editable"] {
        let (token, options) = app
            .create_ballot_with_modes("multiple", submission_mode)
            .await;
        app.put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[0], options[2]] }),
        )
        .await;
        app.clock.set(deadline());

        let response = app.get(&format!("/b/{token}")).await;

        assert_eq!(response.status(), StatusCode::OK);
        let html = body_string(response).await;
        assert!(html.contains("Voting closed. You chose:"), "{html}");
        assert!(
            html.contains("<li>Pizza</li>") && html.contains("<li>Tacos</li>"),
            "{html}"
        );
        assert!(!html.contains("<li>Sushi</li>"), "{html}");
        assert!(!html.contains("<form"), "{html}");
        assert!(
            html.contains(r#"<p class="status">Status: Answered</p>"#),
            "{html}"
        );
        app.clock.set(common::start_time());
    }
}

#[tokio::test]
async fn after_the_deadline_without_response_or_fallback_the_ballot_stays_unanswered() {
    let app = TestApp::new().await;
    let (token, _) = app.create_ballot_with_modes("single", "final").await;
    app.clock.set(deadline());

    let view = app.voter_view(&token).await;
    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert_eq!(view["status"], "unanswered", "{view}");
    assert_eq!(view["response"], json!(null), "{view}");
    assert_eq!(view["result"], json!(null), "{view}");
    assert_eq!(view["voting_open"], json!(false), "{view}");
    assert!(
        html.contains("Voting closed. No answer was given."),
        "{html}"
    );
    assert!(
        html.contains(r#"<p class="status">Status: Unanswered</p>"#),
        "{html}"
    );
    assert!(!html.contains("<form"), "{html}");
}

#[tokio::test]
async fn an_answered_final_ballot_reports_voting_closed_after_the_deadline() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "final").await;
    let path = format!("/api/b/{token}/response");
    app.put_json(&path, &json!({ "option_ids": [options[0]] }))
        .await;
    app.clock.set(deadline());

    let response = app
        .put_json(&path, &json!({ "option_ids": [options[1]] }))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "voting_closed");
    assert_eq!(
        app.voter_view(&token).await["response"]["option_ids"],
        json!([options[0]])
    );
}
