//! Every voting page state carries a Content Security Policy that allows no
//! third-party sources, and the rendered pages reference nothing outside our
//! own host. The not-found page stays neutral and unthemed.

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use chrono::Duration;
use common::{SVG_LOGO, TestApp, ballot_request, body_string, write_file};
use serde_json::{Value, json};

/// Asserts the page's security headers and that it references no other
/// host; returns the HTML.
async fn assert_self_contained(response: Response<Body>, state: &str) -> String {
    let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_owned();
    for directive in [
        "default-src 'none'",
        "script-src 'self'",
        "img-src 'self'",
        "connect-src 'self'",
        "form-action 'self'",
        "base-uri 'none'",
        "frame-ancestors 'none'",
    ] {
        assert!(csp.contains(directive), "{state}: {directive} not in {csp}");
    }
    assert!(csp.contains("style-src 'self' 'nonce-"), "{state}: {csp}");
    assert!(!csp.contains("unsafe-inline"), "{state}: {csp}");
    assert!(!csp.contains("http"), "{state}: {csp}");
    assert!(!csp.contains('*'), "{state}: {csp}");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff",
        "{state}"
    );
    let html = body_string(response).await;
    for external in ["http://", "https://", "src=\"//", "href=\"//", "url("] {
        assert!(!html.contains(external), "{state}: {external} in {html}");
    }
    html
}

async fn app() -> TestApp {
    TestApp::with_config(|config| {
        write_file(&config.logo_folder.clone().unwrap(), "club.svg", SVG_LOGO);
    })
    .await
}

fn tweaked(submission_mode: &str) -> Value {
    let mut request = ballot_request();
    request["submission_mode"] = json!(submission_mode);
    request["accent_color"] = json!("#123456");
    request["logo"] = json!("club");
    request["fallback"] = json!(["Sushi"]);
    request
}

#[tokio::test]
async fn every_voting_page_state_has_a_strict_content_security_policy() {
    let app = app().await;
    let created = app.create_ballot(&tweaked("final")).await;
    let token = created["token"].as_str().unwrap();
    let page = format!("/b/{token}");
    let pizza = created["options"][0]["id"].as_str().unwrap();

    // The open form, with the upcoming Fallback.
    let open = app.get(&page).await;
    assert_eq!(open.status(), StatusCode::OK);
    assert_self_contained(open, "open").await;
    // An invalid form post.
    let invalid = app.post_form(&page, "").await;
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_self_contained(invalid, "invalid").await;
    // The confirmation step.
    let confirm = app.post_form(&page, &format!("option={pizza}")).await;
    assert_eq!(confirm.status(), StatusCode::OK);
    assert_self_contained(confirm, "confirm").await;
    // Answered: read-only.
    let recorded = app
        .post_form(&page, &format!("option={pizza}&confirm=yes"))
        .await;
    assert_eq!(recorded.status(), StatusCode::SEE_OTHER);
    let answered = app.get(&page).await;
    assert_eq!(answered.status(), StatusCode::OK);
    assert_self_contained(answered, "answered").await;
    // A refused second answer.
    let refused = app
        .post_form(&page, &format!("option={pizza}&confirm=yes"))
        .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_self_contained(refused, "refused").await;
    // Closed.
    app.clock.advance(Duration::days(8));
    let closed = app.get(&page).await;
    assert_eq!(closed.status(), StatusCode::OK);
    assert_self_contained(closed, "closed").await;
}

#[tokio::test]
async fn a_closed_ballot_with_the_fallback_applied_has_the_policy_too() {
    let app = app().await;
    let created = app.create_ballot(&tweaked("editable")).await;
    let token = created["token"].as_str().unwrap();
    app.clock.advance(Duration::days(8));

    let response = app.get(&format!("/b/{token}")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let html = assert_self_contained(response, "auto-fallback").await;
    assert!(
        html.contains("No answer was given, so this was recorded:"),
        "{html}"
    );
}

#[tokio::test]
async fn the_not_found_page_is_neutral_and_unthemed() {
    let app = app().await;

    let response = app.get("/b/AAAAAAAAAAAAAAAAAAAAAA").await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let html = body_string(response).await;
    assert!(!html.contains("/assets/"), "{html}");
    assert!(!html.contains("--accent"), "{html}");
}
