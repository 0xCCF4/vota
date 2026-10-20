//! Free text refuses characters that render as nothing or as something
//! else (security audit I-6): format characters (Unicode general category
//! Cf), private use (Co), unassigned code points and noncharacters (Cn),
//! the Braille blank, and more than four combining marks on one base
//! character. Visually identical texts can't pass as different.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// Sends a create request and returns the 422's body, failing on any other status.
async fn rejection(app: &TestApp, request: &Value) -> Value {
    let token = app.admin_token().await;
    let response = app.post_json("/admin/ballots", Some(&token), request).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {request}"
    );
    body_json(response).await
}

fn with(field: &str, value: Value) -> Value {
    let mut request = ballot_request();
    request[field] = value;
    request
}

#[tokio::test]
async fn an_option_label_differing_only_by_a_variation_selector_is_refused() {
    let app = TestApp::new().await;
    let request = with(
        "options",
        json!([{ "label": "A" }, { "label": "A\u{E0100}" }]),
    );

    let error = rejection(&app, &request).await;

    assert_eq!(error["field"], "options[1].label", "{error}");
}

/// Each with visible text around it, so only the character itself is at fault.
fn invisible_texts() -> Vec<String> {
    [
        '\u{0600}',   // Arabic number sign (Cf)
        '\u{0605}',   // Arabic number mark above (Cf)
        '\u{06DD}',   // Arabic end of ayah (Cf)
        '\u{070F}',   // Syriac abbreviation mark (Cf)
        '\u{0890}',   // Arabic pound mark above (Cf)
        '\u{08E2}',   // Arabic disputed end of ayah (Cf)
        '\u{110BD}',  // Kaithi number sign (Cf)
        '\u{13430}',  // Egyptian hieroglyph vertical joiner (Cf)
        '\u{1BCA0}',  // shorthand format letter overlap (Cf)
        '\u{1D173}',  // musical symbol begin beam (Cf)
        '\u{E000}',   // private use (Co)
        '\u{F0000}',  // supplementary private use (Co)
        '\u{0378}',   // unassigned (Cn)
        '\u{FDD0}',   // noncharacter
        '\u{FFFE}',   // noncharacter
        '\u{10FFFF}', // noncharacter
        '\u{2800}',   // Braille pattern blank
        '\u{FFFC}',   // object replacement character
    ]
    .iter()
    .map(|c| format!("left{c}right"))
    .collect()
}

#[tokio::test]
async fn invisible_and_unassigned_characters_are_refused_in_every_single_line_text() {
    let app = TestApp::new().await;
    for text in invisible_texts() {
        for (field, request) in [
            ("title", with("title", json!(text))),
            ("group", with("group", json!(text))),
            ("voter_reference", with("voter_reference", json!(text))),
            (
                "options[0].label",
                with("options", json!([{ "label": text }, { "label": "B" }])),
            ),
        ] {
            let error = rejection(&app, &request).await;
            assert_eq!(error["field"], field, "{text:?}: {error}");
        }
    }
}

#[tokio::test]
async fn a_title_of_only_the_braille_blank_is_refused() {
    let app = TestApp::new().await;

    let error = rejection(&app, &with("title", json!("\u{2800}"))).await;

    assert_eq!(error["field"], "title", "{error}");
}

#[tokio::test]
async fn the_description_refuses_them_too_but_keeps_newlines_and_tabs() {
    let app = TestApp::new().await;
    for text in invisible_texts() {
        let error = rejection(&app, &with("description", json!(text))).await;
        assert_eq!(error["field"], "description", "{text:?}: {error}");
    }

    let created = app
        .create_ballot(&with("description", json!("- one\n\t- two")))
        .await;

    assert_eq!(created["description"], "- one\n\t- two");
}

#[tokio::test]
async fn up_to_four_combining_marks_per_base_character_are_kept_a_fifth_is_refused() {
    let app = TestApp::new().await;
    // `x` has no precomposed forms, so NFC leaves every mark in place.
    let four = "x\u{300}\u{301}\u{302}\u{303}";
    let five = "x\u{300}\u{301}\u{302}\u{303}\u{304}";

    let created = app.create_ballot(&with("title", json!(four))).await;
    let error = rejection(&app, &with("title", json!(five))).await;

    assert_eq!(created["title"], four);
    assert_eq!(error["field"], "title", "{error}");
    assert_eq!(error["message"], "too many combining marks", "{error}");
}

#[tokio::test]
async fn ordinary_text_in_many_scripts_is_kept() {
    let app = TestApp::new().await;
    for text in [
        "Tiếng Việt",
        "हिन्दी में मतदान",
        "ภาษาไทย",
        "مُحَمَّد",
        "בְּרֵאשִׁית",
        "日本語の投票",
        "한국어 투표",
        "Ελληνικά",
        "👍🏽 🗳 🇩🇪",
    ] {
        let created = app.create_ballot(&with("title", json!(text))).await;
        assert_eq!(created["title"], text);
    }
}
