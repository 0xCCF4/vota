//! Choice Mode and Submission Mode: set by the admin at creation, with
//! defaults, and visible to the voter.

#![cfg(feature = "server")]

mod common;

use common::{TestApp, ballot_request, body_string};
use serde_json::json;

#[tokio::test]
async fn a_ballot_is_single_choice_and_final_by_default() {
    let app = TestApp::new().await;

    let created = app.create_ballot(&ballot_request()).await;

    assert_eq!(created["choice_mode"], "single", "{created}");
    assert_eq!(created["submission_mode"], "final", "{created}");
}

#[tokio::test]
async fn admin_sets_both_modes_and_the_voter_api_shows_them() {
    let app = TestApp::new().await;
    let mut request = ballot_request();
    request["choice_mode"] = json!("multiple");
    request["submission_mode"] = json!("editable");

    let created = app.create_ballot(&request).await;

    assert_eq!(created["choice_mode"], "multiple", "{created}");
    assert_eq!(created["submission_mode"], "editable", "{created}");
    let view = app.voter_view(created["token"].as_str().unwrap()).await;
    assert_eq!(view["choice_mode"], "multiple", "{view}");
    assert_eq!(view["submission_mode"], "editable", "{view}");
}

#[tokio::test]
async fn the_voting_page_tells_the_voter_whether_they_can_change_their_answer() {
    let app = TestApp::new().await;
    for (mode, notice, button) in [
        (
            "final",
            r#"<p class="submission-mode final">Once you submit your answer, you cannot change it.</p>"#,
            "Continue",
        ),
        (
            "editable",
            r#"<p class="submission-mode">You can change your answer until then.</p>"#,
            "Save answer",
        ),
    ] {
        let (token, _) = app.create_ballot_with_modes("single", mode).await;

        let html = body_string(app.get(&format!("/b/{token}")).await).await;

        assert!(html.contains(notice), "{mode}: {html}");
        let button = format!(r#"<button type="submit">{button}</button>"#);
        assert!(html.contains(&button), "{mode}: {html}");
    }
}
