//! The admin extends a Ballot's Voting Deadline
//! (`POST /admin/ballots/{id}/extend`): always allowed, even once Answered;
//! a closed Ballot without a Response reopens; the Purge Date shifts to keep
//! its gap when the new deadline passes it.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

// The standard Ballot: deadline 2030-01-22T12:00Z, Purge Date
// 2030-02-19T12:00Z (a gap of 28 days); the Clock starts at 2030-01-15T12:00Z.

fn extend_path(created: &Value) -> String {
    format!("/admin/ballots/{}/extend", created["id"])
}

async fn extend(app: &TestApp, admin: &str, created: &Value, deadline: &str) -> Value {
    let response = app
        .post_json(
            &extend_path(created),
            Some(admin),
            &json!({ "voting_deadline": deadline }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK, "extending to {deadline}");
    body_json(response).await
}

#[tokio::test]
async fn extending_within_the_purge_date_moves_only_the_deadline() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;

    let extended = extend(&app, &admin, &created, "2030-02-01T12:00:00+01:00").await;

    assert_eq!(extended["voting_deadline"], "2030-02-01T11:00:00Z");
    assert_eq!(extended["purge_date"], "2030-02-19T12:00:00Z");
}

#[tokio::test]
async fn extending_past_the_purge_date_shifts_it_to_keep_its_gap() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;

    let extended = extend(&app, &admin, &created, "2030-03-10T06:30:00Z").await;

    assert_eq!(extended["voting_deadline"], "2030-03-10T06:30:00Z");
    assert_eq!(extended["purge_date"], "2030-04-07T06:30:00Z");
}

#[tokio::test]
async fn extending_after_auto_fallback_reopens_the_ballot_for_the_voter() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["fallback"] = json!(["Sushi"]);
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap();
    app.clock.set("2030-01-25T00:00:00Z".parse().unwrap());
    assert_eq!(app.voter_view(token).await["status"], "auto_fallback");

    let extended = extend(&app, &admin, &created, "2030-01-29T00:00:00Z").await;

    assert_eq!(extended["status"], "unanswered");
    assert_eq!(extended["result"], json!(null));
    let pizza = &created["options"][0]["id"];
    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [pizza] }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let voter = body_json(response).await;
    assert_eq!(voter["status"], "answered");
    assert_eq!(voter["result"]["option_ids"], json!([pizza]));
}

#[tokio::test]
async fn extending_after_closing_without_fallback_reopens_the_voting_page() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    let token = created["token"].as_str().unwrap();
    app.clock.set("2030-01-25T00:00:00Z".parse().unwrap());

    extend(&app, &admin, &created, "2030-01-29T00:00:00Z").await;

    let pizza = created["options"][0]["id"].as_str().unwrap();
    let response = app
        .post_form(
            &format!("/b/{token}"),
            &format!("option={pizza}&confirm=yes"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(app.voter_view(token).await["status"], "answered");
}

#[tokio::test]
async fn an_answered_ballot_can_still_be_extended() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["submission_mode"] = json!("editable");
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap();
    let pizza = &created["options"][0]["id"];
    app.put_json(
        &format!("/api/b/{token}/response"),
        &json!({ "option_ids": [pizza] }),
    )
    .await;

    let extended = extend(&app, &admin, &created, "2030-02-01T00:00:00Z").await;

    assert_eq!(extended["status"], "answered");
    assert_eq!(extended["voting_deadline"], "2030-02-01T00:00:00Z");
    assert_eq!(extended["response"]["option_ids"], json!([pizza]));
}

/// Asks to extend to `deadline` and asserts a 422 naming `voting_deadline`,
/// with the Ballot unchanged.
async fn assert_extension_rejected(app: &TestApp, admin: &str, created: &Value, body: &Value) {
    let response = app
        .post_json(&extend_path(created), Some(admin), body)
        .await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {body}"
    );
    let error = body_json(response).await;
    assert_eq!(error["error"], "invalid_input", "{error}");
    assert_eq!(error["field"], "voting_deadline", "for {body}: {error}");
    let path = format!("/admin/ballots/{}", created["id"]);
    let fetched = body_json(app.admin_get(&path, admin).await).await;
    assert_eq!(fetched["voting_deadline"], created["voting_deadline"]);
    assert_eq!(fetched["purge_date"], created["purge_date"]);
}

#[tokio::test]
async fn the_new_deadline_must_be_later_than_the_current_one() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;

    for deadline in ["2030-01-22T12:00:00Z", "2030-01-20T00:00:00Z"] {
        let body = json!({ "voting_deadline": deadline });
        assert_extension_rejected(&app, &admin, &created, &body).await;
    }
}

#[tokio::test]
async fn the_new_deadline_must_lie_in_the_future() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    app.clock.set("2030-02-01T00:00:00Z".parse().unwrap());

    // Later than the current deadline, but already past.
    let body = json!({ "voting_deadline": "2030-01-30T00:00:00Z" });
    assert_extension_rejected(&app, &admin, &created, &body).await;
}
