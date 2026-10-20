//! The voting page's JavaScript enhancement: one self-hosted script (and its
//! stylesheet), served from a fixed asset route and included on every voting
//! page state by the app itself, so a Theme's templates can't drop it. What
//! the script needs comes in `data-*` attributes on its own `<script>` tag;
//! there is never inline script.

#![cfg(feature = "server")]

mod common;

use axum::http::{StatusCode, header};
use chrono::Duration;
use common::{TestApp, ballot_request, body_string, write_file};
use http_body_util::BodyExt;
use serde_json::json;
use sha2::{Digest as _, Sha256};

const SCRIPT: &str = "/assets/app/ballot.js";
const STYLESHEET: &str = "/assets/app/ballot.css";

#[tokio::test]
async fn the_script_is_served_from_our_own_host_with_safe_headers() {
    let app = TestApp::new().await;

    let response = app.get(SCRIPT).await;

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=3600");
    let body = body_string(response).await;
    assert!(body.contains("fetch("), "{body}");
}

#[tokio::test]
async fn the_scripts_stylesheet_is_served_from_our_own_host_with_safe_headers() {
    let app = TestApp::new().await;

    let response = app.get(STYLESHEET).await;

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers[header::CONTENT_TYPE], "text/css; charset=utf-8");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=3600");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let css = String::from_utf8(body.to_vec()).unwrap();
    // Built on the Themes' custom properties, loading nothing else.
    assert!(css.contains("var(--accent"), "{css}");
    assert!(!css.contains("url("), "{css}");
    assert!(!css.contains("@import"), "{css}");
}

#[tokio::test]
async fn a_throttled_client_still_gets_the_script_and_its_stylesheet() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = 1.try_into().unwrap();
    })
    .await;
    for _ in 0..2 {
        app.get_from("192.0.2.9", None, "/b/AAAAAAAAAAAAAAAAAAAAAA")
            .await;
    }
    let blocked = app
        .get_from("192.0.2.9", None, "/b/AAAAAAAAAAAAAAAAAAAAAA")
        .await;
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);

    for path in [SCRIPT, STYLESHEET] {
        let response = app.get_from("192.0.2.9", None, path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
}

/// The URL the voting page loads the asset at `path` from: `path?v=` and
/// a version of 16 lowercase hex digits.
async fn versioned_url(app: &TestApp, path: &str) -> String {
    let (token, _) = app.create_ballot_with_modes("single", "final").await;
    let html = body_string(app.get(&format!("/b/{token}")).await).await;
    let start = html
        .find(&format!("\"{path}?v="))
        .unwrap_or_else(|| panic!("no versioned {path} in {html}"))
        + 1;
    let url = &html[start..start + html[start..].find('"').unwrap()];
    let version = &url[path.len() + 3..];
    assert_eq!(version.len(), 16, "{url}");
    assert!(
        version
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{url}"
    );
    url.to_owned()
}

#[tokio::test]
async fn the_script_and_its_stylesheet_are_cached_for_good_under_their_versioned_urls() {
    let app = TestApp::new().await;

    for path in [SCRIPT, STYLESHEET] {
        let url = versioned_url(&app, path).await;
        let response = app.get(&url).await;

        assert_eq!(response.status(), StatusCode::OK, "{url}");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable",
            "{url}"
        );
        let unversioned = body_string(app.get(path).await).await;
        let body = body_string(response).await;
        assert_eq!(body, unversioned, "{url}");
        // The version is the content's: the start of its SHA-256.
        let digest = Sha256::digest(body.as_bytes());
        let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        assert!(url.ends_with(&format!("?v={hex}")), "{url}");
    }
}

#[tokio::test]
async fn an_old_or_odd_version_still_gets_the_current_asset_cached_briefly() {
    let app = TestApp::new().await;

    for path in [SCRIPT, STYLESHEET] {
        let current = versioned_url(&app, path).await;
        let version = &current[path.len() + 3..];
        let current_body = body_string(app.get(&current).await).await;
        for query in [
            "v=0000000000000000".to_owned(),
            format!("v={}", version.to_uppercase()),
            format!("v={version}&v={version}"),
            format!("v={version}x"),
            format!("x=1&v={version}"),
            "v=".to_owned(),
            "v=%00".to_owned(),
        ] {
            let response = app.get(&format!("{path}?{query}")).await;

            assert_eq!(response.status(), StatusCode::OK, "{path}?{query}");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "public, max-age=3600",
                "{path}?{query}"
            );
            assert_eq!(body_string(response).await, current_body, "{path}?{query}");
        }
    }
}

/// The voting page of the Ballot with this token, as HTML, with the app
/// assets' versions (`?v=` and 16 characters) left out.
async fn page(app: &TestApp, token: &str) -> String {
    let response = app.get(&format!("/b/{token}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    unversioned(&body_string(response).await)
}

/// `html` without the versions of the app assets' URLs.
fn unversioned(html: &str) -> String {
    let mut out = html.to_owned();
    for path in [SCRIPT, STYLESHEET] {
        let versioned = format!("{path}?v=");
        if let Some(at) = out.find(&versioned) {
            out.replace_range(at + path.len()..at + versioned.len() + 16, "");
        }
    }
    out
}

#[tokio::test]
async fn the_voting_page_loads_the_script_with_what_it_needs_in_data_attributes() {
    let app = TestApp::new().await;
    let (token, _) = app.create_ballot_with_modes("single", "final").await;

    let html = page(&app, &token).await;

    let expected = format!(
        r#"<link rel="stylesheet" href="/assets/app/ballot.css">
<script src="/assets/app/ballot.js" defer data-api="/api/b/{token}/response" data-page="/b/{token}" data-deadline="2030-01-22T12:00:00Z" data-now="2030-01-15T12:00:00Z" data-open="true" data-revision="0" data-choice-mode="single" data-submission-mode="final" data-needs-confirmation="true"></script>
</head>"#
    );
    assert!(html.contains(&expected), "{expected}\nnot in\n{html}");
}

/// The `<script …>` start tag of the voting page's script in `html`.
fn script_tag(html: &str) -> &str {
    let start = html
        .find(r#"<script src="/assets/app/ballot.js""#)
        .unwrap_or_else(|| panic!("no script in {html}"));
    let end = start + html[start..].find('>').unwrap();
    &html[start..=end]
}

#[tokio::test]
async fn the_data_attributes_follow_the_ballots_modes() {
    let app = TestApp::new().await;
    let (token, _) = app.create_ballot_with_modes("multiple", "editable").await;

    let html = page(&app, &token).await;

    let tag = script_tag(&html);
    assert!(tag.contains(r#" data-choice-mode="multiple""#), "{tag}");
    assert!(tag.contains(r#" data-submission-mode="editable""#), "{tag}");
    assert!(tag.contains(r#" data-needs-confirmation="false""#), "{tag}");
}

#[tokio::test]
async fn once_the_deadline_has_passed_the_script_is_told_voting_is_closed() {
    let app = TestApp::new().await;
    let (token, _) = app.create_ballot_with_modes("single", "editable").await;
    app.clock.advance(Duration::days(8));

    let html = page(&app, &token).await;

    let tag = script_tag(&html);
    assert!(tag.contains(r#" data-open="false""#), "{tag}");
    assert!(tag.contains(r#" data-now="2030-01-23T12:00:00Z""#), "{tag}");
}

/// Asserts the page loads the script exactly once, same-origin, and has no
/// inline script or event-handler attribute (which the Content Security
/// Policy would block).
fn assert_script_included_and_csp_compatible(html: &str, state: &str) {
    assert_eq!(html.matches("<script").count(), 1, "{state}: {html}");
    let tag = script_tag(html);
    assert!(tag.contains(" defer "), "{state}: {tag}");
    assert_eq!(
        html.matches(r#"<link rel="stylesheet" href="/assets/app/ballot.css">"#)
            .count(),
        1,
        "{state}: {html}"
    );
    let lower = html.to_ascii_lowercase();
    for handler in [
        " onclick",
        " onsubmit",
        " onload",
        " onerror",
        " onchange",
        "javascript:",
    ] {
        assert!(!lower.contains(handler), "{state}: {handler} in {html}");
    }
}

#[tokio::test]
async fn every_voting_page_state_includes_the_script() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["fallback"] = json!(["Sushi"]);
    let final_ballot = app.create_ballot(&request).await;
    let token = final_ballot["token"].as_str().unwrap();
    let pizza = final_ballot["options"][0]["id"].as_str().unwrap();
    let page_path = format!("/b/{token}");
    let (editable, editable_options) = app.create_ballot_with_modes("multiple", "editable").await;
    let unanswered = app.create_ballot(&request).await;

    // Open, with the upcoming Fallback.
    assert_script_included_and_csp_compatible(&page(&app, token).await, "open");
    // An invalid form post (422, the form again with a message).
    let invalid = app.post_form(&page_path, "").await;
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_script_included_and_csp_compatible(&unversioned(&body_string(invalid).await), "invalid");
    // Answered (Final): read-only.
    let recorded = app
        .post_form(&page_path, &format!("option={pizza}&confirm=yes"))
        .await;
    assert_eq!(recorded.status(), StatusCode::SEE_OTHER);
    assert_script_included_and_csp_compatible(&page(&app, token).await, "answered");
    // A refused second answer (409).
    let refused = app
        .post_form(&page_path, &format!("option={pizza}&confirm=yes"))
        .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_script_included_and_csp_compatible(&unversioned(&body_string(refused).await), "refused");
    // Answered (Editable): the form, pre-filled.
    let saved = app
        .post_form(
            &format!("/b/{editable}"),
            &format!("option={}", editable_options[1]),
        )
        .await;
    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    assert_script_included_and_csp_compatible(&page(&app, &editable).await, "editable answered");
    // Closed: with a Response, and with the Fallback applied.
    app.clock.advance(Duration::days(8));
    assert_script_included_and_csp_compatible(&page(&app, token).await, "closed");
    let auto_fallback = page(&app, unanswered["token"].as_str().unwrap()).await;
    assert!(
        auto_fallback.contains("Fallback applied"),
        "{auto_fallback}"
    );
    assert_script_included_and_csp_compatible(&auto_fallback, "auto-fallback");
}

#[tokio::test]
async fn a_folder_theme_overriding_base_html_still_gets_the_script_in_its_head() {
    let base = r#"<!doctype html><html><head><link rel="stylesheet" href="{{ theme_stylesheet }}"></HEAD>
<body class="festive">{% block content %}{% endblock %}</body></html>"#;
    let app = TestApp::with_config(|config| {
        let folder = config.theme_folder.clone().unwrap();
        write_file(&folder, "festive/style.css", "");
        write_file(&folder, "festive/templates/base.html", base);
    })
    .await;
    let mut request = ballot_request();
    request["theme"] = json!("festive");
    let token = app.create_ballot(&request).await["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let html = page(&app, &token).await;

    assert_script_included_and_csp_compatible(&html, "festive");
    let head_end = html.find("</HEAD>").unwrap();
    assert!(html.find("<script").unwrap() < head_end, "{html}");
    assert!(html.contains(r#"<body class="festive">"#), "{html}");
}

#[tokio::test]
async fn a_folder_theme_without_a_head_gets_the_script_at_the_end_of_its_body() {
    let base = r#"<!doctype html><title>{{ title }}</title>
<link rel="stylesheet" href="{{ theme_stylesheet }}">
<body>{% block content %}{% endblock %}</body>"#;
    let app = TestApp::with_config(|config| {
        let folder = config.theme_folder.clone().unwrap();
        write_file(&folder, "bare/style.css", "");
        write_file(&folder, "bare/templates/base.html", base);
    })
    .await;
    let mut request = ballot_request();
    request["theme"] = json!("bare");
    let token = app.create_ballot(&request).await["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let html = page(&app, &token).await;

    assert_script_included_and_csp_compatible(&html, "bare");
    assert!(html.trim_end().ends_with("></script>\n</body>"), "{html}");
}
