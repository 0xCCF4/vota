//! A request body that is not valid JSON at all (truncated, empty, not
//! UTF-8, trailing garbage, unrepresentable numbers or strings) gets one fixed 422 `invalid_json` on the admin
//! and voter APIs, without a field and without echoing the body.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, body_json};
use serde_json::json;

fn invalid_json() -> serde_json::Value {
    json!({
        "error": "invalid_json",
        "message": "the request body is not valid JSON",
    })
}

#[tokio::test]
async fn a_truncated_body_to_the_admin_api_is_invalid_json() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;

    let response = app
        .post_raw(
            "/admin/ballots",
            Some(&admin),
            "application/json",
            r#"{"title": "Lunch on Friday?", "options": [{"label": "Piz"#,
        )
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(response).await, invalid_json());
}

/// Bodies that are not valid JSON: truncated in several places, empty,
/// whitespace only, invalid UTF-8 inside and outside a string, trailing data.
fn broken_bodies() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("truncated object", br#"{"#.to_vec()),
        ("truncated key", br#"{"option_"#.to_vec()),
        ("truncated after colon", br#"{"option_ids":"#.to_vec()),
        ("truncated array", br#"{"option_ids": ["#.to_vec()),
        ("truncated string", br#"{"title": "Lunch"#.to_vec()),
        ("empty", Vec::new()),
        ("whitespace only", b" \n\t ".to_vec()),
        (
            "invalid UTF-8 in a string",
            b"{\"title\": \"Lunch \xff\xfe\"}".to_vec(),
        ),
        ("invalid UTF-8 outside a string", b"\xff\xfe{}".to_vec()),
        ("trailing data", br#"{} {}"#.to_vec()),
        ("not JSON", b"title=Lunch".to_vec()),
        // Well-formed text that no JSON parser here can represent.
        ("number out of range", br#"{"option_ids": 1e400}"#.to_vec()),
        ("lone surrogate escape", br#"{"title": "\ud800"}"#.to_vec()),
    ]
}

#[tokio::test]
async fn broken_bodies_to_the_admin_api_are_invalid_json() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let id = app.create_ballot(&common::ballot_request()).await["id"].clone();

    for (what, body) in broken_bodies() {
        let responses = [
            (
                "create",
                app.post_raw(
                    "/admin/ballots",
                    Some(&admin),
                    "application/json",
                    body.clone(),
                )
                .await,
            ),
            (
                "edit",
                app.admin_patch_raw(
                    &format!("/admin/ballots/{id}"),
                    &admin,
                    "application/json",
                    body.clone(),
                )
                .await,
            ),
            (
                "extend",
                app.post_raw(
                    &format!("/admin/ballots/{id}/extend"),
                    Some(&admin),
                    "application/json",
                    body.clone(),
                )
                .await,
            ),
        ];
        for (endpoint, response) in responses {
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{endpoint}: {what}"
            );
            assert_eq!(
                body_json(response).await,
                invalid_json(),
                "{endpoint}: {what}"
            );
        }
    }
}

#[tokio::test]
async fn broken_bodies_to_the_voter_api_are_invalid_json_and_record_nothing() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;

    for (what, body) in broken_bodies() {
        let response = app
            .put_raw(
                &format!("/api/b/{token}/response"),
                "application/json",
                body,
            )
            .await;

        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{what}"
        );
        assert_eq!(body_json(response).await, invalid_json(), "{what}");
    }
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn valid_json_of_the_wrong_shape_still_names_the_field() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;

    for (body, field, message) in [
        (r#"[]"#, json!(""), "must be a JSON object"),
        (
            r#"{"option_ids": "x"}"#,
            json!("option_ids"),
            "wrong type or value",
        ),
        (
            r#"{"option_ids": [1]}"#,
            json!("option_ids[0]"),
            "wrong type or value",
        ),
        (r#"{"bogus": 1}"#, json!(null), "unknown field"),
    ] {
        let response = app
            .put_raw(
                &format!("/api/b/{token}/response"),
                "application/json",
                body,
            )
            .await;

        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
        assert_eq!(
            body_json(response).await,
            json!({ "error": "invalid_input", "field": field, "message": message }),
            "{body}"
        );
    }
}

#[tokio::test]
async fn wrong_content_type_and_oversized_bodies_keep_their_statuses() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;
    let path = format!("/api/b/{token}/response");

    let wrong_type = app.put_raw(&path, "text/plain", "{").await;
    let oversized = app
        .put_raw(&path, "application/json", "[".repeat(1024 * 1024))
        .await;

    assert_eq!(wrong_type.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
