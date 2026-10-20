//! Admin results: `GET /admin/ballots` lists Ballots with their raw
//! per-Ballot results, optionally filtered by `group` and `status`,
//! unpaginated and in creation order.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// The `GET /admin/ballots{query}` response as JSON, asserting 200.
async fn list(app: &TestApp, query: &str) -> Value {
    let token = app.admin_token().await;
    let response = app
        .admin_get(&format!("/admin/ballots{query}"), &token)
        .await;
    assert_eq!(response.status(), StatusCode::OK, "for {query:?}");
    body_json(response).await
}

/// The Voter References of the listed Ballots, in listed order.
fn references(listed: &Value) -> Vec<&str> {
    listed["ballots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["voter_reference"].as_str().unwrap_or("<none>"))
        .collect()
}

/// Creates a Ballot in `group` (if any) for `reference` with a Fallback of
/// Sushi; returns the admin view.
async fn create(app: &TestApp, group: Option<&str>, reference: &str) -> Value {
    let mut request = ballot_request();
    if let Some(group) = group {
        request["group"] = json!(group);
    }
    request["voter_reference"] = json!(reference);
    request["fallback"] = json!(["Sushi"]);
    app.create_ballot(&request).await
}

#[tokio::test]
async fn admin_lists_every_ballot_in_creation_order_with_its_full_view() {
    let app = TestApp::new().await;
    let first = create(&app, Some("A"), "carol").await;
    let second = create(&app, None, "alice").await;
    let third = create(&app, Some("B"), "bob").await;

    let listed = list(&app, "").await;

    assert_eq!(listed, json!({ "ballots": [first, second, third] }));
}

#[tokio::test]
async fn an_empty_list_is_not_an_error() {
    let app = TestApp::new().await;

    assert_eq!(list(&app, "").await, json!({ "ballots": [] }));
}

#[tokio::test]
async fn admin_filters_the_list_by_group() {
    let app = TestApp::new().await;
    create(&app, Some("Survey A"), "alice").await;
    create(&app, Some("Survey B"), "bob").await;
    create(&app, None, "carol").await;
    create(&app, Some("Survey A"), "dave").await;

    assert_eq!(
        references(&list(&app, "?group=Survey%20A").await),
        ["alice", "dave"]
    );
    assert_eq!(references(&list(&app, "?group=Survey+B").await), ["bob"]);
    assert_eq!(references(&list(&app, "?group=Survey%20C").await), [""; 0]);
}

/// In Group "S": alice answered, bob has a Fallback but no Response, carol
/// has neither; dave (Group "T") answered.
async fn create_survey(app: &TestApp) {
    let alice = create(app, Some("S"), "alice").await;
    create(app, Some("S"), "bob").await;
    let mut carol = ballot_request();
    carol["group"] = json!("S");
    carol["voter_reference"] = json!("carol");
    app.create_ballot(&carol).await;
    let dave = create(app, Some("T"), "dave").await;
    for answered in [alice, dave] {
        let token = answered["token"].as_str().unwrap();
        let pizza = &answered["options"][0]["id"];
        let response = app
            .put_json(
                &format!("/api/b/{token}/response"),
                &json!({ "option_ids": [pizza] }),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn admin_filters_the_list_by_ballot_status_while_voting_is_open() {
    let app = TestApp::new().await;
    create_survey(&app).await;

    let by_status = |status| {
        let app = &app;
        async move { list(app, &format!("?status={status}")).await }
    };
    assert_eq!(references(&by_status("answered").await), ["alice", "dave"]);
    assert_eq!(references(&by_status("unanswered").await), ["bob", "carol"]);
    assert_eq!(references(&by_status("auto_fallback").await), [""; 0]);
}

#[tokio::test]
async fn the_status_filter_uses_the_ballot_status_at_the_clocks_now() {
    let app = TestApp::new().await;
    create_survey(&app).await;
    app.clock.set("2030-01-22T12:00:00Z".parse().unwrap());

    let auto_fallback = list(&app, "?status=auto_fallback").await;

    assert_eq!(references(&auto_fallback), ["bob"]);
    let bob = &auto_fallback["ballots"][0];
    assert_eq!(bob["status"], "auto_fallback");
    assert_eq!(bob["result"]["source"], "fallback");
    assert_eq!(bob["result"]["options"][0]["label"], "Sushi");
    assert_eq!(
        references(&list(&app, "?status=unanswered").await),
        ["carol"]
    );
    assert_eq!(
        references(&list(&app, "?status=answered").await),
        ["alice", "dave"]
    );
}

#[tokio::test]
async fn group_and_status_filters_combine() {
    let app = TestApp::new().await;
    create_survey(&app).await;

    let listed = list(&app, "?status=answered&group=S").await;

    assert_eq!(references(&listed), ["alice"]);
    let alice = &listed["ballots"][0];
    assert_eq!(alice["group"], "S");
    assert_eq!(alice["status"], "answered");
    assert_eq!(
        alice["result"]["options"],
        json!([{ "id": alice["options"][0]["id"], "label": "Pizza" }])
    );
    assert_eq!(alice["response"]["answered_at"], "2030-01-15T12:00:00Z");
    assert_eq!(alice["response"]["changed_at"], "2030-01-15T12:00:00Z");
    assert_eq!(alice["created_at"], "2030-01-15T12:00:00Z");
}
