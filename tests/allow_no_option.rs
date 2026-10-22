//! Allow No Option: a multiple-choice Ballot on which the voter may answer
//! with no Option at all ("none of these"), through the admin API, the
//! voter API and the no-JS form.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, assert_response_rejected, ballot_request, body_json};
use serde_json::{Value, json};

/// Creates a multiple-choice Ballot (Pizza, Sushi, Tacos) with these
/// settings on top; returns the admin view.
async fn create(app: &TestApp, extra: Value) -> Value {
    let mut request = ballot_request();
    request["options"] = json!([{ "label": "Pizza" }, { "label": "Sushi" }, { "label": "Tacos" }]);
    request["choice_mode"] = "multiple".into();
    for (key, value) in extra.as_object().unwrap() {
        request[key] = value.clone();
    }
    app.create_ballot(&request).await
}

fn token(ballot: &Value) -> String {
    ballot["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn an_empty_response_is_recorded_when_the_ballot_allows_no_option() {
    let app = TestApp::new().await;
    let created = create(&app, json!({ "allow_no_option": true })).await;
    assert_eq!(created["allow_no_option"], true, "{created}");
    let token = token(&created);

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["status"], "answered", "{body}");
    assert_eq!(body["allow_no_option"], true, "{body}");
    assert_eq!(body["response"]["option_ids"], json!([]), "{body}");
    assert_eq!(
        body["result"],
        json!({ "source": "response", "option_ids": [] })
    );
}

fn option_ids(ballot: &Value) -> Vec<String> {
    ballot["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect()
}

fn admin_path(ballot: &Value) -> String {
    format!("/admin/ballots/{}", ballot["id"])
}

async fn put_empty(app: &TestApp, token: &str) -> axum::response::Response {
    app.put_json(
        &format!("/api/b/{token}/response"),
        &json!({ "option_ids": [] }),
    )
    .await
}

#[tokio::test]
async fn it_is_off_by_default_and_an_empty_response_is_then_refused() {
    let app = TestApp::new().await;
    for extra in [json!({}), json!({ "allow_no_option": false })] {
        let created = create(&app, extra).await;
        assert_eq!(created["allow_no_option"], false, "{created}");
        let token = token(&created);
        assert_eq!(app.voter_view(&token).await["allow_no_option"], false);

        assert_response_rejected(&app, &token, &json!({ "option_ids": [] }), "option_ids").await;
    }
}

#[tokio::test]
async fn a_single_choice_ballot_cant_allow_no_option() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    for choice_mode in [json!({}), json!({ "choice_mode": "single" })] {
        let mut request = ballot_request();
        request["allow_no_option"] = true.into();
        for (key, value) in choice_mode.as_object().unwrap() {
            request[key] = value.clone();
        }

        let response = app
            .post_json("/admin/ballots", Some(&admin), &request)
            .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let error = body_json(response).await;
        assert_eq!(error["field"], "allow_no_option", "{error}");
    }
}

#[tokio::test]
async fn null_or_a_non_boolean_is_rejected() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    for value in [json!(null), json!("true"), json!(1)] {
        let mut request = ballot_request();
        request["choice_mode"] = "multiple".into();
        request["allow_no_option"] = value.clone();

        let response = app
            .post_json("/admin/ballots", Some(&admin), &request)
            .await;

        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "for {value}"
        );
        let created = create(&app, json!({})).await;
        let response = app
            .admin_patch(
                &admin_path(&created),
                &admin,
                &json!({ "allow_no_option": value }),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "edit to {value}"
        );
    }
}

#[tokio::test]
async fn an_empty_response_makes_the_ballot_answered_so_the_fallback_never_applies() {
    let app = TestApp::new().await;
    let created = create(
        &app,
        json!({ "allow_no_option": true, "fallback": ["Sushi"] }),
    )
    .await;
    let token = token(&created);
    assert_eq!(put_empty(&app, &token).await.status(), StatusCode::OK);
    app.clock.set("2030-01-22T12:00:00Z".parse().unwrap());

    let view = app.voter_view(&token).await;
    assert_eq!(view["status"], "answered", "{view}");
    assert_eq!(
        view["result"],
        json!({ "source": "response", "option_ids": [] })
    );

    let admin = app.admin_token().await;
    let ballot = body_json(app.admin_get(&admin_path(&created), &admin).await).await;
    assert_eq!(ballot["status"], "answered", "{ballot}");
    assert_eq!(ballot["result"]["options"], json!([]), "{ballot}");
    assert_eq!(ballot["response"]["option_ids"], json!([]), "{ballot}");
    let answered = body_json(
        app.admin_get("/admin/ballots?status=answered", &admin)
            .await,
    )
    .await;
    assert_eq!(answered["ballots"][0]["id"], created["id"], "{answered}");
}

#[tokio::test]
async fn an_editable_response_can_change_between_no_option_and_options() {
    let app = TestApp::new().await;
    let created = create(
        &app,
        json!({ "allow_no_option": true, "submission_mode": "editable" }),
    )
    .await;
    let token = token(&created);
    let ids = option_ids(&created);
    let path = format!("/api/b/{token}/response");

    assert_eq!(put_empty(&app, &token).await.status(), StatusCode::OK);
    let some = app
        .put_json(&path, &json!({ "option_ids": [ids[1]] }))
        .await;
    assert_eq!(
        body_json(some).await["response"]["option_ids"],
        json!([ids[1]])
    );
    let none = put_empty(&app, &token).await;
    assert_eq!(body_json(none).await["response"]["option_ids"], json!([]));
}

#[tokio::test]
async fn a_final_empty_response_is_locked_like_any_other() {
    let app = TestApp::new().await;
    let created = create(&app, json!({ "allow_no_option": true })).await;
    let token = token(&created);
    assert_eq!(put_empty(&app, &token).await.status(), StatusCode::OK);

    let again = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [option_ids(&created)[0]] }),
        )
        .await;

    assert_eq!(again.status(), StatusCode::CONFLICT);
    assert_eq!(
        app.voter_view(&token).await["response"]["option_ids"],
        json!([])
    );
}

#[tokio::test]
async fn the_admin_can_turn_it_on_and_off_while_the_ballot_is_unanswered() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = create(&app, json!({})).await;
    let path = admin_path(&created);

    let on = app
        .admin_patch(&path, &admin, &json!({ "allow_no_option": true }))
        .await;
    assert_eq!(on.status(), StatusCode::OK);
    let on = body_json(on).await;
    assert_eq!(on["allow_no_option"], true, "{on}");
    // A content edit: the voter must see the Ballot as it now stands.
    assert_eq!(on["status"], "unanswered");
    let token = token(&created);
    assert_eq!(app.voter_view(&token).await["revision"], 1);

    let off = app
        .admin_patch(&path, &admin, &json!({ "allow_no_option": false }))
        .await;
    assert_eq!(body_json(off).await["allow_no_option"], false);
    assert_response_rejected(&app, &token, &json!({ "option_ids": [] }), "option_ids").await;
}

#[tokio::test]
async fn an_empty_response_seen_before_the_admin_turned_it_off_is_not_recorded() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = create(&app, json!({ "allow_no_option": true })).await;
    let token = token(&created);
    let seen = app.voter_view(&token).await["revision"].clone();
    let off = app
        .admin_patch(
            &admin_path(&created),
            &admin,
            &json!({ "allow_no_option": false }),
        )
        .await;
    assert_eq!(off.status(), StatusCode::OK);

    let response = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [], "revision": seen }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "ballot_changed");
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn once_an_empty_response_is_recorded_the_admin_cant_turn_it_off() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = create(
        &app,
        json!({ "allow_no_option": true, "submission_mode": "editable" }),
    )
    .await;
    let token = token(&created);
    assert_eq!(put_empty(&app, &token).await.status(), StatusCode::OK);

    let response = app
        .admin_patch(
            &admin_path(&created),
            &admin,
            &json!({ "allow_no_option": false }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "already_answered");
    let view = app.voter_view(&token).await;
    assert_eq!(view["allow_no_option"], true, "{view}");
    assert_eq!(view["response"]["option_ids"], json!([]), "{view}");
}

#[tokio::test]
async fn switching_to_single_choice_with_it_on_is_refused_unless_it_is_turned_off_too() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = create(&app, json!({ "allow_no_option": true })).await;
    let path = admin_path(&created);

    let refused = app
        .admin_patch(&path, &admin, &json!({ "choice_mode": "single" }))
        .await;
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(refused).await["field"], "choice_mode");

    let refused = app
        .admin_patch(
            &path,
            &admin,
            &json!({ "choice_mode": "single", "allow_no_option": true }),
        )
        .await;
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(refused).await["field"], "allow_no_option");

    let both = app
        .admin_patch(
            &path,
            &admin,
            &json!({ "choice_mode": "single", "allow_no_option": false }),
        )
        .await;
    assert_eq!(both.status(), StatusCode::OK);
    let both = body_json(both).await;
    assert_eq!(both["choice_mode"], "single", "{both}");
    assert_eq!(both["allow_no_option"], false, "{both}");
}

#[tokio::test]
async fn a_single_choice_ballot_cant_be_edited_to_allow_no_option() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;

    let refused = app
        .admin_patch(
            &admin_path(&created),
            &admin,
            &json!({ "allow_no_option": true }),
        )
        .await;

    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(refused).await["field"], "allow_no_option");
    let both = app
        .admin_patch(
            &admin_path(&created),
            &admin,
            &json!({ "allow_no_option": true, "choice_mode": "multiple" }),
        )
        .await;
    assert_eq!(body_json(both).await["allow_no_option"], true);
}

#[tokio::test]
async fn the_fallback_is_still_never_empty() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["choice_mode"] = "multiple".into();
    request["allow_no_option"] = true.into();
    request["fallback"] = json!([]);

    let response = app
        .post_json("/admin/ballots", Some(&admin), &request)
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body_json(response).await["field"], "fallback");
}

// --- The no-JS voting page ---------------------------------------------------

#[tokio::test]
async fn the_legend_says_that_no_option_is_fine() {
    let app = TestApp::new().await;
    let allowed = token(&create(&app, json!({ "allow_no_option": true })).await);
    let not_allowed = token(&create(&app, json!({})).await);

    let html = common::body_string(app.get(&format!("/b/{allowed}")).await).await;
    assert!(
        html.contains("<legend>Choose any number of options (or none)</legend>"),
        "{html}"
    );
    let html = common::body_string(app.get(&format!("/b/{not_allowed}")).await).await;
    assert!(
        html.contains("<legend>Choose one or more options</legend>"),
        "{html}"
    );
}

#[tokio::test]
async fn a_final_form_without_any_option_is_confirmed_then_recorded() {
    let app = TestApp::new().await;
    let created = create(&app, json!({ "allow_no_option": true })).await;
    let token = token(&created);
    let page = format!("/b/{token}");

    // Nothing ticked: the browser sends no `option` field at all.
    let confirmation = app.post_form(&page, "").await;
    assert_eq!(confirmation.status(), StatusCode::OK);
    let confirmation = common::body_string(confirmation).await;
    assert!(
        confirmation.contains("<h2>Confirm your answer</h2>"),
        "{confirmation}"
    );
    assert!(
        confirmation.contains("You chose no option."),
        "{confirmation}"
    );
    assert!(!confirmation.contains(r#"name="option""#), "{confirmation}");
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");

    let recorded = app.post_form(&page, "confirm=yes").await;

    assert_eq!(recorded.status(), StatusCode::SEE_OTHER);
    let view = app.voter_view(&token).await;
    assert_eq!(view["status"], "answered", "{view}");
    assert_eq!(view["response"]["option_ids"], json!([]), "{view}");
    let html = common::body_string(app.get(&page).await).await;
    assert!(html.contains("Your answer has been recorded."), "{html}");
    assert!(html.contains("You chose no option."), "{html}");
    assert!(!html.contains("<form"), "{html}");
}

#[tokio::test]
async fn an_editable_form_without_any_option_is_saved_at_once() {
    let app = TestApp::new().await;
    let created = create(
        &app,
        json!({ "allow_no_option": true, "submission_mode": "editable" }),
    )
    .await;
    let token = token(&created);
    let page = format!("/b/{token}");
    let ids = option_ids(&created);
    assert_eq!(
        app.post_form(&page, &format!("option={}", ids[0]))
            .await
            .status(),
        StatusCode::SEE_OTHER
    );

    let saved = app.post_form(&page, "").await;

    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        app.voter_view(&token).await["response"]["option_ids"],
        json!([])
    );
    let html = common::body_string(app.get(&page).await).await;
    assert!(html.contains("Your answer is saved."), "{html}");
    assert!(!html.contains(" checked"), "{html}");
}

#[tokio::test]
async fn a_closed_ballot_answered_with_no_option_says_so() {
    let app = TestApp::new().await;
    let created = create(&app, json!({ "allow_no_option": true })).await;
    let token = token(&created);
    assert_eq!(put_empty(&app, &token).await.status(), StatusCode::OK);
    app.clock.set("2030-01-22T12:00:00Z".parse().unwrap());

    let html = common::body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(html.contains("You chose no option."), "{html}");
    assert!(!html.contains(r#"<ul class="selection">"#), "{html}");
}

#[tokio::test]
async fn without_the_setting_a_form_without_any_option_is_still_refused() {
    let app = TestApp::new().await;
    let token = token(&create(&app, json!({})).await);

    let response = app.post_form(&format!("/b/{token}"), "confirm=yes").await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = common::body_string(response).await;
    assert!(html.contains("Select at least one Option."), "{html}");
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}

#[tokio::test]
async fn a_malformed_form_is_not_taken_for_an_empty_response() {
    let app = TestApp::new().await;
    let token = token(
        &create(
            &app,
            json!({ "allow_no_option": true, "submission_mode": "editable" }),
        )
        .await,
    );
    let page = format!("/b/{token}");

    for body in ["options=x", "option", "confirm=no", "%FF=1"] {
        let response = app.post_form(&page, body).await;

        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "for {body}"
        );
        assert_eq!(
            app.voter_view(&token).await["status"],
            "unanswered",
            "after {body}"
        );
    }
    // Without the revision, the page was out of date or incomplete.
    let response = app
        .post_raw(&page, None, "application/x-www-form-urlencoded", "")
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(app.voter_view(&token).await["status"], "unanswered");
}
