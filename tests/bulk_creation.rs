//! Bulk creation: `POST /admin/ballots` with a list of Voter References
//! (one Ballot each) or a `count` (numbered Voter References in a Group),
//! all or nothing, answered with `{"ballots": [...]}`.

#![cfg(feature = "server")]

mod common;

use std::collections::HashSet;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// The [`ballot_request`] with these extra fields.
fn request_with(extra: Value) -> Value {
    let mut request = ballot_request();
    for (key, value) in extra.as_object().unwrap() {
        request[key] = value.clone();
    }
    request
}

/// POSTs the request; returns the status and the JSON body.
async fn create(app: &TestApp, request: &Value) -> (StatusCode, Value) {
    let token = app.admin_token().await;
    let response = app.post_json("/admin/ballots", Some(&token), request).await;
    let status = response.status();
    (status, body_json(response).await)
}

/// The Voter References of the Ballots in this Group, in creation order.
async fn references_in(app: &TestApp, group: &str) -> Vec<String> {
    let token = app.admin_token().await;
    let path = format!("/admin/ballots?group={}", group.replace(' ', "+"));
    let listed = body_json(app.admin_get(&path, &token).await).await;
    listed["ballots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ballot| ballot["voter_reference"].as_str().unwrap().to_owned())
        .collect()
}

fn voter_references(created: &Value) -> Vec<&str> {
    created["ballots"]
        .as_array()
        .unwrap_or_else(|| panic!("no ballots list: {created}"))
        .iter()
        .map(|ballot| ballot["voter_reference"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn a_list_of_voter_references_creates_one_ballot_each_sharing_the_other_fields() {
    let app = TestApp::new().await;

    let (status, created) = create(
        &app,
        &request_with(json!({ "group": "Team", "voter_reference": ["alice", "bob", "carol"] })),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(voter_references(&created), ["alice", "bob", "carol"]);
    let ballots = created["ballots"].as_array().unwrap();
    let mut tokens = HashSet::new();
    let mut ids = HashSet::new();
    for ballot in ballots {
        assert_eq!(ballot["title"], "Lunch on Friday?", "{ballot}");
        assert_eq!(ballot["group"], "Team", "{ballot}");
        assert_eq!(ballot["status"], "unanswered", "{ballot}");
        let token = ballot["token"].as_str().unwrap();
        assert_eq!(
            ballot["url"],
            format!("https://ballots.example.org/b/{token}")
        );
        tokens.insert(token.to_owned());
        ids.insert(ballot["id"].as_i64().unwrap());
    }
    assert_eq!(tokens.len(), 3, "each Ballot has its own Ballot Token");
    assert_eq!(ids.len(), 3);
    assert_eq!(references_in(&app, "Team").await, ["alice", "bob", "carol"]);
}

#[tokio::test]
async fn the_answer_has_the_shape_of_the_request_form_even_for_one_entry() {
    let app = TestApp::new().await;

    let (status, single) = create(&app, &request_with(json!({ "voter_reference": "dave" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(single["voter_reference"], "dave", "{single}");
    assert!(single.get("ballots").is_none(), "{single}");

    let (status, listed) =
        create(&app, &request_with(json!({ "voter_reference": ["erin"] }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(voter_references(&listed), ["erin"]);
    assert_eq!(
        listed.as_object().unwrap().len(),
        1,
        "only `ballots`: {listed}"
    );
}

#[tokio::test]
async fn a_list_without_a_group_creates_ballots_without_a_group() {
    let app = TestApp::new().await;

    let (status, created) = create(
        &app,
        &request_with(json!({ "voter_reference": ["a", "b"] })),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(voter_references(&created), ["a", "b"]);
    assert_eq!(created["ballots"][0]["group"], json!(null));
}

#[tokio::test]
async fn count_numbers_from_one_in_an_empty_group() {
    let app = TestApp::new().await;

    let (status, created) =
        create(&app, &request_with(json!({ "group": "Hall", "count": 3 }))).await;

    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(voter_references(&created), ["1", "2", "3"]);
}

#[tokio::test]
async fn count_continues_after_the_highest_purely_numeric_reference_of_the_group() {
    let app = TestApp::new().await;
    let (status, _) = create(
        &app,
        &request_with(json!({
            "group": "Hall",
            "voter_reference": ["3", "007", "12", "alice", "99a", "-50", "+40", "1e9", "١٠٠", "２０"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    // Another Group's numbers don't count.
    create(
        &app,
        &request_with(json!({ "group": "Other", "voter_reference": "500" })),
    )
    .await;

    let (status, created) =
        create(&app, &request_with(json!({ "group": "Hall", "count": 2 }))).await;

    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(voter_references(&created), ["13", "14"]);
    let (_, again) = create(&app, &request_with(json!({ "group": "Hall", "count": 1 }))).await;
    assert_eq!(voter_references(&again), ["15"]);
}

#[tokio::test]
async fn count_requires_a_group_and_excludes_voter_reference() {
    let app = TestApp::new().await;
    for (extra, message) in [
        (json!({ "count": 2 }), "requires a group"),
        (
            json!({ "group": "G", "count": 2, "voter_reference": "x" }),
            "can't be combined with voter_reference",
        ),
        (
            json!({ "group": "G", "count": 2, "voter_reference": ["x"] }),
            "can't be combined with voter_reference",
        ),
        (json!({ "group": "G", "count": 0 }), "must be at least 1"),
        (json!({ "group": "G", "count": -3 }), "must be at least 1"),
        (
            json!({ "group": "G", "count": 1001 }),
            "at most 1000 Ballots per request",
        ),
    ] {
        let (status, body) = create(&app, &request_with(extra.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {body}");
        assert_eq!(body["field"], "count", "{extra}: {body}");
        assert_eq!(body["message"], message, "{extra}: {body}");
    }
    for extra in [
        json!({ "group": "G", "count": 2.5 }),
        json!({ "group": "G", "count": "2" }),
        json!({ "group": "G", "count": null }),
    ] {
        let (status, body) = create(&app, &request_with(extra.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {body}");
        assert_eq!(body["field"], "count", "{extra}: {body}");
    }
    assert!(references_in(&app, "G").await.is_empty());
}

#[tokio::test]
async fn a_list_is_limited_to_1000_distinct_non_empty_entries() {
    let app = TestApp::new().await;
    let thousand_and_one: Vec<String> = (0..1001).map(|i| format!("v{i}")).collect();
    for (references, field, message) in [
        (
            json!([]),
            "voter_reference",
            "at least one Voter Reference is required",
        ),
        (
            json!(thousand_and_one),
            "voter_reference",
            "at most 1000 Voter References per request",
        ),
        (
            json!(["a", "b", "a"]),
            "voter_reference[2]",
            "duplicate Voter Reference",
        ),
        // Compared after normalisation, like stored ones.
        (
            json!(["Zoë", "Zoe\u{308}"]),
            "voter_reference[1]",
            "duplicate Voter Reference",
        ),
        (json!(["a", ""]), "voter_reference[1]", "must not be empty"),
        (json!(["a", 3]), "voter_reference", "wrong type or value"),
    ] {
        let request = request_with(json!({ "group": "G", "voter_reference": references }));
        let (status, body) = create(&app, &request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["field"], field, "{body}");
        assert_eq!(body["message"], message, "{body}");
    }
    assert!(references_in(&app, "G").await.is_empty());
}

#[tokio::test]
async fn a_thousand_references_fit_the_body_limit_and_are_created() {
    let app = TestApp::new().await;
    // 1000 references of 40 characters: about 43 KiB of JSON.
    let references: Vec<String> = (0..1000)
        .map(|i| format!("voter-{i:04}@departments.example.org..."))
        .collect();
    let request = request_with(json!({ "group": "Big", "voter_reference": references }));
    assert!(request.to_string().len() < 64 * 1024);

    let (status, created) = create(&app, &request).await;

    assert_eq!(status, StatusCode::CREATED);
    let tokens: HashSet<_> = (created["ballots"].as_array().unwrap().iter())
        .map(|ballot| ballot["token"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(tokens.len(), 1000);
    let (status, _) = create(
        &app,
        &request_with(json!({ "group": "Big2", "count": 1000 })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn a_clash_with_existing_references_creates_nothing_and_names_the_clashes() {
    let app = TestApp::new().await;
    create(
        &app,
        &request_with(json!({ "group": "Team", "voter_reference": ["bob", "dave"] })),
    )
    .await;

    let (status, body) = create(
        &app,
        &request_with(
            json!({ "group": "Team", "voter_reference": ["alice", "dave", "carol", "bob"] }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "duplicate_voter_reference", "{body}");
    assert_eq!(body["voter_references"], json!(["dave", "bob"]), "{body}");
    assert_eq!(references_in(&app, "Team").await, ["bob", "dave"]);
    // The same references in another Group are fine.
    let (status, _) = create(
        &app,
        &request_with(
            json!({ "group": "Other", "voter_reference": ["alice", "dave", "carol", "bob"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn a_purged_ballots_reference_is_free_and_its_number_does_not_count() {
    let app = TestApp::new().await;
    let mut request = request_with(json!({ "group": "Old", "voter_reference": ["7", "x"] }));
    request["purge_date"] = json!("2030-01-22T12:00:00Z");
    create(&app, &request).await;
    app.clock.advance(chrono::Duration::days(3650));

    let mut later = request_with(json!({ "group": "Old", "count": 1 }));
    later["voting_deadline"] = json!("2040-01-22T12:00:00Z");
    let (status, created) = create(&app, &later).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(voter_references(&created), ["1"]);
    let mut reuse = request_with(json!({ "group": "Old", "voter_reference": ["x", "7"] }));
    reuse["voting_deadline"] = json!("2040-01-22T12:00:00Z");
    let (status, created) = create(&app, &reuse).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
}
