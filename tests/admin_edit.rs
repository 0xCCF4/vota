//! The admin edits a Ballot (`PATCH /admin/ballots/{id}`) while it has no
//! Response: any subset of the creation fields, validated like creation, on
//! the Ballot as the edit leaves it. Once it has a Response, only its Group
//! and Voter Reference (the admin's bookkeeping) may still change.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// Creates the standard Ballot; returns the admin's token and the creation
/// response.
async fn create(app: &TestApp, request: &Value) -> (String, Value) {
    let admin = app.admin_token().await;
    let created = app.create_ballot(request).await;
    (admin, created)
}

fn path(created: &Value) -> String {
    format!("/admin/ballots/{}", created["id"])
}

#[tokio::test]
async fn admin_changes_the_title_of_an_unanswered_ballot_and_the_voter_sees_it() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "title": "Dinner on Friday?" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["title"], "Dinner on Friday?");
    assert_eq!(edited["options"], created["options"], "Options untouched");
    let voter = app.voter_view(created["token"].as_str().unwrap()).await;
    assert_eq!(voter["title"], "Dinner on Friday?");
}

#[tokio::test]
async fn an_answered_ballot_cant_be_edited_and_stays_as_it_was() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["submission_mode"] = json!("editable");
    let (admin, created) = create(&app, &request).await;
    let token = created["token"].as_str().unwrap();
    let pizza = &created["options"][0]["id"];
    app.put_json(
        &format!("/api/b/{token}/response"),
        &json!({ "option_ids": [pizza] }),
    )
    .await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "title": "Something else?" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "already_answered");
    let fetched = body_json(app.admin_get(&path(&created), &admin).await).await;
    assert_eq!(fetched["title"], "Lunch on Friday?");
    assert_eq!(fetched["status"], "answered");
}

/// Creates an Editable Ballot in the Group "Survey" with the Voter Reference
/// "alice" and answers it (Pizza); returns the admin's token and the admin
/// view of the answered Ballot.
async fn answered(app: &TestApp) -> (String, Value) {
    let mut request = ballot_request();
    request["submission_mode"] = json!("editable");
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("alice");
    let (admin, created) = create(app, &request).await;
    let token = created["token"].as_str().unwrap();
    let pizza = &created["options"][0]["id"];
    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [pizza] }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let fetched = body_json(app.admin_get(&path(&created), &admin).await).await;
    assert_eq!(fetched["status"], "answered");
    (admin, fetched)
}

#[tokio::test]
async fn group_and_voter_reference_of_an_answered_ballot_can_be_changed() {
    let app = TestApp::new().await;
    let (admin, before) = answered(&app).await;

    let response = app
        .admin_patch(
            &path(&before),
            &admin,
            &json!({ "group": "Survey 2", "voter_reference": "bob" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    let mut expected = before.clone();
    expected["group"] = json!("Survey 2");
    expected["voter_reference"] = json!("bob");
    assert_eq!(edited, expected, "only the bookkeeping changed");
    let fetched = body_json(app.admin_get(&path(&before), &admin).await).await;
    assert_eq!(fetched, expected);
}

#[tokio::test]
async fn group_and_voter_reference_of_an_answered_ballot_can_be_cleared_one_at_a_time() {
    let app = TestApp::new().await;
    let (admin, before) = answered(&app).await;

    let response = app
        .admin_patch(&path(&before), &admin, &json!({ "voter_reference": null }))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["group"], "Survey");
    assert_eq!(edited["voter_reference"], json!(null));

    let response = app
        .admin_patch(&path(&before), &admin, &json!({ "group": null }))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(
        (&edited["group"], &edited["voter_reference"]),
        (&json!(null), &json!(null))
    );
    assert_eq!(edited["response"], before["response"], "Response kept");
    assert_eq!(edited["status"], "answered");
}

#[tokio::test]
async fn any_other_field_beside_the_bookkeeping_still_refuses_an_answered_ballot() {
    let app = TestApp::new().await;
    let (admin, before) = answered(&app).await;
    let unchanged_title = before["title"].clone();

    for other in [
        json!({ "title": unchanged_title }),
        json!({ "description": null }),
        json!({ "options": [{ "label": "A" }, { "label": "B" }] }),
        json!({ "choice_mode": "multiple" }),
        json!({ "submission_mode": "final" }),
        json!({ "fallback": null }),
        json!({ "voting_deadline": "2030-01-25T12:00:00Z" }),
        json!({ "purge_date": "2030-03-01T12:00:00Z" }),
    ] {
        let mut request = other.clone();
        request["group"] = json!("Survey 2");
        request["voter_reference"] = json!("bob");

        let response = app.admin_patch(&path(&before), &admin, &request).await;

        assert_eq!(response.status(), StatusCode::CONFLICT, "{request}");
        assert_eq!(
            body_json(response).await["error"],
            "already_answered",
            "{request}"
        );
        let fetched = body_json(app.admin_get(&path(&before), &admin).await).await;
        assert_eq!(fetched, before, "unchanged after {request}");
    }
}

#[tokio::test]
async fn a_voter_reference_taken_in_the_group_is_a_conflict_on_an_answered_ballot_too() {
    let app = TestApp::new().await;
    let (admin, before) = answered(&app).await;
    let mut bob = ballot_request();
    bob["group"] = json!("Survey");
    bob["voter_reference"] = json!("bob");
    app.create_ballot(&bob).await;

    let response = app
        .admin_patch(&path(&before), &admin, &json!({ "voter_reference": "bob" }))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(response).await["error"],
        "duplicate_voter_reference"
    );
    let fetched = body_json(app.admin_get(&path(&before), &admin).await).await;
    assert_eq!(fetched, before, "unchanged");
}

#[tokio::test]
async fn an_invalid_group_on_an_answered_ballot_is_rejected_naming_it() {
    let app = TestApp::new().await;
    let (admin, before) = answered(&app).await;

    let response = app
        .admin_patch(&path(&before), &admin, &json!({ "group": "  " }))
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(response).await["field"], "group");
    let fetched = body_json(app.admin_get(&path(&before), &admin).await).await;
    assert_eq!(fetched, before, "unchanged");
}

#[tokio::test]
async fn the_voter_can_still_change_an_editable_response_after_a_bookkeeping_edit() {
    let app = TestApp::new().await;
    let (admin, before) = answered(&app).await;
    app.admin_patch(&path(&before), &admin, &json!({ "voter_reference": "bob" }))
        .await;
    let token = before["token"].as_str().unwrap();
    let sushi = &before["options"][1]["id"];

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [sushi] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await["response"]["option_ids"],
        json!([sushi])
    );
}

/// The labels of a Ballot view's Options, in order.
fn labels(view: &Value) -> Vec<&str> {
    view["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn options_are_replaced_as_a_whole_with_new_option_ids() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;
    let old_ids: Vec<&Value> = created["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| &o["id"])
        .collect();

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "options": [{ "label": "Curry" }, { "label": "Pizza" }, { "label": "Soup" }] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(labels(&edited), ["Curry", "Pizza", "Soup"]);
    for option in edited["options"].as_array().unwrap() {
        assert_eq!(option["id"].as_str().unwrap().len(), 11);
        assert!(!old_ids.contains(&&option["id"]), "{edited}");
    }
    let voter = app.voter_view(created["token"].as_str().unwrap()).await;
    assert_eq!(voter["options"], edited["options"]);
}

#[tokio::test]
async fn choice_mode_and_submission_mode_can_be_changed() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "choice_mode": "multiple", "submission_mode": "editable" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let voter = app.voter_view(created["token"].as_str().unwrap()).await;
    assert_eq!(voter["choice_mode"], "multiple");
    assert_eq!(voter["submission_mode"], "editable");
}

/// Sends an edit and asserts a 422 naming `field`, with the Ballot unchanged.
async fn assert_edit_rejected(
    app: &TestApp,
    admin: &str,
    created: &Value,
    edit: &Value,
    field: &str,
) {
    let response = app.admin_patch(&path(created), admin, edit).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {edit}"
    );
    let error = body_json(response).await;
    assert_eq!(error["error"], "invalid_input", "{error}");
    assert_eq!(error["field"], field, "for {edit}: {error}");
    let fetched = body_json(app.admin_get(&path(created), admin).await).await;
    assert_eq!(&fetched, created, "unchanged after {edit}");
}

#[tokio::test]
async fn single_choice_with_a_two_option_fallback_is_rejected() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["choice_mode"] = json!("multiple");
    request["fallback"] = json!(["Pizza", "Sushi"]);
    let (admin, created) = create(&app, &request).await;

    assert_edit_rejected(
        &app,
        &admin,
        &created,
        &json!({ "choice_mode": "single" }),
        "fallback",
    )
    .await;
}

/// The labels of a Ballot view's Fallback, or `None` without one.
fn fallback_labels(view: &Value) -> Option<Vec<&str>> {
    view["fallback"].as_array().map(|options| {
        options
            .iter()
            .map(|o| o["label"].as_str().unwrap())
            .collect()
    })
}

#[tokio::test]
async fn a_fallback_given_with_new_options_resolves_against_the_new_options() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "options": [{ "label": "Curry" }, { "label": "Soup" }], "fallback": ["Soup"] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(
        edited["fallback"],
        json!([edited["options"][1]]),
        "{edited}"
    );
}

#[tokio::test]
async fn the_current_fallback_follows_its_labels_into_new_options() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["fallback"] = json!(["Sushi"]);
    let (admin, created) = create(&app, &request).await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "options": [{ "label": "Sushi" }, { "label": "Curry" }] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(
        edited["fallback"],
        json!([edited["options"][0]]),
        "{edited}"
    );
}

#[tokio::test]
async fn new_options_without_the_current_fallbacks_labels_are_rejected() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["fallback"] = json!(["Sushi"]);
    let (admin, created) = create(&app, &request).await;

    let edit = json!({ "options": [{ "label": "Curry" }, { "label": "Soup" }] });
    assert_edit_rejected(&app, &admin, &created, &edit, "fallback").await;
}

#[tokio::test]
async fn a_fallback_can_be_set_changed_and_removed() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;
    let path = path(&created);

    let set = body_json(
        app.admin_patch(&path, &admin, &json!({ "fallback": ["Pizza"] }))
            .await,
    )
    .await;
    let changed = body_json(
        app.admin_patch(&path, &admin, &json!({ "fallback": ["Sushi"] }))
            .await,
    )
    .await;
    let removed = body_json(
        app.admin_patch(&path, &admin, &json!({ "fallback": null }))
            .await,
    )
    .await;

    assert_eq!(fallback_labels(&set), Some(vec!["Pizza"]));
    assert_eq!(fallback_labels(&changed), Some(vec!["Sushi"]));
    assert_eq!(fallback_labels(&removed), None, "{removed}");
}

#[tokio::test]
async fn an_unknown_fallback_label_is_rejected() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;

    assert_edit_rejected(
        &app,
        &admin,
        &created,
        &json!({ "fallback": ["Curry"] }),
        "fallback[0]",
    )
    .await;
}

#[tokio::test]
async fn group_and_voter_reference_can_be_set_and_cleared() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;
    let path = path(&created);

    let set = body_json(
        app.admin_patch(
            &path,
            &admin,
            &json!({ "group": "Survey", "voter_reference": "alice" }),
        )
        .await,
    )
    .await;
    let cleared = body_json(
        app.admin_patch(
            &path,
            &admin,
            &json!({ "group": null, "voter_reference": null }),
        )
        .await,
    )
    .await;

    assert_eq!(
        (&set["group"], &set["voter_reference"]),
        (&json!("Survey"), &json!("alice"))
    );
    assert_eq!(
        (&cleared["group"], &cleared["voter_reference"]),
        (&json!(null), &json!(null))
    );
}

#[tokio::test]
async fn a_voter_reference_taken_in_the_group_is_a_conflict() {
    let app = TestApp::new().await;
    let mut alice = ballot_request();
    alice["group"] = json!("Survey");
    alice["voter_reference"] = json!("alice");
    app.create_ballot(&alice).await;
    let mut request = ballot_request();
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("bob");
    let (admin, created) = create(&app, &request).await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "voter_reference": "alice", "title": "New?" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(response).await["error"],
        "duplicate_voter_reference"
    );
    let fetched = body_json(app.admin_get(&path(&created), &admin).await).await;
    assert_eq!(fetched, created, "unchanged");
}

// The standard Ballot: deadline 2030-01-22T12:00Z, Purge Date four weeks
// later (2030-02-19T12:00Z), so a gap of 28 days.

#[tokio::test]
async fn an_earlier_voting_deadline_leaves_the_purge_date_alone() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;

    let edited = body_json(
        app.admin_patch(
            &path(&created),
            &admin,
            &json!({ "voting_deadline": "2030-01-20T08:00:00+01:00" }),
        )
        .await,
    )
    .await;

    assert_eq!(edited["voting_deadline"], "2030-01-20T07:00:00Z");
    assert_eq!(edited["purge_date"], "2030-02-19T12:00:00Z");
}

#[tokio::test]
async fn a_voting_deadline_past_the_purge_date_shifts_it_keeping_the_gap() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;

    let edited = body_json(
        app.admin_patch(
            &path(&created),
            &admin,
            &json!({ "voting_deadline": "2030-03-01T00:00:00Z" }),
        )
        .await,
    )
    .await;

    assert_eq!(edited["voting_deadline"], "2030-03-01T00:00:00Z");
    assert_eq!(edited["purge_date"], "2030-03-29T00:00:00Z");
}

#[tokio::test]
async fn the_purge_date_can_be_set_but_not_before_the_voting_deadline() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;

    let early = json!({ "purge_date": "2030-01-22T11:59:59Z" });
    assert_edit_rejected(&app, &admin, &created, &early, "purge_date").await;
    let both =
        json!({ "voting_deadline": "2030-02-01T00:00:00Z", "purge_date": "2030-01-30T00:00:00Z" });
    assert_edit_rejected(&app, &admin, &created, &both, "purge_date").await;

    let edited = body_json(
        app.admin_patch(
            &path(&created),
            &admin,
            &json!({ "purge_date": "2030-01-22T12:00:00Z" }),
        )
        .await,
    )
    .await;
    assert_eq!(edited["purge_date"], "2030-01-22T12:00:00Z");
}

#[tokio::test]
async fn a_new_voting_deadline_must_lie_in_the_future() {
    let app = TestApp::new().await;
    let (admin, created) = create(&app, &ballot_request()).await;

    for deadline in ["2030-01-15T12:00:00Z", "2020-01-01T00:00:00Z"] {
        let edit = json!({ "voting_deadline": deadline });
        assert_edit_rejected(&app, &admin, &created, &edit, "voting_deadline").await;
    }
}
