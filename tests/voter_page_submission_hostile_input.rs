//! Hostile and invalid input to the no-JS form `POST /b/{token}`, through the
//! HTTP seam. Field errors re-render the voting form (422) with a fixed
//! message and record nothing.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, body_string};

/// `html` without the voting page's own script tag (which the app includes
/// on every voting page), so what remains can be checked for echoed input.
fn without_the_apps_script(html: &str) -> String {
    let Some(start) = html.find(r#"<script type="module" src="/assets/app/"#) else {
        return html.to_owned();
    };
    let end = start + html[start..].find("</script>").unwrap() + "</script>".len();
    format!("{}{}", &html[..start], &html[end..])
}

/// Posts `body` and asserts the 422 voting form, with nothing recorded.
async fn assert_form_rejected(app: &TestApp, token: &str, body: &str) {
    let response = app.post_form(&format!("/b/{token}"), body).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "for {body:?}"
    );
    let html = body_string(response).await;
    assert!(
        html.contains(r#"<p class="message" role="alert">"#),
        "{html}"
    );
    assert!(
        !without_the_apps_script(&html).contains("script"),
        "echoed input for {body:?}: {html}"
    );
    assert_eq!(
        app.voter_view(token).await["status"],
        "unanswered",
        "{body:?}"
    );
}

#[tokio::test]
async fn malformed_option_values_are_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let real = &options[0];
    for value in [
        String::new(),
        "short".into(),
        format!("{real}A"),
        real[..10].to_owned(),
        "AAAAAAAAAA%2B".into(),
        "AAAAAAAAAA%2F".into(),
        "AAAAAAAAAA%3D".into(),
        "AAAAAAAAAAB".into(),
        "..%2F..%2Fetc%2Fp".into(),
        "AAAAAAAAA%00A".into(),
        "AAAAAAAAA%E2%80%AE".into(),
        "AAAAAAAAA%E2%80%8BA".into(),
        "AAAAAAAAA%FFA".into(),
        format!("+{}", &real[1..]),
        "%3Cscript%3E".into(),
        "A".repeat(10_000),
    ] {
        assert_form_rejected(&app, &token, &format!("option={value}&confirm=yes")).await;
    }
}

#[tokio::test]
async fn confirm_must_be_absent_or_exactly_yes() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    for confirm in [
        "confirm=",
        "confirm=no",
        "confirm=YES",
        "confirm=yes%00",
        "confirm=+yes",
        "confirm=yes&confirm=yes",
        "confirm=%3Cscript%3E",
    ] {
        assert_form_rejected(&app, &token, &format!("option={}&{confirm}", options[0])).await;
    }
}

#[tokio::test]
async fn unknown_fields_are_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    for extra in [
        "status=answered",
        "option%5B%5D=x",
        "__proto__=x",
        "Option=x",
        "%3Cscript%3E=1",
        "opt%FFion=x",
    ] {
        assert_form_rejected(
            &app,
            &token,
            &format!("option={}&confirm=yes&{extra}", options[0]),
        )
        .await;
    }
}

#[tokio::test]
async fn non_form_bodies_are_refused() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let body = format!("option={}&confirm=yes", options[0]);
    for content_type in ["application/json", "text/plain", "multipart/form-data", ""] {
        let response = app
            .post_raw(&format!("/b/{token}"), None, content_type, body.clone())
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{content_type}"
        );
    }
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn bodies_over_64_kib_are_refused() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let body = format!("option={}&confirm=yes", options[0]).repeat(3_000);
    assert!(body.len() > 64 * 1024);

    let response = app.post_form(&format!("/b/{token}"), &body).await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn unknown_and_malformed_tokens_get_the_uniform_not_found_page() {
    let app = TestApp::new().await;
    let (real, options) = app.create_standard_ballot().await;
    let unknown = if real.starts_with('A') { "B" } else { "A" }.to_owned() + &real[1..];
    let body = format!("option={}&confirm=yes", options[0]);
    let expected = common::snapshot(app.get(&format!("/b/{unknown}")).await).await;
    assert_eq!(expected.0, StatusCode::NOT_FOUND);

    for path in common::not_found_token_paths("/b/", &unknown) {
        let response = app.post_form(&path, &body).await;
        assert_eq!(common::snapshot(response).await, expected, "{path}");
    }
    assert_eq!(app.voter_view(&real).await["status"], "unanswered");
}

/// Option values no form accepts, URL-encoded.
fn hostile_option_values(real: &str) -> Vec<String> {
    vec![
        String::new(),
        "short".into(),
        format!("{real}A"),
        "AAAAAAAAAA%2B".into(),
        "AAAAAAAAAAB".into(),
        "..%2F..%2Fetc%2Fp".into(),
        "AAAAAAAAA%00A".into(),
        "AAAAAAAAA%E2%80%AE".into(),
        "AAAAAAAAA%E2%80%8BA".into(),
        "AAAAAAAAA%FFA".into(),
        "%3Cscript%3E".into(),
        "A".repeat(10_000),
    ]
}

#[tokio::test]
async fn hostile_values_among_checked_options_are_rejected() {
    let app = TestApp::new().await;
    for submission_mode in ["final", "editable"] {
        let (token, options) = app
            .create_ballot_with_modes("multiple", submission_mode)
            .await;
        for value in hostile_option_values(&options[0]) {
            for body in [
                format!("option={}&option={value}", options[0]),
                format!("option={value}&option={}&confirm=yes", options[1]),
            ] {
                assert_form_rejected(&app, &token, &body).await;
            }
        }
        // Repeating a valid Option many times is a duplicate, not a bigger vote.
        let repeated = format!("option={}&", options[2]).repeat(1_000);
        assert_form_rejected(&app, &token, &repeated).await;
    }
}

#[tokio::test]
async fn confirm_is_still_checked_on_an_editable_ballot() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "editable").await;
    for confirm in [
        "confirm=",
        "confirm=no",
        "confirm=yes&confirm=yes",
        "confirm=%3Cscript%3E",
    ] {
        assert_form_rejected(&app, &token, &format!("option={}&{confirm}", options[0])).await;
    }
}

#[tokio::test]
async fn hostile_changes_on_an_answered_editable_ballot_keep_the_response() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "editable").await;
    let page = format!("/b/{token}");
    app.post_form(&page, &format!("option={}", options[1]))
        .await;
    let before = app.voter_view(&token).await;
    app.clock.advance(chrono::Duration::minutes(1));

    for body in [
        String::new(),
        format!("option={}&option=%3Cscript%3E", options[0]),
        format!("option={}&status=answered", options[0]),
        format!("option={}&option={}", options[0], options[0]),
    ] {
        let response = app.post_form(&page, &body).await;

        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body:?}"
        );
        let html = body_string(response).await;
        assert!(
            !without_the_apps_script(&html).contains("script"),
            "echoed input for {body:?}: {html}"
        );
        assert_eq!(app.voter_view(&token).await, before, "{body:?}");
    }
}
