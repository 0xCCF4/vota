//! The admin deletes a Ballot immediately (`DELETE /admin/ballots/{id}`):
//! afterwards it behaves exactly like one that never existed.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json, body_string, snapshot};
use serde_json::json;

#[tokio::test]
async fn a_deleted_ballot_is_gone_for_the_admin_and_gives_the_uniform_404_to_voters() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["submission_mode"] = json!("editable");
    request["fallback"] = json!(["Sushi"]);
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("alice");
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap();
    let pizza = &created["options"][0]["id"];
    app.put_json(
        &format!("/api/b/{token}/response"),
        &json!({ "option_ids": [pizza] }),
    )
    .await;
    let path = format!("/admin/ballots/{}", created["id"]);
    // A token that never existed, for byte-identical comparison.
    let unknown = "AAAAAAAAAAAAAAAAAAAAAA";

    let response = app.admin_delete(&path, &admin).await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(body_string(response).await, "");
    assert_eq!(
        app.admin_get(&path, &admin).await.status(),
        StatusCode::NOT_FOUND
    );
    for prefix in ["/b/", "/api/b/"] {
        let deleted = snapshot(app.get(&format!("{prefix}{token}")).await).await;
        let never = snapshot(app.get(&format!("{prefix}{unknown}")).await).await;
        assert_eq!(deleted.0, StatusCode::NOT_FOUND, "{prefix}");
        assert_eq!(deleted, never, "{prefix}");
    }
    let submit = app
        .put_json(
            &format!("/api/b/{token}/response"),
            // The revision the voter saw before the Ballot was deleted.
            &json!({ "option_ids": [pizza], "revision": 0 }),
        )
        .await;
    assert_eq!(
        snapshot(submit).await,
        snapshot(app.get(&format!("/api/b/{unknown}")).await).await
    );
    let listed = body_json(app.admin_get("/admin/ballots", &admin).await).await;
    assert_eq!(listed["ballots"], json!([]));
}

#[tokio::test]
async fn deleting_again_is_not_found() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    let path = format!("/admin/ballots/{}", created["id"]);
    app.admin_delete(&path, &admin).await;

    let response = app.admin_delete(&path, &admin).await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(response).await, json!({ "error": "not_found" }));
}

#[tokio::test]
async fn deleting_one_ballot_leaves_the_others_and_frees_its_voter_reference() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("alice");
    let doomed = app.create_ballot(&request).await;
    let kept = app.create_ballot(&ballot_request()).await;

    app.admin_delete(&format!("/admin/ballots/{}", doomed["id"]), &admin)
        .await;

    let fetched = app
        .admin_get(&format!("/admin/ballots/{}", kept["id"]), &admin)
        .await;
    assert_eq!(body_json(fetched).await, kept);
    app.create_ballot(&request).await;
}
