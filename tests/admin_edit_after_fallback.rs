//! Once a Ballot's Voting Deadline has passed without a Response and its
//! Fallback has applied (Ballot Status Auto-fallback), the admin can no
//! longer change its content (`PATCH /admin/ballots/{id}` answers 409
//! `fallback_applied`), only its Group and Voter Reference. Extending the
//! deadline reopens it, and content edits are allowed again. Ballots whose
//! deadline passed without a Fallback stay editable; a Fallback set on one
//! applies at once.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

// The standard Ballot: deadline 2030-01-22T12:00Z, Purge Date
// 2030-02-19T12:00Z; the Clock starts at 2030-01-15T12:00Z.

fn path(created: &Value) -> String {
    format!("/admin/ballots/{}", created["id"])
}

/// Creates the standard Ballot with the Fallback "Sushi", in the Group
/// "Survey" with the Voter Reference "alice"; returns the admin's token and
/// the creation response.
async fn create_with_fallback(app: &TestApp) -> (String, Value) {
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["fallback"] = json!(["Sushi"]);
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("alice");
    (admin, app.create_ballot(&request).await)
}

#[tokio::test]
async fn after_the_deadline_the_content_of_an_auto_fallback_ballot_cant_change() {
    let app = TestApp::new().await;
    let (admin, created) = create_with_fallback(&app).await;
    app.clock.set("2030-01-23T00:00:00Z".parse().unwrap());
    let before = body_json(app.admin_get(&path(&created), &admin).await).await;
    assert_eq!(before["status"], "auto_fallback");

    let response = app
        .admin_patch(&path(&created), &admin, &json!({ "title": "Late lunch?" }))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(response).await,
        json!({
            "error": "fallback_applied",
            "message": "the Fallback of this Ballot has applied; extend the Voting Deadline to edit it again",
        })
    );
    let fetched = body_json(app.admin_get(&path(&created), &admin).await).await;
    assert_eq!(fetched, before, "unchanged");
}

#[tokio::test]
async fn at_the_deadline_itself_the_content_is_locked_already() {
    let app = TestApp::new().await;
    let (admin, created) = create_with_fallback(&app).await;
    app.clock.set("2030-01-22T12:00:00Z".parse().unwrap());

    let response = app
        .admin_patch(&path(&created), &admin, &json!({ "title": "Late lunch?" }))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "fallback_applied");
}

#[tokio::test]
async fn just_before_the_deadline_the_content_can_still_change() {
    let app = TestApp::new().await;
    let (admin, created) = create_with_fallback(&app).await;
    app.clock.set("2030-01-22T11:59:59.999Z".parse().unwrap());

    let response = app
        .admin_patch(&path(&created), &admin, &json!({ "title": "Late lunch?" }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["title"], "Late lunch?");
    assert_eq!(edited["status"], "unanswered");
}

#[tokio::test]
async fn any_content_field_is_refused_before_its_value_is_looked_at() {
    let app = TestApp::new().await;
    let (admin, created) = create_with_fallback(&app).await;
    app.clock.set("2030-01-23T00:00:00Z".parse().unwrap());
    let before = body_json(app.admin_get(&path(&created), &admin).await).await;

    for content in [
        json!({ "title": before["title"] }),
        json!({ "title": "" }),
        json!({ "description": null }),
        json!({ "options": [{ "label": "A" }, { "label": "B" }] }),
        json!({ "options": [] }),
        json!({ "choice_mode": "multiple" }),
        json!({ "submission_mode": "editable" }),
        json!({ "fallback": null }),
        json!({ "fallback": ["Nonexistent"] }),
        json!({ "voting_deadline": "2030-01-30T12:00:00Z" }),
        json!({ "voting_deadline": "2020-01-01T00:00:00Z" }),
        json!({ "purge_date": "2030-03-01T12:00:00Z" }),
        json!({ "theme": "no-such-theme" }),
        json!({ "accent_color": "#123456" }),
        json!({ "logo": null }),
        json!({ "title": "Late lunch?", "group": "Survey 2", "voter_reference": "bob" }),
    ] {
        let response = app.admin_patch(&path(&created), &admin, &content).await;

        assert_eq!(response.status(), StatusCode::CONFLICT, "{content}");
        assert_eq!(
            body_json(response).await["error"],
            "fallback_applied",
            "{content}"
        );
        let fetched = body_json(app.admin_get(&path(&created), &admin).await).await;
        assert_eq!(fetched, before, "unchanged after {content}");
    }
}

#[tokio::test]
async fn group_and_voter_reference_of_an_auto_fallback_ballot_can_still_change() {
    let app = TestApp::new().await;
    let (admin, created) = create_with_fallback(&app).await;
    app.clock.set("2030-01-23T00:00:00Z".parse().unwrap());
    let before = body_json(app.admin_get(&path(&created), &admin).await).await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "group": "Survey 2", "voter_reference": "bob" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(
        (&edited["group"], &edited["voter_reference"]),
        (&json!("Survey 2"), &json!("bob"))
    );
    assert_eq!(edited["status"], "auto_fallback");
    assert_eq!(edited["result"], before["result"], "result kept");

    for one in [json!({ "group": null }), json!({ "voter_reference": null })] {
        let response = app.admin_patch(&path(&created), &admin, &one).await;
        assert_eq!(response.status(), StatusCode::OK, "{one}");
    }
    let fetched = body_json(app.admin_get(&path(&created), &admin).await).await;
    assert_eq!(
        (&fetched["group"], &fetched["voter_reference"]),
        (&json!(null), &json!(null))
    );
}

#[tokio::test]
async fn after_extending_the_deadline_the_content_can_change_again() {
    let app = TestApp::new().await;
    let (admin, created) = create_with_fallback(&app).await;
    app.clock.set("2030-01-23T00:00:00Z".parse().unwrap());
    let extended = app
        .post_json(
            &format!("{}/extend", path(&created)),
            Some(&admin),
            &json!({ "voting_deadline": "2030-01-30T12:00:00Z" }),
        )
        .await;
    assert_eq!(extended.status(), StatusCode::OK);
    assert_eq!(body_json(extended).await["status"], "unanswered");

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "title": "Late lunch?", "voting_deadline": "2030-01-29T12:00:00Z" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["title"], "Late lunch?");
    assert_eq!(edited["voting_deadline"], "2030-01-29T12:00:00Z");
    assert_eq!(edited["status"], "unanswered");
    let voter = app.voter_view(created["token"].as_str().unwrap()).await;
    assert_eq!(voter["title"], "Late lunch?");
    assert_eq!(voter["voting_open"], true);
}

#[tokio::test]
async fn a_ballot_closed_without_a_fallback_stays_editable_and_a_new_deadline_reopens_it() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    app.clock.set("2030-01-23T00:00:00Z".parse().unwrap());
    let before = body_json(app.admin_get(&path(&created), &admin).await).await;
    assert_eq!(before["status"], "unanswered");

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "title": "Late lunch?", "voting_deadline": "2030-01-30T12:00:00Z" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["title"], "Late lunch?");
    assert_eq!(edited["status"], "unanswered");
    let voter = app.voter_view(created["token"].as_str().unwrap()).await;
    assert_eq!(voter["voting_open"], true);
}

#[tokio::test]
async fn a_fallback_set_on_a_ballot_closed_without_one_applies_at_once() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    let sushi = created["options"][1]["id"].clone();
    app.clock.set("2030-01-23T00:00:00Z".parse().unwrap());

    let response = app
        .admin_patch(&path(&created), &admin, &json!({ "fallback": ["Sushi"] }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["status"], "auto_fallback", "{edited}");
    assert_eq!(
        edited["result"],
        json!({
            "source": "fallback",
            "option_ids": [sushi],
            "options": [{ "id": sushi, "label": "Sushi" }],
        }),
        "{edited}"
    );
    let voter = app.voter_view(created["token"].as_str().unwrap()).await;
    assert_eq!(voter["status"], "auto_fallback", "{voter}");
    assert_eq!(
        voter["result"],
        json!({ "source": "fallback", "option_ids": [sushi] }),
        "{voter}"
    );
    assert_eq!(voter["voting_open"], false, "{voter}");
    let further = app
        .admin_patch(&path(&created), &admin, &json!({ "title": "Late lunch?" }))
        .await;
    assert_eq!(further.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(further).await["error"], "fallback_applied");
}
