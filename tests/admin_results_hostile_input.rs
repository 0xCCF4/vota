//! Hostile and invalid query parameters to `GET /admin/ballots`, through the
//! HTTP seam. Every rejection is a 422 naming the offending parameter.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::json;

/// Percent-encodes `value` for a query string.
fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// GETs `/admin/ballots?{query}` and asserts a 422 naming `field`.
async fn assert_rejected(app: &TestApp, token: &str, query: &str, field: &str) {
    let response = app
        .admin_get(&format!("/admin/ballots?{query}"), token)
        .await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {query:?}"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"], "invalid_input", "{body}");
    assert_eq!(body["field"], field, "for {query:?}: {body}");
}

#[tokio::test]
async fn unknown_query_parameters_are_rejected_without_echoing_their_names() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    for query in [
        "page=1",
        "limit=1",
        "offset=1",
        "Group=1",
        "STATUS=1",
        "group[]=1",
        "voter_reference=1",
        "group=A&page=2",
        "ZZMARK%3Cscript%3EZZ=1",
        "ZZMARK%FF=1",
        "ZZMARK=%FF",
    ] {
        let response = app
            .admin_get(&format!("/admin/ballots?{query}"), &token)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query}"
        );
        let text = common::body_string(response).await;
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["error"], "invalid_input", "{query}: {body}");
        assert_eq!(body["field"], json!(null), "{query}: {body}");
        for name in [
            "page",
            "limit",
            "offset",
            "Group",
            "STATUS",
            "voter_reference",
            "ZZMARK",
        ] {
            assert!(!text.contains(name), "{query}: {text}");
        }
    }
}

#[tokio::test]
async fn a_known_query_parameter_with_invalid_utf8_is_named() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    assert_rejected(&app, &token, "group=%FF", "group").await;
}

#[tokio::test]
async fn repeated_query_parameters_are_rejected() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    assert_rejected(&app, &token, "group=A&group=B", "group").await;
    assert_rejected(&app, &token, "group=A&group=A", "group").await;
    assert_rejected(&app, &token, "status=answered&status=unanswered", "status").await;
}

#[tokio::test]
async fn unknown_status_names_are_rejected() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let names = [
        "",
        "Answered",
        "ANSWERED",
        " answered",
        "answered ",
        "answered\u{0}",
        "\u{202E}answered",
        "auto-fallback",
        "autofallback",
        "Auto-fallback",
        "unanswered,answered",
        "*",
        "%",
        "open",
        "closed",
    ];
    for name in names {
        assert_rejected(&app, &token, &format!("status={}", encode(name)), "status").await;
    }
    assert_rejected(&app, &token, "status", "status").await;
    // Invalid UTF-8.
    assert_rejected(&app, &token, "status=answered%FF", "status").await;
}

#[tokio::test]
async fn hostile_group_filters_are_rejected() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let texts = [
        String::new(),
        "   \t ".into(),
        "x".repeat(201),
        "bell\u{7}".into(),
        "nul\u{0}byte".into(),
        "line\nbreak".into(),
        "escape\u{1b}[31m".into(),
        "c1\u{85}control".into(),
        "bidi\u{202E}override".into(),
        "isolate\u{2066}x\u{2069}".into(),
        "zero\u{200B}width".into(),
        "bom\u{FEFF}".into(),
        "tag\u{E0041}".into(),
    ];
    for text in texts {
        assert_rejected(&app, &token, &format!("group={}", encode(&text)), "group").await;
    }
    assert_rejected(&app, &token, "group", "group").await;
    assert_rejected(&app, &token, "group=%FF%FE", "group").await;
}

#[tokio::test]
async fn sql_and_pattern_characters_in_a_group_filter_match_only_literally() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["group"] = json!("A");
    app.create_ballot(&request).await;
    let token = app.admin_token().await;

    for group in ["%", "_", "' OR '1'='1", "A' --", "A%", "*", "../A"] {
        let response = app
            .admin_get(&format!("/admin/ballots?group={}", encode(group)), &token)
            .await;
        assert_eq!(response.status(), StatusCode::OK, "for {group:?}");
        assert_eq!(
            body_json(response).await,
            json!({ "ballots": [] }),
            "for {group:?}"
        );
    }
}

#[tokio::test]
async fn a_group_filter_is_trimmed_and_nfc_normalised_like_a_group() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["group"] = json!("Caf\u{e9}");
    let created = app.create_ballot(&request).await;
    let token = app.admin_token().await;

    let response = app
        .admin_get(
            &format!("/admin/ballots?group={}", encode(" Cafe\u{301}\t")),
            &token,
        )
        .await;

    assert_eq!(body_json(response).await, json!({ "ballots": [created] }));
}

#[tokio::test]
async fn listing_needs_a_valid_admin_token() {
    let app = TestApp::new().await;
    app.create_ballot(&ballot_request()).await;

    assert_eq!(
        app.get("/admin/ballots").await.status(),
        StatusCode::UNAUTHORIZED
    );
    let response = app
        .admin_get("/admin/ballots?status=answered", "vta_wrong")
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
