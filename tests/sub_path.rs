//! Deployment under a sub-path: with `public_base_url =
//! "https://example.org/polls/"` the reverse proxy forwards `/polls/…`
//! unchanged, every route sits under `/polls/`, and every URL the app
//! generates carries the prefix.

#![cfg(feature = "server")]

mod common;

use axum::http::{StatusCode, header};
use common::{SVG_LOGO, TestApp, ballot_request, body_json, body_string, write_file};

const BASE: &str = "https://example.org/polls/";

/// The app under `/polls/`, with the logo `club`.
async fn prefixed_app() -> TestApp {
    TestApp::with_config(|config| {
        config.public_base_url = BASE.parse().unwrap();
        write_file(config.logo_folder.as_ref().unwrap(), "club.svg", SVG_LOGO);
    })
    .await
}

/// Creates a Ballot through the prefixed admin API; returns the response JSON.
async fn create_ballot(app: &TestApp, request: &serde_json::Value) -> serde_json::Value {
    let admin = app.admin_token().await;
    let response = app
        .post_json("/polls/admin/ballots", Some(&admin), request)
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    body_json(response).await
}

#[tokio::test]
async fn the_voting_page_is_served_under_the_prefix() {
    let app = prefixed_app().await;
    let ballot = create_ballot(&app, &ballot_request()).await;
    let token = ballot["token"].as_str().unwrap();

    let response = app.get(&format!("/polls/b/{token}")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(html.contains("<h1>"), "{html}");
}

#[tokio::test]
async fn the_voting_page_links_its_theme_stylesheet_and_logo_under_the_prefix() {
    let app = prefixed_app().await;
    let mut request = ballot_request();
    request["theme"] = "dark".into();
    request["logo"] = "club".into();
    let ballot = create_ballot(&app, &request).await;
    let token = ballot["token"].as_str().unwrap();

    let html = body_string(app.get(&format!("/polls/b/{token}")).await).await;

    assert!(
        html.contains(r#"<link rel="stylesheet" href="/polls/assets/themes/dark/style.css">"#),
        "{html}"
    );
    assert!(
        html.contains(r#"<img class="logo" src="/polls/assets/logos/club" alt="">"#),
        "{html}"
    );
}

#[tokio::test]
async fn the_voting_pages_script_and_its_urls_are_under_the_prefix() {
    let app = prefixed_app().await;
    let ballot = create_ballot(&app, &ballot_request()).await;
    let token = ballot["token"].as_str().unwrap();

    let html = body_string(app.get(&format!("/polls/b/{token}")).await).await;

    assert!(
        html.contains(r#"<link rel="stylesheet" href="/polls/assets/app/ballot.css?v="#),
        "{html}"
    );
    assert!(
        html.contains(r#"<script type="module" src="/polls/assets/app/"#),
        "{html}"
    );
    assert!(
        html.contains(&format!(r#"data-api="/polls/api/b/{token}/response""#)),
        "{html}"
    );
    assert!(
        html.contains(&format!(r#"data-page="/polls/b/{token}""#)),
        "{html}"
    );
}

#[tokio::test]
async fn the_assets_the_page_links_are_served_under_the_prefix() {
    let app = prefixed_app().await;

    for path in [
        "/polls/assets/themes/dark/style.css",
        "/polls/assets/logos/club",
        "/polls/assets/app/0000000000000000/main.js",
        "/polls/assets/app/ballot.css",
    ] {
        assert_eq!(app.get(path).await.status(), StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn the_no_js_form_redirects_back_to_the_page_under_the_prefix() {
    let app = prefixed_app().await;
    let mut request = ballot_request();
    request["submission_mode"] = "editable".into();
    let ballot = create_ballot(&app, &request).await;
    let token = ballot["token"].as_str().unwrap();
    let option = ballot["options"][0]["id"].as_str().unwrap();

    let response = app
        .post_form(&format!("/polls/b/{token}"), &format!("option={option}"))
        .await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        format!("/polls/b/{token}")
    );
}

#[tokio::test]
async fn the_admin_api_returns_ballot_urls_under_the_prefix() {
    let app = prefixed_app().await;

    let ballot = create_ballot(&app, &ballot_request()).await;

    let token = ballot["token"].as_str().unwrap();
    assert_eq!(
        ballot["url"],
        format!("https://example.org/polls/b/{token}")
    );
}

#[tokio::test]
async fn a_base_url_without_trailing_slash_has_the_same_prefix() {
    let app = TestApp::with_config(|config| {
        config.public_base_url = "https://example.org/polls".parse().unwrap();
    })
    .await;

    let ballot = create_ballot(&app, &ballot_request()).await;

    let token = ballot["token"].as_str().unwrap();
    assert_eq!(
        ballot["url"],
        format!("https://example.org/polls/b/{token}")
    );
    let page = app.get(&format!("/polls/b/{token}")).await;
    assert_eq!(page.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_voter_api_answers_under_the_prefix_with_the_logo_url_under_it() {
    let app = prefixed_app().await;
    let mut request = ballot_request();
    request["logo"] = "club".into();
    let ballot = create_ballot(&app, &request).await;
    let token = ballot["token"].as_str().unwrap();

    let response = app.get(&format!("/polls/api/b/{token}")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(
        view["logo_url"],
        "https://example.org/polls/assets/logos/club"
    );
}

#[tokio::test]
async fn the_uniform_not_found_under_the_prefix_is_byte_identical_to_the_root_deployments() {
    let root = TestApp::new().await;
    let app = prefixed_app().await;
    let unknown = "A".repeat(22);
    let reference_page = common::snapshot(root.get(&format!("/b/{unknown}")).await).await;
    let reference_api = common::snapshot(root.get(&format!("/api/b/{unknown}")).await).await;
    assert_eq!(reference_page.0, StatusCode::NOT_FOUND);
    assert_eq!(reference_api.0, StatusCode::NOT_FOUND);

    for path in common::not_found_token_paths("/polls/b/", &unknown) {
        let response = common::snapshot(app.get(&path).await).await;
        assert_eq!(response, reference_page, "{path}");
    }
    for path in common::not_found_token_paths("/polls/api/b/", &unknown) {
        let response = common::snapshot(app.get(&path).await).await;
        assert_eq!(response, reference_api, "{path}");
    }
}

#[tokio::test]
async fn requests_outside_the_prefix_get_a_plain_not_found_that_the_rate_limit_ignores() {
    let app = TestApp::with_config(|config| {
        config.public_base_url = BASE.parse().unwrap();
        config.rate_limit.failed_lookups = 1.try_into().unwrap();
    })
    .await;
    let ballot = create_ballot(&app, &ballot_request()).await;
    let token = ballot["token"].as_str().unwrap();
    let unknown = "A".repeat(22);

    for path in [
        format!("/b/{token}"),
        format!("/b/{unknown}"),
        format!("/api/b/{unknown}"),
        "/admin/ballots".to_owned(),
        "/assets/app/0000000000000000/main.js".to_owned(),
        "/polls".to_owned(),
        "/pollsb/x".to_owned(),
        "/".to_owned(),
    ] {
        let response = app.get_from("192.0.2.7", None, &path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body_string(response).await, "", "{path}");
    }

    let page = app
        .get_from("192.0.2.7", None, &format!("/polls/b/{token}"))
        .await;
    assert_eq!(page.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_health_check_answers_at_the_root_and_under_the_prefix() {
    let app = prefixed_app().await;

    for path in ["/healthz", "/polls/healthz"] {
        let response = app.get_from("127.0.0.1", None, path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(body_json(response).await["status"], "ok", "{path}");
    }
}

#[tokio::test]
async fn the_health_check_refuses_remote_and_forwarded_requests_at_the_root_and_under_the_prefix() {
    let app = prefixed_app().await;

    for (path, unknown) in [
        ("/healthz", "/no-such-path"),
        ("/polls/healthz", "/polls/no-such-path"),
    ] {
        let not_found = common::snapshot(app.get_from("192.0.2.7", None, unknown).await).await;
        assert_eq!(not_found.0, StatusCode::NOT_FOUND, "{unknown}");
        let remote = app.get_from("192.0.2.7", None, path).await;
        let forwarded = app.get_from("127.0.0.1", Some("192.0.2.7"), path).await;
        assert_eq!(common::snapshot(remote).await, not_found, "remote {path}");
        assert_eq!(
            common::snapshot(forwarded).await,
            not_found,
            "forwarded {path}"
        );
    }
}

/// A folder Theme `linked` whose `base.html` links `{{ base_path }}`.
const LINKING_BASE_HTML: &str = r#"<!doctype html>
<html><head><link rel="stylesheet" href="{{ theme_stylesheet }}"></head>
<body><a class="home" href="{{ base_path }}healthz">Status</a>
{% block content %}{% endblock %}</body></html>"#;

/// The voting page of a Ballot in the Theme `linked`, on an app with this
/// public base URL; its page path is `<base path>b/<token>`.
async fn linked_theme_page(public_base_url: &str, base_path: &str) -> String {
    let url = public_base_url.to_owned();
    let app = TestApp::with_config(move |config| {
        config.public_base_url = url.parse().unwrap();
        let folder = config.theme_folder.clone().unwrap();
        write_file(&folder, "linked/style.css", "");
        write_file(&folder, "linked/templates/base.html", LINKING_BASE_HTML);
    })
    .await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["theme"] = "linked".into();
    let response = app
        .post_json(&format!("{base_path}admin/ballots"), Some(&admin), &request)
        .await;
    let token = body_json(response).await["token"]
        .as_str()
        .unwrap()
        .to_owned();
    body_string(app.get(&format!("{base_path}b/{token}")).await).await
}

#[tokio::test]
async fn theme_templates_get_the_base_path_with_leading_and_trailing_slash() {
    let prefixed = linked_theme_page(BASE, "/polls/").await;
    let root = linked_theme_page("https://example.org/", "/").await;

    assert!(
        prefixed.contains(r#"<a class="home" href="/polls/healthz">"#),
        "{prefixed}"
    );
    assert!(
        root.contains(r#"<a class="home" href="/healthz">"#),
        "{root}"
    );
}

#[tokio::test]
async fn a_theme_that_fails_only_under_a_prefix_stops_startup_even_at_the_root() {
    let started = TestApp::try_with_config(|config| {
        let folder = config.theme_folder.clone().unwrap();
        write_file(&folder, "fragile/style.css", "");
        write_file(
            &folder,
            "fragile/templates/base.html",
            r#"{% if base_path != "/" %}{{ no_such_function() }}{% endif %}{% block content %}{% endblock %}"#,
        );
    })
    .await;

    let error = match started {
        Ok(_) => panic!("startup should fail"),
        Err(error) => format!("{error:#}"),
    };
    assert!(error.contains("fragile"), "{error}");
    assert!(error.contains("base.html"), "{error}");
}

/// Paths below `b/` and `api/b/` (after `prefix`) that name no route:
/// trailing slashes, more segments, empty segments.
fn multi_segment_paths(prefix: &str, token: &str, unknown: &str) -> Vec<(String, &'static str)> {
    let page = |path: String| (format!("{prefix}b/{path}"), "page");
    let api = |path: String| (format!("{prefix}api/b/{path}"), "api");
    vec![
        page(format!("{token}/")),
        page(format!("{unknown}/")),
        page(format!("{token}/x")),
        page(format!("{unknown}/x/y")),
        page(format!("/{token}")),
        page(format!("{token}//")),
        page("/".to_owned()),
        page("//x".to_owned()),
        api(format!("{token}/")),
        api(format!("{unknown}/foo")),
        api(format!("{token}/response/x")),
        api(format!("{token}/response/")),
        api(format!("/{token}")),
        api(format!("{unknown}/x/y/z")),
        api("/".to_owned()),
        api("//x".to_owned()),
    ]
}

#[tokio::test]
async fn multi_segment_paths_get_the_uniform_not_found_under_the_prefix_as_at_the_root() {
    let unknown = "A".repeat(22);
    for (base, prefix) in [(None, "/"), (Some(BASE), "/polls/")] {
        let app = TestApp::with_config(|config| {
            if let Some(base) = base {
                config.public_base_url = base.parse().unwrap();
            }
        })
        .await;
        let reference_page = common::snapshot(app.get(&format!("{prefix}b/{unknown}")).await).await;
        let reference_api =
            common::snapshot(app.get(&format!("{prefix}api/b/{unknown}")).await).await;
        let token = create_ballot_at(&app, prefix).await;

        for (path, surface) in multi_segment_paths(prefix, &token, &unknown) {
            let response = common::snapshot(app.get(&path).await).await;
            let reference = if surface == "page" {
                &reference_page
            } else {
                &reference_api
            };
            assert_eq!(&response, reference, "{path}");
        }
    }
}

#[tokio::test]
async fn multi_segment_paths_count_towards_the_rate_limit_under_the_prefix_as_at_the_root() {
    let unknown = "A".repeat(22);
    for (base, prefix) in [(None, "/"), (Some(BASE), "/polls/")] {
        let app = TestApp::with_config(|config| {
            if let Some(base) = base {
                config.public_base_url = base.parse().unwrap();
            }
            config.rate_limit.failed_lookups = 1.try_into().unwrap();
        })
        .await;
        let token = create_ballot_at(&app, prefix).await;

        for (n, (path, _)) in multi_segment_paths(prefix, &token, &unknown)
            .into_iter()
            .enumerate()
        {
            // A client of its own for each path: its one failure blocks it.
            let client = format!("192.0.2.{}", n + 1);
            let failed = app.get_from(&client, None, &path).await;
            assert_eq!(failed.status(), StatusCode::NOT_FOUND, "{path}");
            let valid = app
                .get_from(&client, None, &format!("{prefix}api/b/{token}"))
                .await;
            assert_eq!(valid.status(), StatusCode::TOO_MANY_REQUESTS, "{path}");
        }
    }
}

/// Creates the standard Ballot through the admin API under `prefix`;
/// returns its Ballot Token.
async fn create_ballot_at(app: &TestApp, prefix: &str) -> String {
    let admin = app.admin_token().await;
    let response = app
        .post_json(
            &format!("{prefix}admin/ballots"),
            Some(&admin),
            &ballot_request(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    body_json(response).await["token"]
        .as_str()
        .unwrap()
        .to_owned()
}
