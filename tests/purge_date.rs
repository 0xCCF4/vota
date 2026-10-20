//! Purge Date: from its Purge Date on, a Ballot is gone on every surface
//! (request-time check), and the purge sweep deletes it for good. The sweep
//! runs once through `jobs::purge_sweep::run_once`, by the app's Clock.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use common::{TestApp, ballot_request, snapshot};
use serde_json::{Value, json};
use vota::jobs::purge_sweep;

/// The Purge Date of [`purgeable`] Ballots.
fn purge_date() -> DateTime<Utc> {
    "2030-02-01T00:00:00Z".parse().unwrap()
}

/// One millisecond before [`purge_date`].
fn just_before_purge() -> DateTime<Utc> {
    purge_date() - Duration::milliseconds(1)
}

/// A create request for an Editable Ballot whose Purge Date is
/// [`purge_date`] (Voting Deadline 2030-01-22T12:00:00Z).
fn purgeable() -> Value {
    let mut request = ballot_request();
    request["submission_mode"] = json!("editable");
    request["purge_date"] = json!("2030-02-01T00:00:00Z");
    request
}

/// A well-formed Ballot Token that never existed.
const UNKNOWN: &str = "AAAAAAAAAAAAAAAAAAAAAA";

#[tokio::test]
async fn the_voting_page_is_the_uniform_404_from_the_purge_date_on() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&purgeable()).await;
    let path = format!("/b/{}", created["token"].as_str().unwrap());

    app.clock.set(just_before_purge());
    assert_eq!(app.get(&path).await.status(), StatusCode::OK);

    app.clock.set(purge_date());
    assert_eq!(
        snapshot(app.get(&path).await).await,
        snapshot(app.get(&format!("/b/{UNKNOWN}")).await).await
    );
}

#[tokio::test]
async fn the_voter_api_is_the_uniform_404_from_the_purge_date_on() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&purgeable()).await;
    let path = format!("/api/b/{}", created["token"].as_str().unwrap());

    app.clock.set(just_before_purge());
    assert_eq!(app.get(&path).await.status(), StatusCode::OK);

    app.clock.set(purge_date());
    assert_eq!(
        snapshot(app.get(&path).await).await,
        snapshot(app.get(&format!("/api/b/{UNKNOWN}")).await).await
    );
}

#[tokio::test]
async fn submitting_to_a_purged_ballot_is_the_uniform_404_on_both_surfaces() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&purgeable()).await;
    let token = created["token"].as_str().unwrap();
    let pizza = created["options"][0]["id"].as_str().unwrap();
    app.clock.set(purge_date());

    // With the revision the voter saw before the Purge Date.
    let put = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [pizza], "revision": 0 }),
        )
        .await;
    let post = app
        .post_form(
            &format!("/b/{token}"),
            &format!("option={pizza}&revision=0"),
        )
        .await;

    assert_eq!(
        snapshot(put).await,
        snapshot(app.get(&format!("/api/b/{UNKNOWN}")).await).await
    );
    assert_eq!(
        snapshot(post).await,
        snapshot(app.get(&format!("/b/{UNKNOWN}")).await).await
    );
}

#[tokio::test]
async fn the_admin_api_finds_no_ballot_from_the_purge_date_on() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&purgeable()).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    app.clock.set(just_before_purge());
    assert_eq!(app.admin_get(&path, &admin).await.status(), StatusCode::OK);

    app.clock.set(purge_date());
    let response = app.admin_get(&path, &admin).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        common::body_json(response).await,
        json!({ "error": "not_found" })
    );
}

#[tokio::test]
async fn a_purged_ballot_can_be_neither_edited_nor_extended() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&purgeable()).await;
    let path = format!("/admin/ballots/{}", created["id"]);
    let edit = json!({ "title": "Too late" });
    let extension = json!({ "voting_deadline": "2030-06-01T00:00:00Z" });

    app.clock.set(just_before_purge());
    let response = app.admin_patch(&path, &admin, &edit).await;
    assert_eq!(response.status(), StatusCode::OK);

    app.clock.set(purge_date());
    let edited = app.admin_patch(&path, &admin, &edit).await;
    assert_eq!(edited.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        common::body_json(edited).await,
        json!({ "error": "not_found" })
    );
    let extended = app
        .post_json(&format!("{path}/extend"), Some(&admin), &extension)
        .await;
    assert_eq!(extended.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        common::body_json(extended).await,
        json!({ "error": "not_found" })
    );
    // Neither brought it back.
    assert_eq!(
        app.admin_get(&path, &admin).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn extending_just_before_the_purge_date_keeps_the_ballot() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&purgeable()).await;
    let path = format!("/admin/ballots/{}", created["id"]);
    app.clock.set(just_before_purge());

    let extended = app
        .post_json(
            &format!("{path}/extend"),
            Some(&admin),
            &json!({ "voting_deadline": "2030-06-01T00:00:00Z" }),
        )
        .await;

    assert_eq!(extended.status(), StatusCode::OK);
    app.clock.set(purge_date());
    assert_eq!(app.admin_get(&path, &admin).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_admin_list_leaves_out_purged_ballots() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let purged = app.create_ballot(&purgeable()).await;
    let mut later = ballot_request();
    later["purge_date"] = json!("2030-02-01T00:00:00.001Z");
    let kept = app.create_ballot(&later).await;

    app.clock.set(just_before_purge());
    let listed = common::body_json(app.admin_get("/admin/ballots", &admin).await).await;
    assert_eq!(ids(&listed), vec![purged["id"].clone(), kept["id"].clone()]);

    app.clock.set(purge_date());
    let listed = common::body_json(app.admin_get("/admin/ballots", &admin).await).await;
    assert_eq!(ids(&listed), vec![kept["id"].clone()]);
}

/// The IDs of the listed Ballots, in order.
fn ids(listed: &Value) -> Vec<Value> {
    listed["ballots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ballot| ballot["id"].clone())
        .collect()
}

#[tokio::test]
async fn deleting_a_purged_ballot_is_not_found() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&purgeable()).await;
    let path = format!("/admin/ballots/{}", created["id"]);
    app.clock.set(purge_date());

    let response = app.admin_delete(&path, &admin).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        common::body_json(response).await,
        json!({ "error": "not_found" })
    );
}

#[tokio::test]
async fn deleting_just_before_the_purge_date_works() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&purgeable()).await;
    let path = format!("/admin/ballots/{}", created["id"]);
    app.clock.set(just_before_purge());

    let response = app.admin_delete(&path, &admin).await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

// --- the purge sweep ------------------------------------------------------

#[tokio::test]
async fn the_sweep_deletes_purged_ballots_for_good() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&purgeable()).await;
    let token = created["token"].as_str().unwrap();
    let pizza = &created["options"][0]["id"];
    app.put_json(
        &format!("/api/b/{token}/response"),
        &json!({ "option_ids": [pizza] }),
    )
    .await;
    app.clock.set(purge_date());

    let swept = purge_sweep::run_once(&app.state).await.unwrap();

    assert_eq!(swept, 1);
    // Even with the Clock back before the Purge Date, it stays gone: it was
    // deleted, not just hidden.
    app.clock.set(just_before_purge());
    let path = format!("/admin/ballots/{}", created["id"]);
    assert_eq!(
        app.admin_get(&path, &admin).await.status(),
        StatusCode::NOT_FOUND
    );
    let listed = common::body_json(app.admin_get("/admin/ballots", &admin).await).await;
    assert_eq!(listed["ballots"], json!([]));
    for prefix in ["/b/", "/api/b/"] {
        assert_eq!(
            snapshot(app.get(&format!("{prefix}{token}")).await).await,
            snapshot(app.get(&format!("{prefix}{UNKNOWN}")).await).await,
            "{prefix}"
        );
    }
}

#[tokio::test]
async fn the_sweep_leaves_ballots_before_their_purge_date_untouched() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    app.create_ballot(&purgeable()).await;
    // Purged one millisecond after the sweep runs.
    let mut later = purgeable();
    later["purge_date"] = json!("2030-02-01T00:00:00.001Z");
    later["group"] = json!("Survey");
    later["voter_reference"] = json!("alice");
    later["fallback"] = json!(["Sushi"]);
    let kept = app.create_ballot(&later).await;
    let token = kept["token"].as_str().unwrap();
    let pizza = &kept["options"][0]["id"];
    app.put_json(
        &format!("/api/b/{token}/response"),
        &json!({ "option_ids": [pizza] }),
    )
    .await;
    let path = format!("/admin/ballots/{}", kept["id"]);
    app.clock.set(purge_date());
    let before = common::body_json(app.admin_get(&path, &admin).await).await;
    let voter_before = app.voter_view(token).await;

    let swept = purge_sweep::run_once(&app.state).await.unwrap();

    assert_eq!(swept, 1);
    assert_eq!(
        common::body_json(app.admin_get(&path, &admin).await).await,
        before
    );
    assert_eq!(app.voter_view(token).await, voter_before);
    assert_eq!(before["response"]["option_ids"], json!([pizza]));
}

#[tokio::test]
async fn sweeping_with_nothing_purged_deletes_nothing() {
    let app = TestApp::new().await;
    app.create_ballot(&purgeable()).await;
    app.clock.set(just_before_purge());

    let swept = purge_sweep::run_once(&app.state).await.unwrap();

    assert_eq!(swept, 0);
    let admin = app.admin_token().await;
    let listed = common::body_json(app.admin_get("/admin/ballots", &admin).await).await;
    assert_eq!(listed["ballots"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_purged_voter_reference_can_be_used_again_before_and_after_the_sweep() {
    let app = TestApp::new().await;
    let mut request = purgeable();
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("alice");
    request["purge_date"] = json!("2030-01-22T12:00:00Z");
    request["voting_deadline"] = json!("2030-01-22T12:00:00Z");
    app.create_ballot(&request).await;
    app.clock.set("2030-01-22T12:00:00Z".parse().unwrap());
    request["voting_deadline"] = json!("2030-03-01T00:00:00Z");
    request["purge_date"] = json!("2030-03-01T00:00:00Z");

    // Purged but not yet swept: as if it never existed.
    let second = app.create_ballot(&request).await;
    app.clock.set("2030-03-01T00:00:00Z".parse().unwrap());
    purge_sweep::run_once(&app.state).await.unwrap();
    app.clock.set("2030-03-01T00:00:01Z".parse().unwrap());
    request["voting_deadline"] = json!("2030-04-01T00:00:00Z");
    request["purge_date"] = json!("2030-04-01T00:00:00Z");
    let third = app.create_ballot(&request).await;

    assert_eq!(second["voter_reference"], "alice");
    assert_eq!(third["voter_reference"], "alice");
}

#[tokio::test]
async fn an_edit_can_take_the_voter_reference_of_a_purged_ballot() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut request = purgeable();
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("alice");
    app.create_ballot(&request).await;
    let mut later = purgeable();
    later["voting_deadline"] = json!("2030-03-01T00:00:00Z");
    later["purge_date"] = json!("2030-03-01T00:00:00Z");
    let other = app.create_ballot(&later).await;
    app.clock.set(purge_date());

    let edited = app
        .admin_patch(
            &format!("/admin/ballots/{}", other["id"]),
            &admin,
            &json!({ "group": "Survey", "voter_reference": "alice" }),
        )
        .await;

    assert_eq!(edited.status(), StatusCode::OK);
    assert_eq!(common::body_json(edited).await["voter_reference"], "alice");
}
