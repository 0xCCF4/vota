//! The admin edits a Ballot's Theme and Theme Tweaks (`PATCH`): like any
//! content edit, only while the Ballot has no Response. `accent_color` and
//! `logo` may be cleared with `null`; `theme` may not (name `classic` to go
//! back to the default).

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{SVG_LOGO, TestApp, ballot_request, body_json, body_string, write_file};
use serde_json::{Value, json};

async fn app() -> TestApp {
    TestApp::with_config(|config| {
        let folder = config.logo_folder.clone().unwrap();
        write_file(&folder, "club.svg", SVG_LOGO);
        write_file(&folder, "other.svg", SVG_LOGO);
    })
    .await
}

fn path(created: &Value) -> String {
    format!("/admin/ballots/{}", created["id"])
}

async fn page(app: &TestApp, created: &Value) -> String {
    let token = created["token"].as_str().unwrap();
    body_string(app.get(&format!("/b/{token}")).await).await
}

fn tweaked() -> Value {
    let mut request = ballot_request();
    request["theme"] = json!("dark");
    request["accent_color"] = json!("#123456");
    request["logo"] = json!("club");
    request
}

#[tokio::test]
async fn the_admin_changes_theme_accent_colour_and_logo_and_the_voter_sees_them() {
    let app = app().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "theme": "minimal", "accent_color": "#ABCDEF", "logo": "other" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["theme"], "minimal");
    assert_eq!(edited["accent_color"], "#abcdef");
    assert_eq!(edited["logo"], "other");
    let html = page(&app, &created).await;
    assert!(html.contains("/assets/themes/minimal/style.css"), "{html}");
    assert!(html.contains("--accent: #abcdef;"), "{html}");
    assert!(html.contains(r#"src="/assets/logos/other""#), "{html}");
}

#[tokio::test]
async fn null_clears_the_accent_colour_and_the_logo() {
    let app = app().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&tweaked()).await;

    let response = app
        .admin_patch(
            &path(&created),
            &admin,
            &json!({ "accent_color": null, "logo": null }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["accent_color"], json!(null));
    assert_eq!(edited["logo"], json!(null));
    assert_eq!(edited["theme"], "dark", "the Theme stays");
    let html = page(&app, &created).await;
    assert!(!html.contains("--accent"), "{html}");
    assert!(!html.contains("<img"), "{html}");
}

#[tokio::test]
async fn null_is_not_a_theme() {
    let app = app().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&tweaked()).await;

    let response = app
        .admin_patch(&path(&created), &admin, &json!({ "theme": null }))
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(response).await["field"], "theme");
}

#[tokio::test]
async fn an_edit_without_these_fields_keeps_them() {
    let app = app().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&tweaked()).await;

    let response = app
        .admin_patch(&path(&created), &admin, &json!({ "title": "New title" }))
        .await;

    let edited = body_json(response).await;
    assert_eq!(edited["theme"], "dark");
    assert_eq!(edited["accent_color"], "#123456");
    assert_eq!(edited["logo"], "club");
}

#[tokio::test]
async fn the_theme_and_theme_tweaks_of_an_answered_ballot_cannot_change() {
    let app = app().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&tweaked()).await;
    let token = created["token"].as_str().unwrap();
    let pizza = &created["options"][0]["id"];
    let answered = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [pizza] }),
        )
        .await;
    assert_eq!(answered.status(), StatusCode::OK);

    for edit in [
        json!({ "theme": "classic" }),
        json!({ "accent_color": "#000000" }),
        json!({ "accent_color": null }),
        json!({ "logo": "other" }),
        json!({ "logo": null }),
        json!({ "group": "Survey", "theme": "classic" }),
    ] {
        let response = app.admin_patch(&path(&created), &admin, &edit).await;

        assert_eq!(response.status(), StatusCode::CONFLICT, "{edit}");
    }
    let fetched = body_json(app.admin_get(&path(&created), &admin).await).await;
    assert_eq!(fetched["theme"], "dark");
    assert_eq!(fetched["accent_color"], "#123456");
    assert_eq!(fetched["logo"], "club");
}
