//! The admin fetches one Ballot by its internal ID (`GET /admin/ballots/{id}`)
//! with its full configuration, Ballot Status, result and Response.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::json;

#[tokio::test]
async fn admin_fetches_a_ballot_by_id_with_its_full_configuration() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let mut request = ballot_request();
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("alice");
    request["fallback"] = json!(["Sushi"]);
    let created = app.create_ballot(&request).await;

    let response = app
        .admin_get(&format!("/admin/ballots/{}", created["id"]), &token)
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let fetched = body_json(response).await;
    assert_eq!(fetched, created);
    assert_eq!(fetched["token"].as_str().unwrap().len(), 22);
    assert_eq!(
        fetched["url"],
        format!(
            "https://ballots.example.org/b/{}",
            fetched["token"].as_str().unwrap()
        )
    );
    assert_eq!(fetched["group"], "Survey");
    assert_eq!(fetched["voter_reference"], "alice");
    assert_eq!(fetched["status"], "unanswered");
    assert_eq!(fetched["purge_date"], "2030-02-19T12:00:00Z");
}

#[tokio::test]
async fn admin_sees_the_response_with_when_it_was_given_and_last_changed() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let mut request = ballot_request();
    request["submission_mode"] = json!("editable");
    let created = app.create_ballot(&request).await;
    let ballot_token = created["token"].as_str().unwrap();
    let (pizza, sushi) = (&created["options"][0]["id"], &created["options"][1]["id"]);
    let path = format!("/api/b/{ballot_token}/response");
    app.clock.set("2030-01-16T08:00:00Z".parse().unwrap());
    app.put_json(&path, &json!({ "option_ids": [pizza] })).await;
    app.clock.set("2030-01-17T09:30:00Z".parse().unwrap());
    app.put_json(&path, &json!({ "option_ids": [sushi] })).await;

    let fetched = body_json(
        app.admin_get(&format!("/admin/ballots/{}", created["id"]), &token)
            .await,
    )
    .await;

    assert_eq!(fetched["status"], "answered", "{fetched}");
    assert_eq!(
        fetched["response"],
        json!({
            "option_ids": [sushi],
            "answered_at": "2030-01-16T08:00:00Z",
            "changed_at": "2030-01-17T09:30:00Z",
            "origin": "voter",
        }),
        "{fetched}"
    );
    assert_eq!(
        fetched["result"],
        json!({
            "source": "response",
            "option_ids": [sushi],
            "options": [{ "id": sushi, "label": "Sushi" }],
        }),
        "{fetched}"
    );
}

#[tokio::test]
async fn an_auto_fallback_result_names_the_fallback_options_with_their_labels() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let mut request = ballot_request();
    request["options"] = json!([{ "label": "Mon" }, { "label": "Tue" }, { "label": "Wed" }]);
    request["choice_mode"] = json!("multiple");
    request["fallback"] = json!(["Wed", "Mon"]);
    let created = app.create_ballot(&request).await;
    let ids: Vec<_> = (0..3)
        .map(|i| created["options"][i]["id"].clone())
        .collect();
    app.clock.set("2030-01-22T12:00:00Z".parse().unwrap());

    let fetched = body_json(
        app.admin_get(&format!("/admin/ballots/{}", created["id"]), &token)
            .await,
    )
    .await;

    assert_eq!(fetched["status"], "auto_fallback", "{fetched}");
    assert_eq!(fetched["response"], json!(null), "{fetched}");
    assert_eq!(
        fetched["result"],
        json!({
            "source": "fallback",
            "option_ids": [ids[0], ids[2]],
            "options": [
                { "id": ids[0], "label": "Mon" },
                { "id": ids[2], "label": "Wed" },
            ],
        }),
        "{fetched}"
    );
}

#[tokio::test]
async fn unknown_and_malformed_ballot_ids_are_the_same_not_found() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    assert_eq!(created["id"], 1);
    let expected = common::snapshot(app.admin_get("/admin/ballots/2", &token).await).await;
    assert_eq!(expected.0, StatusCode::NOT_FOUND);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&expected.2).unwrap(),
        json!({ "error": "not_found" })
    );

    let malformed = [
        "0",
        "-1",
        "+1",
        "01",
        "%201",
        "1%20",
        "1.0",
        "1e0",
        "0x1",
        "abc",
        "%D9%A1",    // ARABIC-INDIC DIGIT ONE
        "%EF%BC%91", // FULLWIDTH DIGIT ONE
        "9223372036854775808",
        "99999999999999999999999999",
        "%00",
        "1%00",
        "..%2F1",
        "%2E%2E",
        "%E2%80%AE1",
        "%FF%FE",
        "999",
    ];
    for id in malformed {
        let response = app.admin_get(&format!("/admin/ballots/{id}"), &token).await;
        assert_eq!(common::snapshot(response).await, expected, "for ID {id:?}");
    }
    let long = "1".repeat(10_000);
    let response = app
        .admin_get(&format!("/admin/ballots/{long}"), &token)
        .await;
    assert_eq!(common::snapshot(response).await, expected);
}

#[tokio::test]
async fn fetching_a_ballot_needs_a_valid_admin_token() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&ballot_request()).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    assert_eq!(app.get(&path).await.status(), StatusCode::UNAUTHORIZED);
    let response = app.admin_get(&path, "vta_wrong").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn percent_encoded_ballot_ids_are_not_found_on_every_admin_ballot_route() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    assert_eq!(created["id"], 1);
    let not_found = json!({ "error": "not_found" });

    for id in ["%31", "%31%30", "1%30"] {
        let path = format!("/admin/ballots/{id}");
        let fetched = app.admin_get(&path, &token).await;
        assert_eq!(fetched.status(), StatusCode::NOT_FOUND, "GET {path}");
        assert_eq!(body_json(fetched).await, not_found, "GET {path}");

        let edited = app
            .admin_patch(&path, &token, &json!({ "title": "Changed" }))
            .await;
        assert_eq!(edited.status(), StatusCode::NOT_FOUND, "PATCH {path}");
        assert_eq!(body_json(edited).await, not_found, "PATCH {path}");

        let extended = app
            .post_json(
                &format!("{path}/extend"),
                Some(&token),
                &json!({ "voting_deadline": "2030-01-29T12:00:00Z" }),
            )
            .await;
        assert_eq!(extended.status(), StatusCode::NOT_FOUND, "extend {path}");
        assert_eq!(body_json(extended).await, not_found, "extend {path}");

        let deleted = app.admin_delete(&path, &token).await;
        assert_eq!(deleted.status(), StatusCode::NOT_FOUND, "DELETE {path}");
        assert_eq!(body_json(deleted).await, not_found, "DELETE {path}");
    }
    // The Ballot itself is untouched and still reachable by its literal ID.
    let fetched = body_json(app.admin_get("/admin/ballots/1", &token).await).await;
    assert_eq!(fetched, created);
}
