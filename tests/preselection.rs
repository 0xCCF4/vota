//! The Preselection: an optional set of the Ballot's own Options, set by the
//! admin (by Option label), that the voting page shows ticked. On a Final
//! Ballot only that: nothing is recorded until the voter submits. On an
//! Editable Ballot it is recorded as the Response from the start (origin
//! `preselection`), so the Ballot is Answered and the Fallback never
//! applies; the voter can change it until the deadline, which makes the
//! Response theirs (origin `voter`).

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// The Voting Deadline of every Ballot built from `common::ballot_request`.
fn deadline() -> DateTime<Utc> {
    "2030-01-22T12:00:00Z".parse().unwrap()
}

/// A Ballot request with the Options Mon, Tue, Wed, Thu, these modes and
/// this Preselection (`null`: none).
fn request(choice_mode: &str, submission_mode: &str, preselection: Value) -> Value {
    let mut request = ballot_request();
    request["options"] = json!([
        { "label": "Mon" },
        { "label": "Tue" },
        { "label": "Wed" },
        { "label": "Thu" },
    ]);
    request["choice_mode"] = json!(choice_mode);
    request["submission_mode"] = json!(submission_mode);
    if !preselection.is_null() {
        request["preselection"] = preselection;
    }
    request
}

/// The IDs of a Ballot's Options, in order.
fn ids(ballot: &Value) -> Vec<String> {
    ballot["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap().to_owned())
        .collect()
}

fn token(ballot: &Value) -> String {
    ballot["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn on_a_final_ballot_the_preselection_is_shown_but_nothing_is_recorded() {
    let app = TestApp::new().await;

    let created = app
        .create_ballot(&request("multiple", "final", json!(["Thu", "Tue"])))
        .await;

    let ids = ids(&created);
    assert_eq!(
        created["preselection"],
        json!([
            { "id": ids[1], "label": "Tue" },
            { "id": ids[3], "label": "Thu" },
        ]),
        "{created}"
    );
    assert_eq!(created["status"], "unanswered", "{created}");
    assert_eq!(created["response"], json!(null), "{created}");
    assert_eq!(created["result"], json!(null), "{created}");
    let view = app.voter_view(&token(&created)).await;
    assert_eq!(
        view["preselection"],
        json!({ "option_ids": [ids[1], ids[3]] }),
        "{view}"
    );
    assert_eq!(view["status"], "unanswered", "{view}");
    assert_eq!(view["response"], json!(null), "{view}");
}

#[tokio::test]
async fn an_editable_ballot_starts_answered_with_its_preselection_as_the_response() {
    let app = TestApp::new().await;

    let created = app
        .create_ballot(&request("single", "editable", json!(["Wed"])))
        .await;

    let ids = ids(&created);
    assert_eq!(created["status"], "answered", "{created}");
    assert_eq!(
        created["response"]["option_ids"],
        json!([ids[2]]),
        "{created}"
    );
    assert_eq!(created["response"]["origin"], "preselection", "{created}");
    assert_eq!(
        created["result"],
        json!({
            "source": "preselection",
            "option_ids": [ids[2]],
            "options": [{ "id": ids[2], "label": "Wed" }],
        }),
        "{created}"
    );
    let view = app.voter_view(&token(&created)).await;
    assert_eq!(view["status"], "answered", "{view}");
    assert_eq!(view["response"]["origin"], "preselection", "{view}");
    assert_eq!(
        view["result"],
        json!({ "source": "preselection", "option_ids": [ids[2]] }),
        "{view}"
    );
}

#[tokio::test]
async fn a_voters_response_has_the_origin_voter() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(null)))
        .await;
    let ids = ids(&created);

    let response = app
        .put_json(
            &format!("/api/b/{}/response", token(&created)),
            &json!({ "option_ids": [ids[0]] }),
        )
        .await;

    let view = body_json(response).await;
    assert_eq!(view["response"]["origin"], "voter", "{view}");
    assert_eq!(view["result"]["source"], "response", "{view}");
    assert_eq!(view["preselection"], json!(null), "{view}");
}

#[tokio::test]
async fn the_voter_saving_the_very_preselected_options_makes_the_response_theirs() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("multiple", "editable", json!(["Mon", "Thu"])))
        .await;
    let ids = ids(&created);
    let later = common::start_time() + chrono::Duration::hours(1);
    app.clock.set(later);

    let response = app
        .put_json(
            &format!("/api/b/{}/response", token(&created)),
            &json!({ "option_ids": [ids[0], ids[3]] }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["response"]["origin"], "voter", "{view}");
    assert_eq!(view["response"]["option_ids"], json!([ids[0], ids[3]]));
    assert_eq!(view["result"]["source"], "response", "{view}");
    // Answered when the voter first saved, not when the admin preselected.
    let at = "2030-01-15T13:00:00Z";
    assert_eq!(later.to_rfc3339_opts(chrono::SecondsFormat::Secs, true), at);
    assert_eq!(view["response"]["answered_at"], at, "{view}");
    assert_eq!(view["response"]["changed_at"], at, "{view}");
}

#[tokio::test]
async fn the_fallback_never_applies_to_an_editable_ballot_with_a_preselection() {
    let app = TestApp::new().await;
    let mut request = request("single", "editable", json!(["Mon"]));
    request["fallback"] = json!(["Tue"]);
    let created = app.create_ballot(&request).await;
    let ids = ids(&created);
    app.clock.set(deadline());

    let view = app.voter_view(&token(&created)).await;

    assert_eq!(view["status"], "answered", "{view}");
    assert_eq!(
        view["result"],
        json!({ "source": "preselection", "option_ids": [ids[0]] }),
        "{view}"
    );
    assert_eq!(
        view["fallback"],
        json!({ "option_ids": [ids[1]] }),
        "{view}"
    );
}

#[tokio::test]
async fn on_a_final_ballot_the_fallback_still_applies_without_a_response() {
    let app = TestApp::new().await;
    let mut request = request("single", "final", json!(["Mon"]));
    request["fallback"] = json!(["Tue"]);
    let created = app.create_ballot(&request).await;
    let ids = ids(&created);
    app.clock.set(deadline());

    let view = app.voter_view(&token(&created)).await;

    assert_eq!(view["status"], "auto_fallback", "{view}");
    assert_eq!(
        view["result"],
        json!({ "source": "fallback", "option_ids": [ids[1]] }),
        "{view}"
    );
}

/// PATCHes the Ballot with this body; the status and the answer JSON.
async fn patch(app: &TestApp, ballot: &Value, body: Value) -> (StatusCode, Value) {
    let admin = app.admin_token().await;
    let response = app
        .admin_patch(&format!("/admin/ballots/{}", ballot["id"]), &admin, &body)
        .await;
    (response.status(), body_json(response).await)
}

#[tokio::test]
async fn the_admin_can_edit_while_the_only_response_is_the_preselection() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(["Mon"])))
        .await;
    let ids = ids(&created);
    let later = common::start_time() + chrono::Duration::hours(2);
    app.clock.set(later);

    let (status, edited) = patch(&app, &created, json!({ "title": "Which day?" })).await;

    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["title"], "Which day?");
    assert_eq!(edited["status"], "answered", "{edited}");
    assert_eq!(edited["response"]["origin"], "preselection", "{edited}");
    assert_eq!(
        edited["response"]["option_ids"],
        json!([ids[0]]),
        "{edited}"
    );
    // Recorded anew by the edit.
    assert_eq!(edited["response"]["answered_at"], "2030-01-15T14:00:00Z");
}

#[tokio::test]
async fn an_admin_edit_of_the_preselection_replaces_the_preselected_response() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("multiple", "editable", json!(["Mon"])))
        .await;
    let ids = ids(&created);

    let (status, edited) = patch(&app, &created, json!({ "preselection": ["Wed", "Tue"] })).await;

    assert_eq!(status, StatusCode::OK, "{edited}");
    let both = json!([
        { "id": ids[1], "label": "Tue" },
        { "id": ids[2], "label": "Wed" },
    ]);
    assert_eq!(edited["preselection"], both, "{edited}");
    assert_eq!(edited["response"]["option_ids"], json!([ids[1], ids[2]]));
    assert_eq!(edited["response"]["origin"], "preselection", "{edited}");
}

#[tokio::test]
async fn new_options_need_the_preselection_given_again() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(["Mon"])))
        .await;
    let options = json!([{ "label": "Fri" }, { "label": "Sat" }]);

    let (status, error) = patch(&app, &created, json!({ "options": options })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["field"], "preselection", "{error}");

    let body = json!({ "options": options, "preselection": ["Sat"] });
    let (status, edited) = patch(&app, &created, body).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    let ids = ids(&edited);
    assert_eq!(
        edited["preselection"],
        json!([{ "id": ids[1], "label": "Sat" }])
    );
    assert_eq!(
        edited["response"]["option_ids"],
        json!([ids[1]]),
        "{edited}"
    );
}

#[tokio::test]
async fn removing_the_preselection_of_an_editable_ballot_leaves_it_unanswered() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(["Mon"])))
        .await;

    let (status, edited) = patch(&app, &created, json!({ "preselection": null })).await;

    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["preselection"], json!(null), "{edited}");
    assert_eq!(edited["response"], json!(null), "{edited}");
    assert_eq!(edited["status"], "unanswered", "{edited}");
}

#[tokio::test]
async fn switching_the_submission_mode_records_or_drops_the_preselected_response() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(["Tue"])))
        .await;
    let ids = ids(&created);

    let (status, final_) = patch(&app, &created, json!({ "submission_mode": "final" })).await;
    assert_eq!(status, StatusCode::OK, "{final_}");
    assert_eq!(final_["status"], "unanswered", "{final_}");
    assert_eq!(final_["response"], json!(null), "{final_}");
    assert_eq!(
        final_["preselection"],
        json!([{ "id": ids[1], "label": "Tue" }])
    );

    let (status, editable) = patch(&app, &created, json!({ "submission_mode": "editable" })).await;
    assert_eq!(status, StatusCode::OK, "{editable}");
    assert_eq!(editable["status"], "answered", "{editable}");
    assert_eq!(editable["response"]["origin"], "preselection", "{editable}");
}

#[tokio::test]
async fn once_the_voter_saved_anything_admin_edits_are_refused() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(["Mon"])))
        .await;
    let ids = ids(&created);
    let saved = app
        .put_json(
            &format!("/api/b/{}/response", token(&created)),
            &json!({ "option_ids": [ids[0]] }),
        )
        .await;
    assert_eq!(saved.status(), StatusCode::OK);

    let (status, error) = patch(&app, &created, json!({ "title": "Which day?" })).await;

    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["error"], "already_answered", "{error}");
    // The admin's bookkeeping may still change.
    let (status, edited) = patch(&app, &created, json!({ "group": "Team" })).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["response"]["origin"], "voter", "{edited}");
}

#[tokio::test]
async fn once_voting_closed_on_the_preselection_admin_edits_are_refused_until_extended() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(["Mon"])))
        .await;
    app.clock.set(deadline());

    let (status, error) = patch(&app, &created, json!({ "title": "Which day?" })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["error"], "already_answered", "{error}");

    let admin = app.admin_token().await;
    let extended = app
        .post_json(
            &format!("/admin/ballots/{}/extend", created["id"]),
            Some(&admin),
            &json!({ "voting_deadline": "2030-01-29T12:00:00Z" }),
        )
        .await;
    assert_eq!(extended.status(), StatusCode::OK);
    let (status, edited) = patch(&app, &created, json!({ "title": "Which day?" })).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["response"]["origin"], "preselection", "{edited}");
}

#[tokio::test]
async fn an_invalid_preselection_is_rejected_naming_the_field() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut no_option = request("multiple", "editable", json!([]));
    no_option["allow_no_option"] = json!(true);
    let cases = [
        (request("multiple", "final", json!([])), "preselection"),
        (no_option, "preselection"),
        (
            request("single", "final", json!(["Mon", "Tue"])),
            "preselection",
        ),
        (
            request("single", "final", json!(["Fri"])),
            "preselection[0]",
        ),
        (
            request("multiple", "final", json!(["Mon", "Mon"])),
            "preselection[1]",
        ),
        (request("multiple", "final", json!("Mon")), "preselection"),
    ];
    for (body, field) in cases {
        let response = app.post_json("/admin/ballots", Some(&admin), &body).await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
        let error = body_json(response).await;
        if error["field"].is_string() {
            assert_eq!(error["field"], field, "{body}: {error}");
        }
    }
    let mut null = ballot_request();
    null["preselection"] = json!(null);
    let response = app.post_json("/admin/ballots", Some(&admin), &null).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn bulk_creation_preselects_on_every_ballot() {
    let app = TestApp::new().await;
    let mut listed = request("single", "editable", json!(["Thu"]));
    listed["group"] = json!("Team");
    listed["voter_reference"] = json!(["ann", "bob"]);
    let mut counted = request("single", "final", json!(["Mon"]));
    counted["group"] = json!("Club");
    counted["count"] = json!(2);

    for (body, origin) in [(listed, Some("preselection")), (counted, None)] {
        let created = app.create_ballot(&body).await;
        let ballots = created["ballots"].as_array().unwrap();
        assert_eq!(ballots.len(), 2, "{created}");
        for ballot in ballots {
            let ids = ids(ballot);
            let label = body["preselection"][0].as_str().unwrap();
            let id = &ids[["Mon", "Tue", "Wed", "Thu"]
                .iter()
                .position(|l| *l == label)
                .unwrap()];
            assert_eq!(
                ballot["preselection"],
                json!([{ "id": id, "label": label }])
            );
            match origin {
                Some(origin) => {
                    assert_eq!(ballot["response"]["origin"], origin, "{ballot}");
                    assert_eq!(ballot["response"]["option_ids"], json!([id]), "{ballot}");
                }
                None => assert_eq!(ballot["response"], json!(null), "{ballot}"),
            }
        }
    }
}

/// The voting page of this Ballot.
async fn page(app: &TestApp, ballot: &Value) -> String {
    common::body_string(app.get(&format!("/b/{}", token(ballot))).await).await
}

/// The start tag of the input of this Option on a voting page.
fn input<'a>(html: &'a str, id: &str) -> &'a str {
    let at = html
        .find(&format!(r#"value="{id}""#))
        .unwrap_or_else(|| panic!("no input of {id} in {html}"));
    let start = html[..at].rfind('<').unwrap();
    let end = at + html[at..].find('>').unwrap();
    &html[start..=end]
}

#[tokio::test]
async fn the_final_voting_page_ticks_the_preselection_and_says_to_confirm_or_change_it() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("multiple", "final", json!(["Tue", "Thu"])))
        .await;
    let ids = ids(&created);

    let html = page(&app, &created).await;

    assert!(input(&html, &ids[1]).contains(" checked"), "{html}");
    assert!(input(&html, &ids[3]).contains(" checked"), "{html}");
    assert!(!input(&html, &ids[0]).contains(" checked"), "{html}");
    assert!(
        html.contains("Preselected; confirm or change, then submit."),
        "{html}"
    );
    assert!(html.contains(">Continue</button>"), "{html}");

    // Posting it is the confirmation step as ever: nothing is recorded yet.
    let body = format!("option={}&option={}", ids[1], ids[3]);
    let step = app
        .post_form(&format!("/b/{}", token(&created)), &body)
        .await;
    assert_eq!(step.status(), StatusCode::OK);
    let step = common::body_string(step).await;
    assert!(step.contains(r#"name="confirm" value="yes""#), "{step}");
    let view = app.voter_view(&token(&created)).await;
    assert_eq!(view["response"], json!(null), "{view}");

    let confirmed = app
        .post_form(
            &format!("/b/{}", token(&created)),
            &format!("{body}&confirm=yes"),
        )
        .await;
    assert_eq!(confirmed.status(), StatusCode::SEE_OTHER);
    let view = app.voter_view(&token(&created)).await;
    assert_eq!(view["response"]["origin"], "voter", "{view}");
    let html = page(&app, &created).await;
    assert!(!html.contains("Preselected"), "{html}");
}

#[tokio::test]
async fn the_editable_voting_page_shows_the_preselection_as_saved_until_the_voter_saves() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(["Wed"])))
        .await;
    let ids = ids(&created);

    let html = page(&app, &created).await;

    assert!(input(&html, &ids[2]).contains(" checked"), "{html}");
    assert!(
        html.contains("Preselected for you. You can change it until voting closes."),
        "{html}"
    );
    assert!(!html.contains("Your answer is saved"), "{html}");

    // Saving the very same Option without JavaScript makes it the voter's.
    let saved = app
        .post_form(
            &format!("/b/{}", token(&created)),
            &format!("option={}", ids[2]),
        )
        .await;
    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    let html = page(&app, &created).await;
    assert!(input(&html, &ids[2]).contains(" checked"), "{html}");
    assert!(!html.contains("Preselected"), "{html}");
    assert!(html.contains("Your answer is saved"), "{html}");
}

#[tokio::test]
async fn once_closed_the_page_says_the_preselection_was_recorded() {
    let app = TestApp::new().await;
    let created = app
        .create_ballot(&request("single", "editable", json!(["Wed"])))
        .await;
    app.clock.set(deadline());

    let html = page(&app, &created).await;

    assert!(
        html.contains("You kept the preselection, so this was recorded:"),
        "{html}"
    );
    assert!(html.contains("<li>Wed</li>"), "{html}");
}
