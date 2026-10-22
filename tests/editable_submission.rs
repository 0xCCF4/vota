//! Editable Ballots: the voter may change their Response as often as they
//! like, through the voter API and the no-JS form; the last one wins.

#![cfg(feature = "server")]

mod common;

use axum::http::{StatusCode, header};
use common::{TestApp, body_json, body_string};
use serde_json::json;

#[tokio::test]
async fn a_new_submission_through_the_api_replaces_the_response() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "editable").await;
    let path = format!("/api/b/{token}/response");
    app.clock.advance(chrono::Duration::minutes(5));
    app.put_json(&path, &json!({ "option_ids": [options[0]] }))
        .await;
    app.clock.advance(chrono::Duration::hours(2));

    let response = app
        .put_json(&path, &json!({ "option_ids": [options[2]] }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let expected = json!({
        "option_ids": [options[2]],
        "answered_at": "2030-01-15T12:05:00Z",
        "changed_at": "2030-01-15T14:05:00Z",
    });
    let body = body_json(response).await;
    assert_eq!(body["status"], "answered", "{body}");
    assert_eq!(body["response"], expected, "{body}");
    assert_eq!(app.voter_view(&token).await["response"], expected);
}

#[tokio::test]
async fn a_new_multiple_choice_response_replaces_the_whole_selection() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "editable").await;
    let path = format!("/api/b/{token}/response");
    app.put_json(&path, &json!({ "option_ids": [options[0], options[1]] }))
        .await;

    let response = app
        .put_json(&path, &json!({ "option_ids": [options[1], options[2]] }))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        app.voter_view(&token).await["response"]["option_ids"],
        json!([options[1], options[2]])
    );
}

#[tokio::test]
async fn an_invalid_change_keeps_the_previous_response() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "editable").await;
    let path = format!("/api/b/{token}/response");
    app.put_json(&path, &json!({ "option_ids": [options[0]] }))
        .await;
    let before = app.voter_view(&token).await;
    app.clock.advance(chrono::Duration::minutes(1));

    let response = app.put_json(&path, &json!({ "option_ids": [] })).await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(response).await["field"], "option_ids");
    assert_eq!(app.voter_view(&token).await, before);
}

#[tokio::test]
async fn concurrent_changes_all_succeed_and_leave_exactly_one_of_them() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "editable").await;
    let path = format!("/api/b/{token}/response");
    let sets = [
        json!([options[0]]),
        json!([options[1], options[2]]),
        json!([options[0], options[1], options[2]]),
    ];

    let submissions: Vec<_> = (0..15)
        .map(|i| {
            let router = app.router.clone();
            let request = Request::put(&path)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "option_ids": sets[i % 3], "revision": 0 }).to_string(),
                ))
                .unwrap();
            tokio::spawn(async move { router.oneshot(request).await.unwrap() })
        })
        .collect();
    for submission in submissions {
        assert_eq!(submission.await.unwrap().status(), StatusCode::OK);
    }

    let stored = &app.voter_view(&token).await["response"]["option_ids"];
    assert!(
        sets.contains(stored),
        "a mixed selection was stored: {stored}"
    );
}

#[tokio::test]
async fn the_form_records_without_a_confirmation_step_and_redirects() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "editable").await;
    let page = format!("/b/{token}");

    let response = app
        .post_form(&page, &format!("option={}", options[1]))
        .await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], page.as_str());
    let view = app.voter_view(&token).await;
    assert_eq!(view["status"], "answered");
    assert_eq!(view["response"]["option_ids"], json!([options[1]]));
}

#[tokio::test]
async fn reopening_shows_the_form_pre_filled_with_the_current_response() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("multiple", "editable").await;
    let page = format!("/b/{token}");
    app.post_form(
        &page,
        &format!("option={}&option={}", options[0], options[2]),
    )
    .await;

    let response = app.get(&page).await;

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(
        html.contains(r#"<p class="status" data-state="answered" title="Ballot Status: Answered"><span class="visually-hidden">Status: </span>Answered</p>"#),
        "{html}"
    );
    assert!(
        html.contains("Your answer is saved. You can change it until then."),
        "{html}"
    );
    for (id, checked) in [
        (&options[0], true),
        (&options[1], false),
        (&options[2], true),
    ] {
        let checkbox = format!(
            r#"<input type="checkbox" name="option" value="{id}" id="option-{id}"{}>"#,
            if checked { " checked" } else { "" }
        );
        assert!(html.contains(&checkbox), "{checkbox} not in {html}");
    }
    assert!(
        html.contains(r#"<button type="submit">Change answer</button>"#),
        "{html}"
    );
}

#[tokio::test]
async fn a_single_choice_response_pre_selects_its_radio_button() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "editable").await;
    app.put_json(
        &format!("/api/b/{token}/response"),
        &json!({ "option_ids": [options[1]] }),
    )
    .await;

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    for (id, checked) in [
        (&options[0], false),
        (&options[1], true),
        (&options[2], false),
    ] {
        let radio = format!(
            r#"<input type="radio" name="option" value="{id}" id="option-{id}" required{}>"#,
            if checked { " checked" } else { "" }
        );
        assert!(html.contains(&radio), "{radio} not in {html}");
    }
}

#[tokio::test]
async fn changing_the_answer_through_the_form_replaces_it() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "editable").await;
    let page = format!("/b/{token}");
    app.post_form(&page, &format!("option={}", options[0]))
        .await;
    app.clock.advance(chrono::Duration::minutes(30));

    let response = app
        .post_form(&page, &format!("option={}", options[2]))
        .await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let view = app.voter_view(&token).await;
    assert_eq!(
        view["response"],
        json!({
            "option_ids": [options[2]],
            "answered_at": "2030-01-15T12:00:00Z",
            "changed_at": "2030-01-15T12:30:00Z",
        })
    );
}
