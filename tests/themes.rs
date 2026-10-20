//! Themes: built-in Themes ship with the binary, folder Themes load from the
//! theme folder and override built-ins of the same name. The admin picks one
//! per Ballot by name; omitting it means the default, `classic`.

#![cfg(feature = "server")]

mod common;

use axum::http::{StatusCode, header};
use common::{TestApp, ballot_request, body_string};
use serde_json::{Value, json};

/// The voting page of the Ballot created by `request`.
async fn page_of(app: &TestApp, request: &Value) -> String {
    let created = app.create_ballot(request).await;
    let token = created["token"].as_str().unwrap();
    body_string(app.get(&format!("/b/{token}")).await).await
}

fn stylesheet_link(theme: &str) -> String {
    format!(r#"<link rel="stylesheet" href="/assets/themes/{theme}/style.css">"#)
}

#[tokio::test]
async fn a_ballot_without_a_theme_uses_the_default_classic_theme() {
    let app = TestApp::new().await;

    let created = app.create_ballot(&ballot_request()).await;

    assert_eq!(created["theme"], "classic");
    let html = page_of(&app, &ballot_request()).await;
    assert!(html.contains(&stylesheet_link("classic")), "{html}");
}

#[tokio::test]
async fn the_built_in_themes_stylesheets_are_served_from_our_own_host() {
    let app = TestApp::new().await;

    for theme in ["classic", "dark", "minimal"] {
        let response = app.get(&format!("/assets/themes/{theme}/style.css")).await;

        assert_eq!(response.status(), StatusCode::OK, "{theme}");
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "text/css; charset=utf-8");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert!(
            headers[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("max-age="),
            "{theme}"
        );
        let css = body_string(response).await;
        assert!(css.contains("--accent"), "{theme}: {css}");
    }
}

#[tokio::test]
async fn the_admin_picks_a_built_in_theme_by_name() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["theme"] = json!("dark");

    let created = app.create_ballot(&request).await;

    assert_eq!(created["theme"], "dark");
    let token = created["token"].as_str().unwrap();
    let html = body_string(app.get(&format!("/b/{token}")).await).await;
    assert!(html.contains(&stylesheet_link("dark")), "{html}");
    assert!(!html.contains(&stylesheet_link("classic")), "{html}");
}
