//! Hostile and invalid input to `PUT /api/b/{token}/response`, through the
//! HTTP seam. Every rejection is a 422 naming the offending field, and nothing
//! is recorded.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, assert_response_rejected, body_json};
use serde_json::{Value, json};

/// Strings that are not even shaped like a server-generated Option ID.
fn malformed_option_ids(real: &str) -> Vec<String> {
    vec![
        String::new(),
        "short".into(),
        format!("{real}A"),
        real[..10].to_owned(),
        // 11 characters, but outside the base64url alphabet.
        "AAAAAAAAAA+".into(),
        "AAAAAAAAAA/".into(),
        "AAAAAAAAAA=".into(),
        // 11 base64url characters whose last one carries non-zero padding bits.
        "AAAAAAAAAAB".into(),
        "../../etc/p".into(),
        "..%2F..%2Fet".into(),
        "AAAAAAAAA\u{0}A".into(),
        "AAAAAAAAA\u{202E}".into(),
        "AAAAAAAAA\u{200B}A".into(),
        format!(" {}", &real[1..]),
        "A".repeat(10_000),
    ]
}

#[tokio::test]
async fn malformed_option_ids_are_rejected_by_format() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    for id in malformed_option_ids(&options[0]) {
        let body = json!({ "option_ids": [id] });
        assert_response_rejected(&app, &token, &body, "option_ids[0]").await;
        let response = app
            .put_json(&format!("/api/b/{token}/response"), &body)
            .await;
        assert_eq!(
            body_json(response).await["message"],
            "not an Option ID",
            "for {body}"
        );
    }
}

#[tokio::test]
async fn option_ids_of_wrong_type_or_missing_are_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    for (body, field) in [
        (json!({ "option_ids": options[0] }), "option_ids"),
        (json!({ "option_ids": null }), "option_ids"),
        (json!({ "option_ids": {} }), "option_ids"),
        (json!({ "option_ids": [42] }), "option_ids[0]"),
        (json!({ "option_ids": [null] }), "option_ids[0]"),
        (json!({ "option_ids": [[options[0]]] }), "option_ids[0]"),
        (
            json!({ "option_ids": [{ "id": options[0] }] }),
            "option_ids[0]",
        ),
        (json!({}), "option_ids"),
    ] {
        assert_response_rejected(&app, &token, &body, field).await;
    }
}

#[tokio::test]
async fn unknown_fields_are_rejected_without_echoing_their_names() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    for field in ["status", "answered_at", "__proto__", "ZZMARK<script>ZZ"] {
        let mut body = json!({ "option_ids": [options[0]], "revision": 0 });
        body[field] = json!("x");
        let response = app
            .put_json(&format!("/api/b/{token}/response"), &body)
            .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let text = common::body_string(response).await;
        let error: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            error,
            json!({ "error": "invalid_input", "field": null, "message": "unknown field" })
        );
        assert!(!text.contains(field), "{text}");
    }
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn malformed_json_is_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let valid = json!({ "option_ids": [options[0]] }).to_string();
    for body in [
        String::new(),
        "{".into(),
        "[]".into(),
        "null".into(),
        format!("{valid} {{}}"),
        valid.replace(&options[0], "\\ud800"),
    ] {
        let response = app
            .put_raw(
                &format!("/api/b/{token}/response"),
                "application/json",
                body.clone(),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "for {body:?}"
        );
    }
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn non_json_bodies_are_refused() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let path = format!("/api/b/{token}/response");
    for content_type in ["text/plain", "application/x-www-form-urlencoded", ""] {
        let response = app
            .put_raw(&path, content_type, format!("option_ids={}", options[0]))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{content_type}"
        );
    }
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn bodies_over_64_kib_are_refused() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let padding = vec![options[0].clone(); 7_000];
    let body = json!({ "option_ids": padding });
    assert!(body.to_string().len() > 64 * 1024);

    let response = app
        .put_json(&format!("/api/b/{token}/response"), &body)
        .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn rejections_never_echo_the_rejected_value() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;
    let marker = "AAAAAAAAAAB<script>";
    for body in [
        json!({ "option_ids": [marker] }),
        json!({ "option_ids": marker }),
    ] {
        let response = app
            .put_json(&format!("/api/b/{token}/response"), &body)
            .await;
        let text: Value = body_json(response).await;
        assert!(!text.to_string().contains("script"), "{text}");
    }
}

#[tokio::test]
async fn unknown_and_malformed_tokens_get_the_uniform_not_found() {
    let app = TestApp::new().await;
    let (real, options) = app.create_standard_ballot().await;
    let unknown = if real.starts_with('A') { "B" } else { "A" }.to_owned() + &real[1..];
    let body = json!({ "option_ids": [options[0]], "revision": 0 });
    let expected = common::snapshot(app.get(&format!("/api/b/{unknown}")).await).await;
    assert_eq!(expected.0, StatusCode::NOT_FOUND);

    for path in common::not_found_token_paths("/api/b/", &unknown) {
        let path = format!("{path}/response");
        let response = app.put_json(&path, &body).await;
        assert_eq!(common::snapshot(response).await, expected, "{path}");
    }
    assert_eq!(app.voter_view(&real).await["status"], "unanswered");
}

#[tokio::test]
async fn malformed_option_ids_among_several_are_rejected_naming_them() {
    let app = TestApp::new().await;
    for submission_mode in ["final", "editable"] {
        let (token, options) = app
            .create_ballot_with_modes("multiple", submission_mode)
            .await;
        for id in malformed_option_ids(&options[0]) {
            let body = json!({ "option_ids": [options[0], options[1], id] });
            assert_response_rejected(&app, &token, &body, "option_ids[2]").await;
        }
        let repeated = vec![options[2].clone(); 1_000];
        let body = json!({ "option_ids": repeated });
        assert_response_rejected(&app, &token, &body, "option_ids[1]").await;
    }
}

#[tokio::test]
async fn hostile_changes_on_an_answered_editable_ballot_keep_the_response() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "editable").await;
    let path = format!("/api/b/{token}/response");
    app.put_json(&path, &json!({ "option_ids": [options[1]] }))
        .await;
    let before = app.voter_view(&token).await;
    app.clock.advance(chrono::Duration::minutes(1));

    for body in [
        json!({ "option_ids": [] }),
        json!({ "option_ids": [options[0], "AAAAAAAAAAB<script>"] }),
        json!({ "option_ids": [options[0]], "changed_at": "2000-01-01T00:00:00Z" }),
        json!({ "option_ids": [options[0]], "submission_mode": "final" }),
        json!({ "option_ids": null }),
    ] {
        let response = app.put_json(&path, &body).await;

        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
        let text = body_json(response).await.to_string();
        assert!(!text.contains("script"), "{text}");
        assert_eq!(app.voter_view(&token).await, before, "{body}");
    }
}
