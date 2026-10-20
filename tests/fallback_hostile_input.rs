//! Hostile and invalid Fallbacks in `POST /admin/ballots`, through the HTTP
//! seam. Every rejection is a 422 naming `fallback` or the offending
//! `fallback[i]`.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// A request with the Options Mon, Tue, Wed and this Choice Mode and
/// Fallback.
fn request(choice_mode: &str, fallback: Value) -> Value {
    let mut request = ballot_request();
    request["options"] = json!([{ "label": "Mon" }, { "label": "Tue" }, { "label": "Wed" }]);
    request["choice_mode"] = json!(choice_mode);
    request["fallback"] = fallback;
    request
}

/// Sends `request` and asserts a 422 naming `field`.
async fn assert_rejected(app: &TestApp, request: &Value, field: &str) {
    let token = app.admin_token().await;
    let response = app.post_json("/admin/ballots", Some(&token), request).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {request}"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"], "invalid_input", "{body}");
    assert_eq!(body["field"], field, "for {request}: {body}");
}

/// The labels of the created Ballot's Fallback.
fn fallback_labels(created: &Value) -> Vec<&str> {
    created["fallback"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["label"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn an_empty_fallback_is_rejected() {
    let app = TestApp::new().await;
    for mode in ["single", "multiple"] {
        assert_rejected(&app, &request(mode, json!([])), "fallback").await;
    }
}

#[tokio::test]
async fn more_than_one_option_on_a_single_choice_ballot_is_rejected() {
    let app = TestApp::new().await;
    assert_rejected(&app, &request("single", json!(["Mon", "Tue"])), "fallback").await;
}

#[tokio::test]
async fn several_options_on_a_multiple_choice_ballot_are_accepted() {
    let app = TestApp::new().await;

    let created = app
        .create_ballot(&request("multiple", json!(["Wed", "Mon", "Tue"])))
        .await;

    assert_eq!(fallback_labels(&created), ["Mon", "Tue", "Wed"]);
}

#[tokio::test]
async fn the_default_choice_mode_is_single_for_the_fallback_too() {
    let app = TestApp::new().await;
    let mut request = request("single", json!(["Mon", "Wed"]));
    request.as_object_mut().unwrap().remove("choice_mode");
    assert_rejected(&app, &request, "fallback").await;
}

#[tokio::test]
async fn a_label_that_is_not_an_option_of_the_ballot_is_rejected_naming_it() {
    let app = TestApp::new().await;
    for (fallback, field) in [
        (json!(["Fri"]), "fallback[0]"),
        (json!(["Mon", "Fri"]), "fallback[1]"),
        // Exact match only: no case folding, no prefix matching.
        (json!(["mon"]), "fallback[0]"),
        (json!(["MON"]), "fallback[0]"),
        (json!(["Mo"]), "fallback[0]"),
        (json!(["Mon Tue"]), "fallback[0]"),
        (json!(["*"]), "fallback[0]"),
        (json!(["%"]), "fallback[0]"),
        (json!(["Mon' OR '1'='1"]), "fallback[0]"),
        (json!(["../Mon"]), "fallback[0]"),
    ] {
        assert_rejected(&app, &request("multiple", fallback), field).await;
    }
}

#[tokio::test]
async fn an_option_id_instead_of_a_label_is_rejected() {
    let app = TestApp::new().await;
    let other = app.create_ballot(&request("single", json!(["Mon"]))).await;
    let id = other["options"][0]["id"].as_str().unwrap();
    assert_rejected(&app, &request("single", json!([id])), "fallback[0]").await;
}

#[tokio::test]
async fn a_label_given_twice_is_rejected_naming_the_duplicate() {
    let app = TestApp::new().await;
    for fallback in [
        json!(["Tue", "Tue"]),
        // Duplicates after trimming and NFC normalisation, too.
        json!(["Tue", " Tue\t"]),
    ] {
        assert_rejected(&app, &request("multiple", fallback), "fallback[1]").await;
    }
    let caf = |label: &str| {
        let mut request = request("multiple", json!(["Cafe\u{301}", "Caf\u{e9}"]));
        request["options"] = json!([{ "label": label }, { "label": "Tea" }]);
        request
    };
    assert_rejected(&app, &caf("Caf\u{e9}"), "fallback[1]").await;
}

#[tokio::test]
async fn labels_are_matched_after_trimming_and_nfc_normalisation() {
    let app = TestApp::new().await;
    let mut request = request("multiple", json!(["  Tue \t", "Cafe\u{301}"]));
    request["options"] = json!([{ "label": "Tue" }, { "label": "Caf\u{e9}" }]);

    let created = app.create_ballot(&request).await;

    assert_eq!(fallback_labels(&created), ["Tue", "Caf\u{e9}"]);
}

#[tokio::test]
async fn hostile_labels_are_rejected_naming_them() {
    let app = TestApp::new().await;
    let hostile = [
        String::new(),
        "   \t ".into(),
        "x".repeat(201),
        "Mon\u{0}".into(),
        "M\non".into(),
        "Mon\u{1b}[31m".into(),
        "Mon\u{7f}".into(),
        "Mo\u{85}n".into(),
        "\u{202E}noM".into(),
        "\u{2066}Mon\u{2069}".into(),
        "Mon\u{200F}".into(),
        "Mo\u{200B}n".into(),
        "\u{FEFF}Mon".into(),
        "Mon\u{E0041}".into(),
    ];
    for label in hostile {
        let fallback = json!(["Tue", label]);
        assert_rejected(&app, &request("multiple", fallback), "fallback[1]").await;
    }
}

#[tokio::test]
async fn a_fallback_of_the_wrong_type_is_rejected() {
    let app = TestApp::new().await;
    for fallback in [
        json!(null),
        json!("Mon"),
        json!(42),
        json!(true),
        json!({ "label": "Mon" }),
        json!({ "0": "Mon" }),
    ] {
        assert_rejected(&app, &request("single", fallback), "fallback").await;
    }
    for fallback in [
        json!([42]),
        json!([null]),
        json!([["Mon"]]),
        json!([{ "label": "Mon" }]),
    ] {
        assert_rejected(&app, &request("single", fallback), "fallback[0]").await;
    }
}

#[tokio::test]
async fn a_huge_fallback_is_rejected() {
    let app = TestApp::new().await;
    let labels: Vec<_> = std::iter::repeat_n("Mon", 5_000).collect();
    assert_rejected(&app, &request("multiple", json!(labels)), "fallback[1]").await;
}

#[tokio::test]
async fn a_fallback_is_checked_against_the_ballots_own_options_only() {
    let app = TestApp::new().await;
    let mut other = request("single", json!(["Fri"]));
    other["options"] = json!([{ "label": "Fri" }, { "label": "Sat" }]);
    app.create_ballot(&other).await;

    assert_rejected(&app, &request("single", json!(["Fri"])), "fallback[0]").await;
}

#[tokio::test]
async fn invalid_options_are_reported_before_the_fallback() {
    let app = TestApp::new().await;
    let mut request = request("single", json!(["Nope"]));
    request["options"] = json!([{ "label": "Mon" }]);
    assert_rejected(&app, &request, "options").await;
}
