//! Theme Tweaks: an optional accent colour (applied only as the CSS custom
//! property `--accent`, in a style element the page's Content Security
//! Policy allows by nonce) and an optional logo from the logo folder.

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use common::{TestApp, ballot_request, body_string};
use serde_json::json;

/// The nonce the page's Content-Security-Policy allows styles by.
fn style_nonce(response: &Response<Body>) -> String {
    let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_owned();
    let start = csp.find("'nonce-").expect("a style nonce") + "'nonce-".len();
    let end = start + csp[start..].find('\'').unwrap();
    csp[start..end].to_owned()
}

#[tokio::test]
async fn the_accent_colour_is_applied_as_a_css_custom_property() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["accent_color"] = json!("#1A2b3C");

    let created = app.create_ballot(&request).await;

    assert_eq!(created["accent_color"], "#1a2b3c", "stored lowercase");
    let token = created["token"].as_str().unwrap();
    let response = app.get(&format!("/b/{token}")).await;
    let nonce = style_nonce(&response);
    let html = body_string(response).await;
    let style = format!(
        "<style nonce=\"{nonce}\">:root {{ --accent: #1a2b3c; --accent-contrast: #ffffff; }}</style>"
    );
    assert!(html.contains(&style), "{style} not in {html}");
}

#[tokio::test]
async fn a_light_accent_colour_gets_black_text_on_it() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["accent_color"] = json!("#ffd166");

    let created = app.create_ballot(&request).await;

    let token = created["token"].as_str().unwrap();
    let html = body_string(app.get(&format!("/b/{token}")).await).await;
    assert!(
        html.contains("--accent: #ffd166; --accent-contrast: #000000;"),
        "{html}"
    );
}

#[tokio::test]
async fn without_an_accent_colour_the_theme_keeps_its_own() {
    let app = TestApp::new().await;

    let created = app.create_ballot(&ballot_request()).await;

    assert_eq!(created["accent_color"], json!(null));
    let token = created["token"].as_str().unwrap();
    let html = body_string(app.get(&format!("/b/{token}")).await).await;
    assert!(!html.contains("--accent"), "{html}");
    assert!(!html.contains("<style"), "{html}");
}

#[tokio::test]
async fn each_voting_page_response_gets_a_fresh_nonce() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&ballot_request()).await;
    let token = created["token"].as_str().unwrap();

    let first = style_nonce(&app.get(&format!("/b/{token}")).await);
    let second = style_nonce(&app.get(&format!("/b/{token}")).await);

    assert_ne!(first, second);
    assert!(first.len() >= 22, "{first}");
    assert_eq!(
        StatusCode::OK,
        app.get(&format!("/b/{token}")).await.status()
    );
}

/// The app with these files in its logo folder.
async fn app_with_logos(files: &[(&str, &[u8])]) -> TestApp {
    let files: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(path, contents)| ((*path).to_owned(), contents.to_vec()))
        .collect();
    TestApp::with_config(move |config| {
        let folder = config.logo_folder.clone().unwrap();
        for (path, contents) in files {
            common::write_file(&folder, &path, contents);
        }
    })
    .await
}

#[tokio::test]
async fn a_logo_from_the_logo_folder_is_shown_on_the_voting_page() {
    let app = app_with_logos(&[("club.svg", common::SVG_LOGO.as_bytes())]).await;
    let mut request = ballot_request();
    request["logo"] = json!("club");

    let created = app.create_ballot(&request).await;

    assert_eq!(created["logo"], "club");
    let token = created["token"].as_str().unwrap();
    let html = body_string(app.get(&format!("/b/{token}")).await).await;
    assert!(
        html.contains(r#"<img class="logo" src="/assets/logos/club" alt="">"#),
        "{html}"
    );
}

#[tokio::test]
async fn without_a_logo_the_page_shows_none() {
    let app = app_with_logos(&[("club.svg", common::SVG_LOGO.as_bytes())]).await;

    let created = app.create_ballot(&ballot_request()).await;

    assert_eq!(created["logo"], json!(null));
    let token = created["token"].as_str().unwrap();
    let html = body_string(app.get(&format!("/b/{token}")).await).await;
    assert!(!html.contains("<img"), "{html}");
}

#[tokio::test]
async fn the_voter_api_names_the_theme_accent_colour_and_logo_url() {
    let app = app_with_logos(&[("club.svg", common::SVG_LOGO.as_bytes())]).await;
    let mut request = ballot_request();
    request["theme"] = json!("dark");
    request["accent_color"] = json!("#123456");
    request["logo"] = json!("club");
    let created = app.create_ballot(&request).await;

    let view = app.voter_view(created["token"].as_str().unwrap()).await;

    assert_eq!(view["theme"], "dark");
    assert_eq!(view["accent_color"], "#123456");
    assert_eq!(
        view["logo_url"],
        "https://ballots.example.org/assets/logos/club"
    );
    assert!(view.get("logo").is_none(), "{view}");
}

#[tokio::test]
async fn the_voter_api_shows_the_defaults_without_theme_tweaks() {
    let app = TestApp::new().await;
    let created = app.create_ballot(&ballot_request()).await;

    let view = app.voter_view(created["token"].as_str().unwrap()).await;

    assert_eq!(view["theme"], "classic");
    assert_eq!(view["accent_color"], json!(null));
    assert_eq!(view["logo_url"], json!(null));
}
