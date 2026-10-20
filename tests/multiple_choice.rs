//! Multiple-choice Ballots: one or more Options, through the voter API and the
//! no-JS form.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, assert_response_rejected, body_json, body_string};
use serde_json::json;

#[tokio::test]
async fn submitting_several_options_through_the_api_records_them_all() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "final").await;

    // Submitted out of order; the Response lists them in the Ballot's order.
    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[2], options[0]] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["status"], "answered", "{body}");
    assert_eq!(
        body["response"]["option_ids"],
        json!([options[0], options[2]]),
        "{body}"
    );
    assert_eq!(app.voter_view(&token).await["response"], body["response"]);
}

#[tokio::test]
async fn every_option_at_once_is_accepted() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "final").await;

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": options }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await["response"]["option_ids"],
        json!(options)
    );
}

#[tokio::test]
async fn invalid_selections_through_the_api_are_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "final").await;
    let (_, foreign) = app.create_ballot_with_modes("multiple", "final").await;
    for (body, field) in [
        (json!({ "option_ids": [] }), "option_ids"),
        (
            json!({ "option_ids": [options[0], options[1], options[0]] }),
            "option_ids[2]",
        ),
        // Malformed IDs and duplicates are named even beside an Option
        // that isn't this Ballot's (which alone would be `ballot_changed`).
        (
            json!({ "option_ids": [foreign[1], "AAAAAAAAAAB"] }),
            "option_ids[1]",
        ),
        (
            json!({ "option_ids": [foreign[1], foreign[1]] }),
            "option_ids[1]",
        ),
        (
            json!({ "option_ids": [options[0], "AAAAAAAAAAB"] }),
            "option_ids[1]",
        ),
        (json!({ "option_ids": [options[0], null] }), "option_ids[1]"),
    ] {
        assert_response_rejected(&app, &token, &body, field).await;
    }
}

#[tokio::test]
async fn the_voting_page_shows_checkboxes() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "final").await;

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(
        html.contains("<legend>Choose one or more options</legend>"),
        "{html}"
    );
    for id in &options {
        let checkbox =
            format!(r#"<input type="checkbox" name="option" value="{id}" id="option-{id}">"#);
        assert!(html.contains(&checkbox), "{checkbox} not in {html}");
    }
    assert!(!html.contains(r#"type="radio""#), "{html}");
}

#[tokio::test]
async fn several_checked_options_are_confirmed_then_recorded_through_the_form() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "final").await;
    let page = format!("/b/{token}");
    let chosen = format!("option={}&option={}", options[0], options[2]);

    let confirmation = body_string(app.post_form(&page, &chosen).await).await;

    assert!(
        confirmation.contains("<h2>Confirm your answer</h2>"),
        "{confirmation}"
    );
    assert!(confirmation.contains("<li>Pizza</li>"), "{confirmation}");
    assert!(confirmation.contains("<li>Tacos</li>"), "{confirmation}");
    assert!(!confirmation.contains("Sushi"), "{confirmation}");
    for id in [&options[0], &options[2]] {
        let hidden = format!(r#"<input type="hidden" name="option" value="{id}">"#);
        assert!(confirmation.contains(&hidden), "{confirmation}");
    }

    let response = app.post_form(&page, &format!("{chosen}&confirm=yes")).await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let html = body_string(app.get(&page).await).await;
    assert!(html.contains("Your answer has been recorded."), "{html}");
    assert!(html.contains("<li>Pizza</li>"), "{html}");
    assert!(html.contains("<li>Tacos</li>"), "{html}");
    assert!(!html.contains("<form"), "{html}");
    assert_eq!(
        app.voter_view(&token).await["response"]["option_ids"],
        json!([options[0], options[2]])
    );
}

#[tokio::test]
async fn invalid_form_selections_re_render_the_checkboxes_naming_the_problem() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "final").await;
    let (_, foreign) = app.create_ballot_with_modes("multiple", "final").await;
    let not_part = "That Option is not part of this Ballot.";
    for (body, message) in [
        (String::new(), "Select at least one Option."),
        ("confirm=yes".into(), "Select at least one Option."),
        (
            format!("option={}&option={}&confirm=yes", options[1], options[1]),
            "Select each Option only once.",
        ),
        // Malformed values and duplicates are named even beside an Option
        // that isn't this Ballot's (which alone shows the changed Ballot).
        (
            format!("option={}&option=not-an-id&confirm=yes", foreign[1]),
            not_part,
        ),
        (
            format!("option={}&option={}&confirm=yes", foreign[1], foreign[1]),
            "Select each Option only once.",
        ),
        (
            format!("option={}&option=not-an-id&confirm=yes", options[0]),
            not_part,
        ),
        (
            format!("option={}&option=&confirm=yes", options[0]),
            not_part,
        ),
    ] {
        let response = app.post_form(&format!("/b/{token}"), &body).await;

        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body:?}"
        );
        let html = body_string(response).await;
        assert!(
            html.contains(&format!(r#"<p class="message" role="alert">{message}</p>"#)),
            "for {body:?}: {html}"
        );
        assert!(
            html.contains(r#"<input type="checkbox" name="option""#),
            "{html}"
        );
        assert_eq!(
            app.voter_view(&token).await["status"],
            "unanswered",
            "{body:?}"
        );
    }
}

#[tokio::test]
async fn a_final_multiple_choice_response_cannot_be_changed() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "final").await;
    let path = format!("/api/b/{token}/response");
    app.put_json(&path, &json!({ "option_ids": [options[0], options[1]] }))
        .await;
    let before = app.voter_view(&token).await;

    let response = app
        .put_json(&path, &json!({ "option_ids": [options[2]] }))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(app.voter_view(&token).await, before);
}
