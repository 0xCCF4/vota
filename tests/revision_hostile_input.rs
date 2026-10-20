//! The revision a submission sends back is untrusted input: through the
//! voter API a JSON non-negative integer, through the no-JS form exactly one
//! value of plain decimal digits as the page wrote it. Anything else, or
//! none, is a 422 that records nothing (and never echoes the value).

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, body_json, body_string};
use serde_json::json;

#[tokio::test]
async fn an_api_response_without_a_revision_is_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let body = json!({ "option_ids": [options[0]] });

    let response = app
        .put_raw(
            &format!("/api/b/{token}/response"),
            "application/json",
            body.to_string(),
        )
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let error = body_json(response).await;
    assert_eq!(error["error"], "invalid_input", "{error}");
    assert_eq!(error["field"], "revision", "{error}");
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn an_api_revision_that_is_not_a_non_negative_integer_is_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;

    for revision in [
        json!(null),
        json!("0"),
        json!(-1),
        json!(0.0),
        json!(0.5),
        json!(1e3),
        json!(true),
        json!([0]),
        json!({ "value": 0 }),
        json!(9_223_372_036_854_775_808_u64),
        json!(u64::MAX),
    ] {
        let body = json!({ "option_ids": [options[0]], "revision": revision });
        common::assert_response_rejected(&app, &token, &body, "revision").await;
    }
}

#[tokio::test]
async fn an_api_revision_is_never_echoed() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;

    let body = json!({ "option_ids": [options[0]], "revision": "<script>ZZMARK</script>" });
    let response = app
        .put_json(&format!("/api/b/{token}/response"), &body)
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!body_string(response).await.contains("ZZMARK"));
}

/// Posts `body` as it is (no revision added) and asserts the 422 voting
/// form with its notice, nothing recorded and nothing echoed.
async fn assert_form_rejected(app: &TestApp, token: &str, body: &str) {
    let response = app
        .post_raw(
            &format!("/b/{token}"),
            None,
            "application/x-www-form-urlencoded",
            body.to_owned(),
        )
        .await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "for {body:?}"
    );
    let html = body_string(response).await;
    assert!(
        html.contains(
            "The page was out of date or incomplete. Please check the Ballot and answer again."
        ),
        "for {body:?}: {html}"
    );
    assert!(
        !html.contains("ZZMARK"),
        "echoed input for {body:?}: {html}"
    );
    assert_eq!(
        app.voter_view(token).await["status"],
        "unanswered",
        "{body:?}"
    );
}

#[tokio::test]
async fn a_form_without_a_revision_is_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;

    for body in [
        format!("option={}", options[0]),
        format!("option={}&confirm=yes", options[0]),
    ] {
        assert_form_rejected(&app, &token, &body).await;
    }
}

#[tokio::test]
async fn a_form_revision_that_is_not_plain_decimal_digits_is_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;

    for revision in [
        "",
        "-1",
        "+0",
        "00",
        "01",
        "0.0",
        "1e0",
        "%200",
        "0%20",
        "0x0",
        "%EF%BC%90",
        "9223372036854775808",
        "99999999999999999999999",
        "ZZMARK",
        "0&revision=0",
    ] {
        let body = format!("option={}&confirm=yes&revision={revision}", options[0]);
        assert_form_rejected(&app, &token, &body).await;
    }
}
