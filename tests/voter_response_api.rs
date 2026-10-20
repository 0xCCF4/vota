//! The voter answers a Final single-choice Ballot through the voter API.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, assert_response_rejected, body_json, snapshot};
use serde_json::json;

#[tokio::test]
async fn submitting_one_option_records_the_response_and_answers_the_ballot() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    app.clock.advance(chrono::Duration::minutes(5));

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [options[1]] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let expected = json!({
        "option_ids": [options[1]],
        "answered_at": "2030-01-15T12:05:00Z",
        "changed_at": "2030-01-15T12:05:00Z",
    });
    let body = body_json(response).await;
    assert_eq!(body["status"], "answered", "{body}");
    assert_eq!(body["response"], expected, "{body}");
    let fetched = app.voter_view(&token).await;
    assert_eq!(fetched["status"], "answered", "{fetched}");
    assert_eq!(fetched["response"], expected, "{fetched}");
}

#[tokio::test]
async fn a_second_submission_on_a_final_ballot_is_rejected_and_changes_nothing() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let path = format!("/api/b/{token}/response");
    app.put_json(&path, &json!({ "option_ids": [options[0]] }))
        .await;
    let before = app.voter_view(&token).await;
    app.clock.advance(chrono::Duration::minutes(1));

    let response = app
        .put_json(&path, &json!({ "option_ids": [options[1]] }))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "already_answered");
    assert_eq!(app.voter_view(&token).await, before);
}

#[tokio::test]
async fn an_option_the_admin_has_since_replaced_answers_ballot_changed() {
    let app = TestApp::new().await;
    let (token, stale, _) = app.create_ballot_with_replaced_options().await;

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [stale[0]] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "ballot_changed");
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn concurrent_submissions_on_a_final_ballot_store_exactly_one_response() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let path = format!("/api/b/{token}/response");

    let submissions: Vec<_> = (0..16)
        .map(|i| {
            let router = app.router.clone();
            let request = Request::put(&path)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "option_ids": [options[i % 2]], "revision": 0 }).to_string(),
                ))
                .unwrap();
            tokio::spawn(async move { router.oneshot(request).await.unwrap() })
        })
        .collect();
    let mut accepted = Vec::new();
    let mut conflicts = 0;
    for submission in submissions {
        let response = submission.await.unwrap();
        match response.status() {
            StatusCode::OK => accepted.push(body_json(response).await),
            StatusCode::CONFLICT => conflicts += 1,
            other => panic!("unexpected status {other}"),
        }
    }

    assert_eq!(accepted.len(), 1);
    assert_eq!(conflicts, 15);
    let stored = app.voter_view(&token).await;
    assert_eq!(stored["response"], accepted[0]["response"]);
    assert_eq!(
        stored["response"]["option_ids"].as_array().unwrap().len(),
        1
    );
}

#[tokio::test]
async fn an_empty_selection_is_rejected() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;
    assert_response_rejected(&app, &token, &json!({ "option_ids": [] }), "option_ids").await;
}

#[tokio::test]
async fn more_than_one_option_on_a_single_choice_ballot_is_rejected() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let body = json!({ "option_ids": [options[0], options[1]] });
    assert_response_rejected(&app, &token, &body, "option_ids").await;
}

#[tokio::test]
async fn an_option_of_another_ballot_answers_exactly_like_one_that_never_existed() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let (_, foreign) = app.create_standard_ballot().await;
    let path = format!("/api/b/{token}/response");
    // Well-formed, but no Ballot has it.
    let nowhere = "AAAAAAAAAAA";

    for (exists_elsewhere, never_existed) in [
        (json!([foreign[0]]), json!([nowhere])),
        (
            json!([options[0], foreign[1]]),
            json!([options[0], nowhere]),
        ),
    ] {
        let foreign = snapshot(
            app.put_json(&path, &json!({ "option_ids": exists_elsewhere }))
                .await,
        )
        .await;
        let unknown = snapshot(
            app.put_json(&path, &json!({ "option_ids": never_existed }))
                .await,
        )
        .await;

        assert_eq!(foreign.0, StatusCode::CONFLICT);
        assert_eq!(
            foreign, unknown,
            "reveals whether an Option ID exists elsewhere"
        );
    }
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn a_duplicated_option_is_rejected_naming_the_duplicate() {
    let app = TestApp::new().await;
    let (token, options) = app.create_standard_ballot().await;
    let body = json!({ "option_ids": [options[0], options[0]] });
    assert_response_rejected(&app, &token, &body, "option_ids[1]").await;
}
