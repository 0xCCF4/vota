//! A Response is recorded only against the Ballot as its voter saw it: the
//! voter API exposes the Ballot's `revision`, the voting page and its
//! confirmation step carry it as a hidden field, and a submission must send
//! it back. If an admin changed the Ballot's content since (a new revision),
//! nothing is recorded: 409 `ballot_changed`. Bookkeeping edits (Group,
//! Voter Reference) don't make a new revision.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, body_json, body_string};
use serde_json::json;

/// Creates a Final single-choice Ballot "Do you approve proposal X?" with
/// the Options Yes and No; returns its admin ID, Ballot Token and Option IDs.
async fn proposal(app: &TestApp, submission_mode: &str) -> (i64, String, Vec<String>) {
    let mut request = common::ballot_request();
    request["title"] = json!("Do you approve proposal X?");
    request["options"] = json!([{ "label": "Yes" }, { "label": "No" }]);
    request["submission_mode"] = json!(submission_mode);
    let created = app.create_ballot(&request).await;
    let options = created["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect();
    (
        created["id"].as_i64().unwrap(),
        created["token"].as_str().unwrap().to_owned(),
        options,
    )
}

async fn admin_edit(app: &TestApp, id: i64, change: serde_json::Value) {
    let admin = app.admin_token().await;
    let response = app
        .admin_patch(&format!("/admin/ballots/{id}"), &admin, &change)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_voter_api_shows_the_revision_which_content_edits_advance_and_bookkeeping_edits_keep() {
    let app = TestApp::new().await;
    let (id, token, _) = proposal(&app, "final").await;
    assert_eq!(app.voter_view(&token).await["revision"], 0);

    admin_edit(&app, id, json!({ "title": "Do you REJECT proposal X?" })).await;
    assert_eq!(app.voter_view(&token).await["revision"], 1);

    admin_edit(
        &app,
        id,
        json!({ "group": "board", "voter_reference": "m-17" }),
    )
    .await;
    assert_eq!(app.voter_view(&token).await["revision"], 1);
}

#[tokio::test]
async fn the_voting_page_and_the_confirmation_step_carry_the_revision() {
    let app = TestApp::new().await;
    let (id, token, options) = proposal(&app, "final").await;
    admin_edit(&app, id, json!({ "title": "Do you approve proposal Y?" })).await;
    let hidden = r#"<input type="hidden" name="revision" value="1">"#;

    let page = body_string(app.get(&format!("/b/{token}")).await).await;
    assert!(page.contains(hidden), "{page}");
    assert!(page.contains(r#" data-revision="1""#), "{page}");

    let confirmation = app
        .post_form(
            &format!("/b/{token}"),
            &format!("option={}&revision=1", options[0]),
        )
        .await;
    assert_eq!(confirmation.status(), StatusCode::OK);
    let confirmation = body_string(confirmation).await;
    assert!(
        confirmation.contains("Confirm your answer"),
        "{confirmation}"
    );
    assert!(confirmation.contains(hidden), "{confirmation}");
}

#[tokio::test]
async fn an_api_response_to_wording_the_admin_has_since_negated_is_not_recorded() {
    let app = TestApp::new().await;
    let (id, token, options) = proposal(&app, "final").await;
    let seen = app.voter_view(&token).await;
    assert_eq!(seen["title"], "Do you approve proposal X?");

    admin_edit(&app, id, json!({ "title": "Do you REJECT proposal X?" })).await;
    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[0]], "revision": seen["revision"] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "ballot_changed");
    let now = app.voter_view(&token).await;
    assert_eq!(now["status"], "unanswered");
    assert_eq!(now["title"], "Do you REJECT proposal X?");
    // Fetched again, the voter answers the new wording.
    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[1]], "revision": now["revision"] }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_confirmation_of_wording_the_admin_has_since_negated_is_not_recorded() {
    let app = TestApp::new().await;
    let (id, token, options) = proposal(&app, "final").await;
    let page = format!("/b/{token}");
    let yes = &options[0];
    let confirmation = body_string(
        app.post_form(&page, &format!("option={yes}&revision=0"))
            .await,
    )
    .await;
    assert!(confirmation.contains("Do you approve proposal X?"));

    admin_edit(&app, id, json!({ "title": "Do you REJECT proposal X?" })).await;
    let response = app
        .post_form(&page, &format!("option={yes}&confirm=yes&revision=0"))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let html = body_string(response).await;
    assert!(
        html.contains("This Ballot was changed by its organiser"),
        "{html}"
    );
    assert!(html.contains("Do you REJECT proposal X?"), "{html}");
    assert!(
        html.contains(r#"<input type="hidden" name="revision" value="1">"#),
        "{html}"
    );
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn an_editable_response_from_a_stale_page_is_not_recorded() {
    let app = TestApp::new().await;
    let (id, token, options) = proposal(&app, "editable").await;

    admin_edit(&app, id, json!({ "choice_mode": "multiple" })).await;
    let api = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[0]], "revision": 0 }),
        )
        .await;
    let page = app
        .post_form(
            &format!("/b/{token}"),
            &format!("option={}&revision=0", options[0]),
        )
        .await;

    assert_eq!(api.status(), StatusCode::CONFLICT);
    assert_eq!(page.status(), StatusCode::CONFLICT);
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn a_bookkeeping_edit_since_the_voter_loaded_the_ballot_does_not_refuse_the_response() {
    let app = TestApp::new().await;
    let (id, token, options) = proposal(&app, "editable").await;

    admin_edit(&app, id, json!({ "group": "board" })).await;
    let api = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[0]], "revision": 0 }),
        )
        .await;
    let page = app
        .post_form(
            &format!("/b/{token}"),
            &format!("option={}&revision=0", options[1]),
        )
        .await;

    assert_eq!(api.status(), StatusCode::OK);
    assert_eq!(page.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        app.voter_view(&token).await["response"]["option_ids"],
        json!([options[1]])
    );
}

#[tokio::test]
async fn a_revision_from_the_future_is_refused_like_a_stale_one() {
    let app = TestApp::new().await;
    let (_, token, options) = proposal(&app, "final").await;

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[0]], "revision": 7 }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "ballot_changed");
}
