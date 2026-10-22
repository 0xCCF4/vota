//! The Fallback: an optional set of the Ballot's own Options, set by the admin
//! at creation (by Option label), that becomes the Ballot's result when the
//! Voting Deadline passes without a Response (Ballot Status Auto-fallback).

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use common::{TestApp, ballot_request, body_string};
use serde_json::{Value, json};

/// The Voting Deadline of every Ballot built from `common::ballot_request`.
fn deadline() -> DateTime<Utc> {
    "2030-01-22T12:00:00Z".parse().unwrap()
}

/// A Ballot request with the Options Mon, Tue, Wed, Thu and this Choice Mode
/// and Fallback.
fn request_with_fallback(choice_mode: &str, fallback: Value) -> Value {
    let mut request = ballot_request();
    request["options"] = json!([
        { "label": "Mon" },
        { "label": "Tue" },
        { "label": "Wed" },
        { "label": "Thu" },
    ]);
    request["choice_mode"] = json!(choice_mode);
    request["fallback"] = fallback;
    request
}

/// The `{id, label}` objects of these Options of a created Ballot.
fn options_labelled(created: &Value, labels: &[&str]) -> Value {
    created["options"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| labels.contains(&o["label"].as_str().unwrap()))
        .cloned()
        .collect()
}

#[tokio::test]
async fn admin_sets_a_fallback_by_option_labels_and_sees_it_in_option_order() {
    let app = TestApp::new().await;

    let created = app
        .create_ballot(&request_with_fallback("multiple", json!(["Thu", "Tue"])))
        .await;

    assert_eq!(
        created["fallback"],
        options_labelled(&created, &["Tue", "Thu"]),
        "{created}"
    );
}

#[tokio::test]
async fn a_ballot_has_no_fallback_unless_the_admin_sets_one() {
    let app = TestApp::new().await;

    let created = app.create_ballot(&ballot_request()).await;

    assert_eq!(created["fallback"], json!(null), "{created}");
    assert!(created.as_object().unwrap().contains_key("fallback"));
}

/// Creates a Ballot with these modes and Fallback; returns its token and the
/// IDs of the Options Mon, Tue, Wed, Thu.
async fn create(
    app: &TestApp,
    choice_mode: &str,
    submission_mode: &str,
    fallback: Option<Value>,
) -> (String, Vec<String>) {
    let mut request = request_with_fallback(choice_mode, fallback.unwrap_or(json!(null)));
    if request["fallback"].is_null() {
        request.as_object_mut().unwrap().remove("fallback");
    }
    request["submission_mode"] = json!(submission_mode);
    let created = app.create_ballot(&request).await;
    let ids = created["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect();
    (created["token"].as_str().unwrap().to_owned(), ids)
}

#[tokio::test]
async fn without_a_response_the_fallback_applies_once_the_deadline_passes() {
    let app = TestApp::new().await;
    let (token, ids) = create(&app, "multiple", "final", Some(json!(["Tue", "Thu"]))).await;
    app.clock.set(deadline());

    let view = app.voter_view(&token).await;

    assert_eq!(view["status"], "auto_fallback", "{view}");
    assert_eq!(view["response"], json!(null), "{view}");
    assert_eq!(
        view["result"],
        json!({ "source": "fallback", "option_ids": [ids[1], ids[3]] }),
        "{view}"
    );
}

#[tokio::test]
async fn ballot_status_and_result_for_every_combination_across_the_deadline() {
    let app = TestApp::new().await;
    for submission_mode in ["final", "editable"] {
        for with_fallback in [false, true] {
            for with_response in [false, true] {
                let case = format!(
                    "{submission_mode}, fallback: {with_fallback}, response: {with_response}"
                );
                app.clock.set(common::start_time());
                let fallback = with_fallback.then(|| json!(["Wed"]));
                let (token, ids) = create(&app, "single", submission_mode, fallback).await;
                if with_response {
                    let response = app
                        .put_json(
                            &format!("/api/b/{token}/response"),
                            &json!({ "option_ids": [ids[0]] }),
                        )
                        .await;
                    assert_eq!(response.status(), StatusCode::OK, "{case}");
                }
                let response_result = json!({ "source": "response", "option_ids": [ids[0]] });
                let fallback_result = json!({ "source": "fallback", "option_ids": [ids[2]] });

                // Just before the deadline: the Fallback never applies.
                app.clock
                    .set(deadline() - chrono::Duration::milliseconds(1));
                let before = app.voter_view(&token).await;
                let (status, result) = if with_response {
                    ("answered", response_result.clone())
                } else {
                    ("unanswered", json!(null))
                };
                assert_eq!(before["status"], status, "{case}: {before}");
                assert_eq!(before["result"], result, "{case}: {before}");
                assert_eq!(before["voting_open"], json!(true), "{case}: {before}");

                // At the deadline and long after it.
                for now in [deadline(), deadline() + chrono::Duration::days(3)] {
                    app.clock.set(now);
                    let after = app.voter_view(&token).await;
                    let (status, result) = match (with_response, with_fallback) {
                        (true, _) => ("answered", response_result.clone()),
                        (false, true) => ("auto_fallback", fallback_result.clone()),
                        (false, false) => ("unanswered", json!(null)),
                    };
                    assert_eq!(after["status"], status, "{case} at {now}: {after}");
                    assert_eq!(after["result"], result, "{case} at {now}: {after}");
                    assert_eq!(after["voting_open"], json!(false), "{case}: {after}");
                    let expected_response = if with_response {
                        json!([ids[0]])
                    } else {
                        json!(null)
                    };
                    assert_eq!(
                        after["response"]["option_ids"], expected_response,
                        "{case}: {after}"
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn the_voter_api_shows_the_fallback_before_the_deadline() {
    let app = TestApp::new().await;
    let (token, ids) = create(&app, "multiple", "final", Some(json!(["Thu", "Tue"]))).await;
    app.clock.set(deadline() - chrono::Duration::seconds(1));

    let view = app.voter_view(&token).await;

    assert_eq!(
        view["fallback"],
        json!({ "option_ids": [ids[1], ids[3]] }),
        "{view}"
    );
    // Shown, but not applied: no result, still Unanswered.
    assert_eq!(view["status"], "unanswered", "{view}");
    assert_eq!(view["result"], json!(null), "{view}");
    assert_eq!(view["response"], json!(null), "{view}");
}

#[tokio::test]
async fn the_voter_api_shows_a_null_fallback_when_the_ballot_has_none() {
    let app = TestApp::new().await;
    let (token, _) = create(&app, "single", "final", None).await;

    let view = app.voter_view(&token).await;

    assert_eq!(view["fallback"], json!(null), "{view}");
    assert!(view.as_object().unwrap().contains_key("fallback"), "{view}");
}

#[tokio::test]
async fn the_voting_page_shows_the_fallback_before_the_deadline() {
    let app = TestApp::new().await;
    let (token, ids) = create(&app, "multiple", "editable", Some(json!(["Wed", "Mon"]))).await;
    app.clock.set(deadline() - chrono::Duration::seconds(1));

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(
        html.contains(
            r#"<p class="fallback" id="fallback-hint">No answer by the deadline? The Options marked <span class="fallback-badge">default</span> will be recorded.</p>"#
        ),
        "{html}"
    );
    // The Fallback's Options are marked on their own cards, and their
    // inputs are described by the hint; the others are not.
    for (id, label, is_fallback) in [
        (&ids[0], "Mon", true),
        (&ids[1], "Tue", false),
        (&ids[2], "Wed", true),
        (&ids[3], "Thu", false),
    ] {
        let label_tag = format!(r#"<label for="option-{id}">{label}</label>"#);
        let at = html
            .find(&label_tag)
            .unwrap_or_else(|| panic!("{label_tag} not in {html}"));
        let card = &html[at..at + html[at..].find("</div>").unwrap()];
        assert_eq!(
            card.contains(r#"<span class="fallback-badge" title="Recorded if you don't answer by the deadline">default</span>"#),
            is_fallback,
            "{label}: {card}"
        );
        let input = format!(r#"id="option-{id}" aria-describedby="fallback-hint">"#);
        assert_eq!(html.contains(&input), is_fallback, "{label}: {html}");
    }
    assert!(html.contains("<form"), "{html}");
}

#[tokio::test]
async fn the_voting_page_mentions_no_fallback_when_the_ballot_has_none() {
    let app = TestApp::new().await;
    let (token, _) = create(&app, "single", "editable", None).await;

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(!html.contains("No answer by the deadline"), "{html}");
    assert!(!html.contains("fallback-badge"), "{html}");
}

#[tokio::test]
async fn the_voting_page_drops_the_fallback_notice_once_the_voter_answered() {
    let app = TestApp::new().await;
    let (token, ids) = create(&app, "single", "editable", Some(json!(["Wed"]))).await;
    app.post_form(&format!("/b/{token}"), &format!("option={}", ids[0]))
        .await;

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(html.contains("Your answer is saved."), "{html}");
    assert!(!html.contains("No answer by the deadline"), "{html}");
    assert!(!html.contains("fallback-badge"), "{html}");
}

#[tokio::test]
async fn the_closed_page_shows_the_applied_fallback() {
    let app = TestApp::new().await;
    let (token, _) = create(&app, "multiple", "final", Some(json!(["Mon", "Thu"]))).await;
    app.clock.set(deadline());

    let response = app.get(&format!("/b/{token}")).await;

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(
        html.contains(r#"<p class="status" data-state="closed" title="Ballot Status: Auto-fallback"><span class="visually-hidden">Status: </span>Closed</p>"#),
        "{html}"
    );
    assert!(
        html.contains(r#"<p class="closed">No answer was given, so this was recorded:</p>"#),
        "{html}"
    );
    assert!(
        html.contains("<li>Mon</li>") && html.contains("<li>Thu</li>"),
        "{html}"
    );
    assert!(
        !html.contains("<li>Tue</li>") && !html.contains("<li>Wed</li>"),
        "{html}"
    );
    assert!(!html.contains("<form"), "{html}");
}

#[tokio::test]
async fn the_closed_page_shows_the_response_not_the_fallback_when_the_voter_answered() {
    let app = TestApp::new().await;
    let (token, ids) = create(&app, "single", "final", Some(json!(["Mon"]))).await;
    app.post_form(
        &format!("/b/{token}"),
        &format!("option={}&confirm=yes", ids[1]),
    )
    .await;
    app.clock.set(deadline());

    let html = body_string(app.get(&format!("/b/{token}")).await).await;

    assert!(
        html.contains(r#"<p class="closed">You chose</p>"#),
        "{html}"
    );
    assert!(html.contains("<li>Tue</li>"), "{html}");
    assert!(!html.contains("Fallback"), "{html}");
}

#[tokio::test]
async fn a_form_response_after_the_deadline_shows_the_applied_fallback_with_409() {
    let app = TestApp::new().await;
    let (token, ids) = create(&app, "single", "editable", Some(json!(["Thu"]))).await;
    app.clock.set(deadline());

    let response = app
        .post_form(&format!("/b/{token}"), &format!("option={}", ids[0]))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let html = body_string(response).await;
    assert!(
        html.contains("Voting has closed. Your answer could not be accepted."),
        "{html}"
    );
    assert!(
        html.contains("No answer was given, so this was recorded:"),
        "{html}"
    );
    assert!(html.contains("<li>Thu</li>"), "{html}");
    assert_eq!(app.voter_view(&token).await["status"], "auto_fallback");
}

#[tokio::test]
async fn the_admin_view_of_a_new_ballot_reports_its_derived_status_and_no_result() {
    let app = TestApp::new().await;

    let created = app
        .create_ballot(&request_with_fallback("single", json!(["Wed"])))
        .await;

    assert_eq!(created["status"], "unanswered", "{created}");
    assert_eq!(created["result"], json!(null), "{created}");
    assert!(created.as_object().unwrap().contains_key("result"));
}
