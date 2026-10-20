#![cfg(feature = "server")]

mod common;

use axum::http::{StatusCode, header};
use common::{TestApp, ballot_request, body_string};
use serde_json::json;

#[tokio::test]
async fn voting_page_shows_title_and_options_as_radio_buttons() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&ballot_request()).await;
    let token = created["token"].as_str().unwrap();

    let response = app.get(&format!("/b/{token}")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let html = body_string(response).await;
    assert!(html.contains("<h1>Lunch on Friday?</h1>"), "{html}");
    for option in created["options"].as_array().unwrap() {
        let radio = format!(
            r#"<input type="radio" name="option" value="{}" id="option-{}" required>"#,
            option["id"].as_str().unwrap(),
            option["id"].as_str().unwrap()
        );
        assert!(html.contains(&radio), "{radio} not in {html}");
    }
    assert!(html.contains(">Pizza</label>"), "{html}");
    assert!(html.contains(">Sushi</label>"), "{html}");
}

#[tokio::test]
async fn voting_page_shows_the_deadline_in_the_configured_time_zone() {
    let app = TestApp::with_config(|config| {
        config.default_time_zone = chrono_tz::Europe::Berlin;
    })
    .await;
    let mut request = ballot_request();
    request["voting_deadline"] = json!("2030-01-22T12:00:00Z");
    let token = app.create_ballot(&request).await["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(
        html.contains(
            r#"<time datetime="2030-01-22T12:00:00Z">Tuesday, 22 January 2030, 13:00 CET</time>"#
        ),
        "{html}"
    );
}

#[tokio::test]
async fn voting_page_escapes_html_in_title_and_options() {
    let app = TestApp::new().await;
    let request = json!({
        "title": "<script>alert(1)</script>",
        "options": [{ "label": "<b>bold</b>" }, { "label": "\"quoted\" & 'single'" }],
        "voting_deadline": "2030-01-22T12:00:00Z",
    });
    let token = app.create_ballot(&request).await["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(!html.contains("<script>"), "{html}");
    assert!(!html.contains("<b>bold"), "{html}");
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;&#x2f;script&gt;"),
        "{html}"
    );
}

#[tokio::test]
async fn unknown_and_malformed_tokens_get_byte_identical_not_found_pages() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&ballot_request()).await;
    let real = created["token"].as_str().unwrap();
    let unknown = if real.starts_with('A') { "B" } else { "A" }.to_owned() + &real[1..];

    let mut responses = Vec::new();
    for path in common::not_found_token_paths("/b/", &unknown) {
        responses.push(common::snapshot(app.get(&path).await).await);
    }

    assert_eq!(responses[0].0, StatusCode::NOT_FOUND);
    assert!(String::from_utf8_lossy(&responses[0].2).contains("Not found"));
    for (path, response) in common::not_found_token_paths("/b/", &unknown)
        .iter()
        .zip(&responses)
        .skip(1)
    {
        assert_eq!(response, &responses[0], "{path}");
    }
}
