//! The voter answers a Final single-choice Ballot through the no-JS form:
//! choose, confirm, then post/redirect/get to a read-only view.

#![cfg(feature = "server")]

mod common;

use axum::http::{StatusCode, header};
use common::{TestApp, body_string};

#[tokio::test]
async fn an_unanswered_ballot_shows_its_status() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(
        html.contains(r#"<p class="status" data-state="open" title="Ballot Status: Unanswered"><span class="visually-hidden">Status: </span>Open</p>"#),
        "{html}"
    );
}

#[tokio::test]
async fn choosing_an_option_asks_for_confirmation_before_locking_it_in() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;

    let response = app
        .post_form(&format!("/b/{token}"), &format!("option={}", options[1]))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(html.contains("<h2>Confirm your answer</h2>"), "{html}");
    assert!(html.contains("<li>Sushi</li>"), "{html}");
    assert!(!html.contains("Pizza"), "{html}");
    assert!(
        html.contains("You cannot change your answer after you confirm it."),
        "{html}"
    );
    let hidden = format!(
        r#"<input type="hidden" name="option" value="{}">"#,
        options[1]
    );
    assert!(html.contains(&hidden), "{html}");
    assert!(
        html.contains(
            r#"<button type="submit" name="confirm" value="yes">Confirm and submit</button>"#
        ),
        "{html}"
    );
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn confirming_records_the_response_and_redirects_to_a_read_only_view() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let page = format!("/b/{token}");

    let response = app
        .post_form(&page, &format!("option={}&confirm=yes", options[1]))
        .await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], page.as_str());
    let html = body_string(app.get(&page).await).await;
    assert!(
        html.contains(r#"<p class="status" data-state="answered" title="Ballot Status: Answered"><span class="visually-hidden">Status: </span>Answered</p>"#),
        "{html}"
    );
    assert!(html.contains("Your answer has been recorded."), "{html}");
    assert!(html.contains("<li>Sushi</li>"), "{html}");
    assert!(!html.contains("Pizza"), "{html}");
    assert!(!html.contains("<form"), "{html}");
    assert!(!html.contains("<input"), "{html}");
    let view = app.voter_view(&token).await;
    assert_eq!(view["status"], "answered");
    assert_eq!(
        view["response"]["option_ids"],
        serde_json::json!([options[1]])
    );
}

#[tokio::test]
async fn submitting_again_on_an_answered_final_ballot_is_refused() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let page = format!("/b/{token}");
    app.post_form(&page, &format!("option={}&confirm=yes", options[0]))
        .await;
    let before = app.voter_view(&token).await;

    for body in [
        format!("option={}", options[1]),
        format!("option={}&confirm=yes", options[1]),
    ] {
        let response = app.post_form(&page, &body).await;

        assert_eq!(response.status(), StatusCode::CONFLICT, "{body}");
        let html = body_string(response).await;
        assert!(
            html.contains("This Ballot has already been answered. Your answer cannot be changed."),
            "{html}"
        );
        assert!(html.contains("<li>Pizza</li>"), "{html}");
        assert!(!html.contains("<form"), "{html}");
    }
    assert_eq!(app.voter_view(&token).await, before);
}

#[tokio::test]
async fn an_option_the_admin_has_since_replaced_shows_the_changed_ballot_with_a_notice() {
    let app = TestApp::new().await;
    let (token, stale, current) = app.create_ballot_with_replaced_options().await;

    // Before and on the confirmation step alike.
    for body in [
        format!("option={}", stale[0]),
        format!("option={}&confirm=yes", stale[0]),
    ] {
        let response = app.post_form(&format!("/b/{token}"), &body).await;

        assert_eq!(response.status(), StatusCode::CONFLICT, "{body}");
        let html = body_string(response).await;
        assert!(
            html.contains(
                r#"<p class="message" role="alert">This Ballot was changed by its organiser, so your answer was not saved. Please check it and answer again.</p>"#
            ),
            "{html}"
        );
        let fresh = format!(
            r#"<input type="radio" name="option" value="{}""#,
            current[0]
        );
        assert!(html.contains(&fresh), "{html}");
        assert!(html.contains("Curry"), "{html}");
        assert!(!html.contains(&stale[0]), "{html}");
    }
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

/// Posts `body` to the form and asserts the voting form comes back (422) with
/// `message`, and nothing was recorded.
async fn assert_form_rejected(app: &TestApp, token: &str, body: &str, message: &str) {
    let response = app.post_form(&format!("/b/{token}"), body).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "for {body:?}"
    );
    let html = body_string(response).await;
    assert!(
        html.contains(&format!(r#"<p class="message" role="alert">{message}</p>"#)),
        "for {body:?}: {html}"
    );
    assert!(
        html.contains(r#"<input type="radio" name="option""#),
        "{html}"
    );
    assert_eq!(
        app.voter_view(token).await["status"],
        "unanswered",
        "{body:?}"
    );
}

#[tokio::test]
async fn invalid_selections_re_render_the_form_naming_the_problem() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let (_, foreign) = app.create_standard_ballot().await;
    for (body, message) in [
        (String::new(), "Select exactly one Option."),
        ("confirm=yes".into(), "Select exactly one Option."),
        (
            format!("option={}&option={}&confirm=yes", options[0], options[1]),
            "Select exactly one Option.",
        ),
        (
            format!("option={}&option={}&confirm=yes", options[0], options[0]),
            "Select each Option only once.",
        ),
        (
            format!("option={}&option=not-an-id&confirm=yes", foreign[0]),
            "That Option is not part of this Ballot.",
        ),
        (
            "option=not-an-id&confirm=yes".into(),
            "That Option is not part of this Ballot.",
        ),
    ] {
        assert_form_rejected(&app, &token, &body, message).await;
    }
}

#[tokio::test]
async fn an_unreadable_form_or_confirmation_is_named_without_echoing_it() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    for (body, message) in [
        (
            format!("option={}&confirm=maybe", options[0]),
            "The confirmation was not valid. Please try again.",
        ),
        (
            format!("option={}&confirm=yes&vote=maybe", options[0]),
            "The form could not be read. Please try again.",
        ),
        (
            format!("option={}&confirm=yes&maybe=%FF", options[0]),
            "The form could not be read. Please try again.",
        ),
    ] {
        assert_form_rejected(&app, &token, &body, message).await;
        let html = body_string(app.post_form(&format!("/b/{token}"), &body).await).await;
        assert!(!html.contains("maybe"), "echoed input for {body:?}: {html}");
    }
}

#[tokio::test]
async fn a_response_submitted_through_the_api_shows_read_only_on_the_page() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    app.put_json(
        &format!("/api/b/{token}/response"),
        &serde_json::json!({ "option_ids": [options[0]] }),
    )
    .await;

    let response = app.get(&format!("/b/{token}")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(
        html.contains(r#"<p class="status" data-state="answered" title="Ballot Status: Answered"><span class="visually-hidden">Status: </span>Answered</p>"#),
        "{html}"
    );
    assert!(html.contains("<li>Pizza</li>"), "{html}");
    assert!(!html.contains("<form"), "{html}");
}
