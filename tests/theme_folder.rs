//! Folder Themes: `<theme_folder>/<name>/style.css` (required) and optional
//! template overrides in `<theme_folder>/<name>/templates/`. Loaded at
//! startup; a folder Theme with a built-in Theme's name replaces it.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_string, write_file};
use serde_json::json;

/// The app with these files in its theme folder.
async fn app_with_themes(files: &[(&str, &str)]) -> TestApp {
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(path, contents)| ((*path).to_owned(), (*contents).to_owned()))
        .collect();
    TestApp::with_config(move |config| {
        let folder = config.theme_folder.clone().unwrap();
        for (path, contents) in files {
            write_file(&folder, &path, contents);
        }
    })
    .await
}

/// Creates a Ballot with this Theme; returns its voting page.
async fn page_with_theme(app: &TestApp, theme: &str) -> String {
    let mut request = ballot_request();
    request["theme"] = json!(theme);
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap();
    body_string(app.get(&format!("/b/{token}")).await).await
}

#[tokio::test]
async fn a_folder_theme_is_available_by_name_with_its_stylesheet() {
    let app = app_with_themes(&[("autumn/style.css", "body { color: brown; }")]).await;

    let html = page_with_theme(&app, "autumn").await;

    assert!(
        html.contains(r#"<link rel="stylesheet" href="/assets/themes/autumn/style.css">"#),
        "{html}"
    );
    let response = app.get("/assets/themes/autumn/style.css").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_string(response).await, "body { color: brown; }");
}

#[tokio::test]
async fn a_folder_theme_overrides_the_built_in_theme_of_the_same_name() {
    let app = app_with_themes(&[("classic/style.css", "/* our classic */")]).await;

    let response = app.get("/assets/themes/classic/style.css").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_string(response).await, "/* our classic */");
    // Other built-in Themes stay.
    let dark = app.get("/assets/themes/dark/style.css").await;
    assert_eq!(dark.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_folder_theme_may_override_templates_and_falls_back_to_the_built_in_ones() {
    let base = r#"<!doctype html><html><head><link rel="stylesheet" href="{{ theme_stylesheet }}"></head>
<body class="festive"><header>Our Club</header>{% block content %}{% endblock %}</body></html>"#;
    let app = app_with_themes(&[
        ("festive/style.css", ""),
        ("festive/templates/base.html", base),
    ])
    .await;

    let html = page_with_theme(&app, "festive").await;

    // Its own base.html, with the built-in ballot.html in it.
    assert!(
        html.contains(r#"<body class="festive"><header>Our Club</header>"#),
        "{html}"
    );
    assert!(html.contains("<h1>Lunch on Friday?</h1>"), "{html}");
    assert!(html.contains(r#"type="radio""#), "{html}");
}

#[tokio::test]
async fn a_folder_theme_may_override_the_voting_page_template_alone() {
    let ballot = r#"{% extends "base.html" %}{% block content %}<h1 class="big">{{ title }}</h1>
<form method="post">{% for option in options %}<p>{{ option.label }}</p>{% endfor %}</form>{% endblock %}"#;
    let app = app_with_themes(&[
        ("plain/style.css", ""),
        ("plain/templates/ballot.html", ballot),
    ])
    .await;

    let html = page_with_theme(&app, "plain").await;

    assert!(
        html.contains(r#"<h1 class="big">Lunch on Friday?</h1>"#),
        "{html}"
    );
    assert!(html.contains("<p>Pizza</p>"), "{html}");
    // The built-in base.html around it.
    assert!(
        html.contains(r#"<link rel="stylesheet" href="/assets/themes/plain/style.css">"#),
        "{html}"
    );
}

#[tokio::test]
async fn directories_whose_names_are_not_theme_names_are_ignored() {
    let app = app_with_themes(&[
        ("Autumn/style.css", ""),
        ("my theme/style.css", ""),
        ("-dash/style.css", ""),
        (".hidden/style.css", ""),
        ("autumn/style.css", "/* the real one */"),
        ("loose-file.css", ""),
    ])
    .await;
    let admin = app.admin_token().await;

    for name in ["Autumn", "my theme", "-dash", ".hidden", "loose-file.css"] {
        let mut request = ballot_request();
        request["theme"] = json!(name);
        let response = app
            .post_json("/admin/ballots", Some(&admin), &request)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{name}"
        );
    }
    let html = page_with_theme(&app, "autumn").await;
    assert!(html.contains("/assets/themes/autumn/style.css"), "{html}");
}

/// The startup error for a theme folder with these files.
async fn startup_error(files: &[(&str, &str)]) -> String {
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(path, contents)| ((*path).to_owned(), (*contents).to_owned()))
        .collect();
    let started = TestApp::try_with_config(move |config| {
        let folder = config.theme_folder.clone().unwrap();
        for (path, contents) in files {
            write_file(&folder, &path, contents);
        }
    })
    .await;
    match started {
        Ok(_) => panic!("startup should fail"),
        Err(error) => format!("{error:#}"),
    }
}

#[tokio::test]
async fn a_theme_template_that_does_not_parse_fails_startup_naming_the_file() {
    let error = startup_error(&[
        ("broken/style.css", ""),
        ("broken/templates/ballot.html", "{% if %}"),
    ])
    .await;

    assert!(error.contains("broken/templates/ballot.html"), "{error}");
}

#[tokio::test]
async fn a_folder_theme_without_a_stylesheet_fails_startup_naming_the_file() {
    let error = startup_error(&[("nostyle/templates/base.html", "")]).await;

    assert!(error.contains("nostyle/style.css"), "{error}");
}

#[tokio::test]
async fn a_configured_theme_folder_that_does_not_exist_fails_startup() {
    let started = TestApp::try_with_config(|config| {
        config.theme_folder = Some(config.theme_folder.clone().unwrap().join("missing"));
    })
    .await;

    let error = format!("{:#}", started.err().expect("startup should fail"));
    assert!(error.contains("missing"), "{error}");
}

#[tokio::test]
async fn a_ballot_whose_theme_and_logo_were_removed_falls_back_to_the_default_theme_and_no_logo() {
    let app = TestApp::with_config(|config| {
        let themes = config.theme_folder.clone().unwrap();
        write_file(&themes, "autumn/style.css", "");
        write_file(
            &themes,
            "autumn/templates/base.html",
            "{% block content %}{% endblock %}",
        );
        let logos = config.logo_folder.clone().unwrap();
        write_file(&logos, "club.svg", common::SVG_LOGO);
    })
    .await;
    let mut request = ballot_request();
    request["theme"] = json!("autumn");
    request["logo"] = json!("club");
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap().to_owned();

    let app = app
        .restart(|config| {
            let themes = config.theme_folder.clone().unwrap();
            std::fs::remove_dir_all(themes.join("autumn")).unwrap();
            let logos = config.logo_folder.clone().unwrap();
            std::fs::remove_file(logos.join("club.svg")).unwrap();
        })
        .await;

    let response = app.get(&format!("/b/{token}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(
        html.contains(r#"<link rel="stylesheet" href="/assets/themes/classic/style.css">"#),
        "{html}"
    );
    assert!(!html.contains("autumn"), "{html}");
    assert!(!html.contains("<img"), "{html}");
    let view = app.voter_view(&token).await;
    assert_eq!(view["theme"], "classic");
    assert_eq!(view["logo_url"], json!(null));
    // The admin still sees what was chosen, and may edit other fields.
    let admin = app.admin_token().await;
    let path = format!("/admin/ballots/{}", created["id"]);
    let edited = app
        .admin_patch(&path, &admin, &json!({ "title": "Still here?" }))
        .await;
    assert_eq!(edited.status(), StatusCode::OK);
    let edited = common::body_json(edited).await;
    assert_eq!(edited["theme"], "autumn");
    assert_eq!(edited["logo"], "club");
}

#[tokio::test]
async fn a_theme_template_extending_a_missing_template_fails_startup_naming_theme_and_file() {
    let error = startup_error(&[
        ("broken/style.css", ""),
        (
            "broken/templates/ballot.html",
            r#"{% extends "nowhere.html" %}"#,
        ),
    ])
    .await;

    assert!(error.contains(r#"Theme "broken""#), "{error}");
    assert!(error.contains("broken/templates/ballot.html"), "{error}");
    // minijinja's own message.
    assert!(error.contains("nowhere.html"), "{error}");
}

#[tokio::test]
async fn a_theme_template_using_an_unknown_filter_fails_startup_naming_theme_and_file() {
    let error = startup_error(&[
        ("broken/style.css", ""),
        (
            "broken/templates/base.html",
            "<html><body>{{ title|shout }}{% block content %}{% endblock %}</body></html>",
        ),
    ])
    .await;

    assert!(error.contains(r#"Theme "broken""#), "{error}");
    assert!(error.contains("broken/templates/base.html"), "{error}");
    assert!(error.contains("shout"), "{error}");
}

#[tokio::test]
async fn the_built_in_themes_pass_the_trial_render() {
    // An empty theme folder: only the built-in Themes, each trial-rendered.
    let started = TestApp::try_with_config(|_| {}).await;

    if let Err(error) = started {
        panic!("{error:#}");
    }
}

#[tokio::test]
async fn a_render_error_at_request_time_gives_a_neutral_error_page_and_a_log_without_secrets() {
    // Fails only for a title the trial render's samples don't have.
    let ballot = r#"{% extends "base.html" %}{% block content %}
{% if title == "Explode" %}{{ no_such_function() }}{% endif %}
<form method="post"></form>{% endblock %}"#;
    let app = app_with_themes(&[
        ("fragile/style.css", ""),
        ("fragile/templates/ballot.html", ballot),
    ])
    .await;
    let mut request = ballot_request();
    request["title"] = json!("Explode");
    request["theme"] = json!("fragile");
    request["voter_reference"] = json!("alice@example.org");
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap();
    let logs = common::CapturedLogs::default();
    let guard = logs.start();

    let response = app.get(&format!("/b/{token}")).await;

    drop(guard);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let html = body_string(response).await;
    assert!(html.contains("Something went wrong"), "{html}");
    assert!(!html.contains("no_such_function"), "{html}");
    assert!(!html.contains(token), "{html}");
    let logs = logs.text();
    assert!(logs.contains("rendering the voting page failed"), "{logs}");
    assert!(!logs.contains(token), "{logs}");
    assert!(!logs.contains("alice@example.org"), "{logs}");
}
