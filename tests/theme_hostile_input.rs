//! Hostile and invalid Theme, accent colour and logo values on create and
//! edit, and on the asset routes. Names are only ever looked up among the
//! registry's known names; nothing reaches the filesystem.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// Sends `request` to create a Ballot and asserts a 422 naming `field`.
async fn assert_create_rejected(app: &TestApp, request: &Value, field: &str) {
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

fn with(field: &str, value: Value) -> Value {
    let mut request = ballot_request();
    request[field] = value;
    request
}

/// Names no Theme or logo can have, or that no Theme or logo has: unknown,
/// path traversal (plain, encoded, Windows-style), absolute paths, uppercase
/// or padded variants of a real name, overlong, empty, control characters.
fn hostile_names(real: &str) -> Vec<Value> {
    let mut names: Vec<Value> = [
        "no-such-name".to_owned(),
        "../classic".into(),
        "..".into(),
        ".".into(),
        "classic/../classic".into(),
        "..%2Fclassic".into(),
        "%2e%2e".into(),
        "..\\classic".into(),
        "/etc/passwd".into(),
        "classic/style.css".into(),
        real.to_uppercase(),
        format!(" {real}"),
        format!("{real} "),
        format!("{real}\n"),
        format!("{real}\u{0}"),
        format!("-{real}"),
        format!("{real}.css"),
        "a".repeat(65),
        String::new(),
        "ünïcode".into(),
    ]
    .into_iter()
    .map(Value::from)
    .collect();
    names.extend([
        json!(null),
        json!(42),
        json!(["classic"]),
        json!({ "name": real }),
    ]);
    names
}

#[tokio::test]
async fn unknown_and_malformed_theme_names_are_rejected_on_create() {
    let app = TestApp::new().await;

    for name in hostile_names("classic") {
        assert_create_rejected(&app, &with("theme", name), "theme").await;
    }
}

/// Values that are not `#rrggbb`: CSS injection, shorthand, alpha, names,
/// functions, missing `#`, non-hex digits, padding, wrong types.
fn hostile_colours() -> Vec<Value> {
    [
        "red;}body{display:none}",
        "#fff;}body{background:url(https://evil.example/x)}",
        "#123456;}",
        "#123456 ",
        " #123456",
        "#123456\n",
        "#fff",
        "#12345g",
        "#1234567",
        "#12345",
        "#12345678",
        "123456",
        "##123456",
        "red",
        "rgb(1,2,3)",
        "var(--x)",
        "#１２３４５６",
        "</style><script>alert(1)</script>",
        "",
    ]
    .into_iter()
    .map(Value::from)
    .chain([json!(null), json!(123456), json!(["#123456"])])
    .collect()
}

#[tokio::test]
async fn malformed_accent_colours_are_rejected_on_create() {
    let app = TestApp::new().await;

    for colour in hostile_colours() {
        assert_create_rejected(&app, &with("accent_color", colour), "accent_color").await;
    }
}

/// The app with a logo `club` in its logo folder.
async fn app_with_logo() -> TestApp {
    TestApp::with_config(|config| {
        let folder = config.logo_folder.clone().unwrap();
        common::write_file(&folder, "club.svg", common::SVG_LOGO);
    })
    .await
}

#[tokio::test]
async fn unknown_and_malformed_logo_names_are_rejected_on_create() {
    let app = app_with_logo().await;

    let mut names = hostile_names("club");
    names.extend([json!("club.svg"), json!("../logos/club.svg")]);
    for name in names {
        assert_create_rejected(&app, &with("logo", name), "logo").await;
    }
}

#[tokio::test]
async fn hostile_theme_accent_colour_and_logo_values_are_rejected_on_edit() {
    let app = app_with_logo().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["theme"] = json!("dark");
    request["accent_color"] = json!("#123456");
    request["logo"] = json!("club");
    let created = app.create_ballot(&request).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    let mut cases: Vec<(&str, Value)> = Vec::new();
    // `null` clears the accent colour and the logo, but is no Theme.
    cases.extend(hostile_names("classic").into_iter().map(|n| ("theme", n)));
    cases.extend(
        hostile_names("club")
            .into_iter()
            .filter(|n| !n.is_null())
            .map(|n| ("logo", n)),
    );
    cases.extend(
        hostile_colours()
            .into_iter()
            .filter(|c| !c.is_null())
            .map(|c| ("accent_color", c)),
    );
    for (field, value) in cases {
        let response = app
            .admin_patch(&path, &admin, &json!({ field: value }))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{field}: {value}"
        );
        assert_eq!(body_json(response).await["field"], field, "{value}");
    }
    let fetched = body_json(app.admin_get(&path, &admin).await).await;
    assert_eq!(fetched["theme"], "dark");
    assert_eq!(fetched["accent_color"], "#123456");
    assert_eq!(fetched["logo"], "club");
}
