//! Group and Voter Reference: optional admin bookkeeping on a Ballot. A Voter
//! Reference is unique within its Group and never reaches the voter.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json, body_string};
use serde_json::{Value, json};

/// A Ballot request with this Group and Voter Reference.
fn request_for(group: &str, voter_reference: &str) -> Value {
    let mut request = ballot_request();
    request["group"] = json!(group);
    request["voter_reference"] = json!(voter_reference);
    request
}

#[tokio::test]
async fn admin_attaches_a_group_and_voter_reference_at_creation() {
    let app = TestApp::new().await;

    let created = app
        .create_ballot(&request_for("Team survey 2030", "alice@example.org"))
        .await;

    assert_eq!(created["group"], "Team survey 2030", "{created}");
    assert_eq!(created["voter_reference"], "alice@example.org", "{created}");
}

#[tokio::test]
async fn group_and_voter_reference_are_optional() {
    let app = TestApp::new().await;

    let created = app.create_ballot(&ballot_request()).await;

    assert_eq!(created["group"], json!(null), "{created}");
    assert_eq!(created["voter_reference"], json!(null), "{created}");
}

#[tokio::test]
async fn a_duplicate_voter_reference_within_a_group_is_a_conflict() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    app.create_ballot(&request_for("Survey", "alice")).await;

    let response = app
        .post_json(
            "/admin/ballots",
            Some(&token),
            &request_for("Survey", "alice"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response).await;
    assert_eq!(body["error"], "duplicate_voter_reference", "{body}");
    assert!(
        !body.to_string().contains("alice") && !body.to_string().contains("Survey"),
        "the conflict must not echo stored values: {body}"
    );
    let listed = body_json(app.admin_get("/admin/ballots?group=Survey", &token).await).await;
    assert_eq!(
        listed["ballots"].as_array().unwrap().len(),
        1,
        "the refused Ballot must not be stored: {listed}"
    );
}

#[tokio::test]
async fn the_same_voter_reference_may_appear_in_different_groups() {
    let app = TestApp::new().await;

    app.create_ballot(&request_for("Survey A", "alice")).await;
    app.create_ballot(&request_for("Survey B", "alice")).await;
}

#[tokio::test]
async fn a_group_may_hold_many_ballots_with_different_or_no_voter_references() {
    let app = TestApp::new().await;
    let mut without_reference = ballot_request();
    without_reference["group"] = json!("Survey");

    app.create_ballot(&request_for("Survey", "alice")).await;
    app.create_ballot(&request_for("Survey", "bob")).await;
    app.create_ballot(&without_reference).await;
    app.create_ballot(&without_reference).await;
}

#[tokio::test]
async fn a_voter_reference_without_a_group_is_not_constrained() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["voter_reference"] = json!("alice");

    app.create_ballot(&request).await;
    app.create_ballot(&request).await;
}

#[tokio::test]
async fn voter_references_are_compared_after_normalisation() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    app.create_ballot(&request_for("Caf\u{e9}", "Ren\u{e9}"))
        .await;

    // Same Group and Voter Reference, decomposed and padded.
    let response = app
        .post_json(
            "/admin/ballots",
            Some(&token),
            &request_for(" Cafe\u{301} ", "\tRene\u{301}\t"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

/// Distinctive markers that must never reach the voter.
const GROUP_MARKER: &str = "GrpMarker7Q2xZ";
const REFERENCE_MARKER: &str = "RefMarker9K4wV";

/// Asserts that neither marker appears in `body` (case-insensitively, so an
/// upper-cased or lower-cased rendering would be caught too).
fn assert_no_markers(body: &str, context: &str) {
    let lower = body.to_lowercase();
    for marker in [GROUP_MARKER, REFERENCE_MARKER] {
        assert!(
            !lower.contains(&marker.to_lowercase()),
            "{context} leaks {marker}: {body}"
        );
    }
}

/// Every voter-facing body for the Ballot with this token: the voting page
/// and the voter API.
async fn voter_bodies(app: &TestApp, token: &str) -> Vec<(String, String)> {
    let page = body_string(app.get(&format!("/b/{token}")).await).await;
    let api = body_string(app.get(&format!("/api/b/{token}")).await).await;
    vec![("page".into(), page), ("voter API".into(), api)]
}

#[tokio::test]
async fn group_and_voter_reference_never_reach_the_voter() {
    let app = TestApp::new().await;
    let mut request = request_for(GROUP_MARKER, REFERENCE_MARKER);
    request["submission_mode"] = json!("editable");
    request["fallback"] = json!(["Pizza"]);
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap();
    let pizza = created["options"][0]["id"].as_str().unwrap();

    // Unanswered.
    for (surface, body) in voter_bodies(&app, token).await {
        assert_no_markers(&body, &format!("{surface} (unanswered)"));
    }

    // Submitting through the voter API and the no-JS form.
    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [pizza] }),
        )
        .await;
    assert_no_markers(&body_string(response).await, "voter API submission");
    let response = app
        .post_form(&format!("/b/{token}"), &format!("option={pizza}"))
        .await;
    assert_no_markers(&body_string(response).await, "form submission");

    // Answered, and after the deadline.
    for (surface, body) in voter_bodies(&app, token).await {
        assert_no_markers(&body, &format!("{surface} (answered)"));
    }
    app.clock.set("2030-01-23T12:00:00Z".parse().unwrap());
    for (surface, body) in voter_bodies(&app, token).await {
        assert_no_markers(&body, &format!("{surface} (closed)"));
    }
}

#[tokio::test]
async fn group_and_voter_reference_never_reach_the_voter_of_an_auto_fallback_ballot() {
    let app = TestApp::new().await;
    let mut request = request_for(GROUP_MARKER, REFERENCE_MARKER);
    request["fallback"] = json!(["Sushi"]);
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap();
    app.clock.set("2030-01-23T12:00:00Z".parse().unwrap());

    for (surface, body) in voter_bodies(&app, token).await {
        assert!(body.contains("auto_fallback") || surface == "page");
        assert_no_markers(&body, &surface);
    }
}
