//! Voting Deadline and Purge Date inputs are capped at now + `max_date_horizon`
//! (default one year) on create, edit and extend: beyond it is a 422 naming the
//! field. A Purge Date shifted by an edit of the Voting Deadline is capped
//! too (a 422 on `voting_deadline`); one shifted by an extension is not.

#![cfg(feature = "server")]

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// Thirty days: the horizon of [`app`].
const HORIZON: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// `common::start_time()` (2030-01-15T12:00:00Z) plus [`HORIZON`].
const LATEST: &str = "2030-02-14T12:00:00Z";

/// One millisecond after [`LATEST`].
const BEYOND: &str = "2030-02-14T12:00:00.001Z";

async fn app() -> TestApp {
    TestApp::with_config(|config| config.max_date_horizon = HORIZON).await
}

async fn assert_422(response: axum::http::Response<axum::body::Body>, field: &str, what: &str) {
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{what}"
    );
    let error = body_json(response).await;
    assert_eq!(error["error"], "invalid_input", "{what}: {error}");
    assert_eq!(error["field"], field, "{what}: {error}");
}

#[tokio::test]
async fn a_voting_deadline_beyond_the_horizon_is_rejected_at_creation() {
    let app = app().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["voting_deadline"] = json!(BEYOND);

    let response = app
        .post_json("/admin/ballots", Some(&admin), &request)
        .await;

    assert_422(response, "voting_deadline", "deadline beyond the horizon").await;
}

#[tokio::test]
async fn a_purge_date_beyond_the_horizon_is_rejected_at_creation() {
    let app = app().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["purge_date"] = json!(BEYOND);

    let response = app
        .post_json("/admin/ballots", Some(&admin), &request)
        .await;

    assert_422(response, "purge_date", "Purge Date beyond the horizon").await;
}

#[tokio::test]
async fn dates_exactly_at_the_horizon_are_accepted_at_creation() {
    let app = app().await;
    let mut request = ballot_request();
    request["voting_deadline"] = json!(LATEST);
    request["purge_date"] = json!(LATEST);

    let created = app.create_ballot(&request).await;

    assert_eq!(created["voting_deadline"], LATEST);
    assert_eq!(created["purge_date"], LATEST);
}

#[tokio::test]
async fn the_default_purge_date_is_not_capped() {
    let app = app().await;
    let mut request = ballot_request();
    request["voting_deadline"] = json!(LATEST);

    let created = app.create_ballot(&request).await;

    // Voting Deadline plus the default four weeks, past the horizon.
    assert_eq!(created["purge_date"], "2030-03-14T12:00:00Z");
}

#[tokio::test]
async fn the_horizon_counts_from_the_clocks_now() {
    let app = app().await;
    app.clock.advance(chrono::Duration::days(10));
    let mut request = ballot_request();
    request["voting_deadline"] = json!("2030-02-24T12:00:00Z");

    let created = app.create_ballot(&request).await;

    assert_eq!(created["voting_deadline"], "2030-02-24T12:00:00Z");
}

/// Creates the standard Ballot (deadline 2030-01-22T12:00Z, Purge Date
/// 2030-02-19T12:00Z); returns the admin's token and the creation response.
async fn created(app: &TestApp) -> (String, Value) {
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    (admin, created)
}

#[tokio::test]
async fn edits_beyond_the_horizon_are_rejected_naming_the_field() {
    let app = app().await;
    let (admin, created) = created(&app).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    for (body, field) in [
        (json!({ "voting_deadline": BEYOND }), "voting_deadline"),
        (json!({ "purge_date": BEYOND }), "purge_date"),
    ] {
        let response = app.admin_patch(&path, &admin, &body).await;
        assert_422(response, field, &body.to_string()).await;
    }
    let fetched = body_json(app.admin_get(&path, &admin).await).await;
    assert_eq!(fetched, created, "unchanged");
}

#[tokio::test]
async fn edits_up_to_the_horizon_are_accepted() {
    let app = app().await;
    let (admin, created) = created(&app).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    let response = app
        .admin_patch(
            &path,
            &admin,
            &json!({ "voting_deadline": LATEST, "purge_date": LATEST }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(
        (&edited["voting_deadline"], &edited["purge_date"]),
        (&json!(LATEST), &json!(LATEST))
    );
}

#[tokio::test]
async fn an_extension_beyond_the_horizon_is_rejected() {
    let app = app().await;
    let (admin, created) = created(&app).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    let response = app
        .post_json(
            &format!("{path}/extend"),
            Some(&admin),
            &json!({ "voting_deadline": BEYOND }),
        )
        .await;

    assert_422(response, "voting_deadline", "extension beyond the horizon").await;
    let fetched = body_json(app.admin_get(&path, &admin).await).await;
    assert_eq!(fetched, created, "unchanged");
}

#[tokio::test]
async fn the_purge_date_an_extension_shifts_is_not_capped() {
    let app = app().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["purge_date"] = json!("2030-01-29T12:00:00Z");
    let created = app.create_ballot(&request).await;

    let response = app
        .post_json(
            &format!("/admin/ballots/{}/extend", created["id"]),
            Some(&admin),
            &json!({ "voting_deadline": LATEST }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let extended = body_json(response).await;
    assert_eq!(extended["voting_deadline"], LATEST);
    // Shifted to keep the 7-day gap, past the horizon.
    assert_eq!(extended["purge_date"], "2030-02-21T12:00:00Z");
}

/// Creates a Ballot with a 7-day gap between Voting Deadline
/// (2030-01-22T12:00Z) and Purge Date (2030-01-29T12:00Z).
async fn created_with_a_week_to_purge(app: &TestApp) -> (String, Value) {
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["purge_date"] = json!("2030-01-29T12:00:00Z");
    (admin, app.create_ballot(&request).await)
}

#[tokio::test]
async fn an_edit_whose_shifted_purge_date_passes_the_horizon_is_rejected_on_the_deadline() {
    let app = app().await;
    let (admin, created) = created_with_a_week_to_purge(&app).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    // Within the horizon, but the Purge Date would shift to BEYOND.
    let response = app
        .admin_patch(
            &path,
            &admin,
            &json!({ "voting_deadline": "2030-02-07T12:00:00.001Z" }),
        )
        .await;

    assert_422(
        response,
        "voting_deadline",
        "shifted Purge Date beyond the horizon",
    )
    .await;
    let fetched = body_json(app.admin_get(&path, &admin).await).await;
    assert_eq!(fetched, created, "unchanged");
}

#[tokio::test]
async fn an_edit_may_shift_the_purge_date_up_to_the_horizon() {
    let app = app().await;
    let (admin, created) = created_with_a_week_to_purge(&app).await;

    let response = app
        .admin_patch(
            &format!("/admin/ballots/{}", created["id"]),
            &admin,
            &json!({ "voting_deadline": "2030-02-07T12:00:00Z" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["purge_date"], LATEST);
}

#[tokio::test]
async fn the_default_horizon_is_one_year() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();

    // Start 2030-01-15T12:00Z; humantime's year is 365.25 days.
    request["voting_deadline"] = json!("2031-01-15T18:00:00Z");
    let created = app.create_ballot(&request).await;
    assert_eq!(created["voting_deadline"], "2031-01-15T18:00:00Z");

    request["voting_deadline"] = json!("2031-01-15T18:00:00.001Z");
    let response = app
        .post_json("/admin/ballots", Some(&admin), &request)
        .await;
    assert_422(response, "voting_deadline", "beyond one year").await;
}
