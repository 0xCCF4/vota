//! A deleted or purged Ballot leaves no readable trace in the database
//! files: every connection overwrites deleted content (`secure_delete`), and
//! after an admin DELETE and after a sweep that deleted Ballots the
//! write-ahead log is checkpointed and truncated, so the old page images in
//! it are gone too. (Reading the files' bytes, as someone with a copy of
//! them would; backups and filesystem snapshots taken before are another
//! matter.)

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request};
use serde_json::json;
use vota::jobs::purge_sweep;

const TITLE: &str = "SWEEPTITLE_QQQ";
const REFERENCE: &str = "SWEEPREF_QQQ";

/// Creates an Editable Ballot with marked title and Voter Reference that
/// is purged on 2030-02-01, answers it, and returns its ID and Ballot Token.
async fn answered_ballot(app: &TestApp) -> (i64, String) {
    let mut request = ballot_request();
    request["title"] = json!(TITLE);
    request["group"] = json!("survey");
    request["voter_reference"] = json!(REFERENCE);
    request["submission_mode"] = json!("editable");
    request["purge_date"] = json!("2030-02-01T00:00:00Z");
    let created = app.create_ballot(&request).await;
    let token = created["token"].as_str().unwrap().to_owned();
    let pizza = created["options"][0]["id"].as_str().unwrap();
    let answered = app
        .put_json(
            &format!("/api/b/{token}/response"),
            &json!({ "option_ids": [pizza] }),
        )
        .await;
    assert_eq!(answered.status(), StatusCode::OK);
    (created["id"].as_i64().unwrap(), token)
}

/// Which of `needles` appear anywhere in the database files.
fn traces(app: &TestApp, needles: &[&str]) -> Vec<String> {
    let files: Vec<Vec<u8>> = app
        .database_files()
        .iter()
        .filter_map(|file| std::fs::read(file).ok())
        .collect();
    needles
        .iter()
        .filter(|needle| {
            files
                .iter()
                .any(|bytes| bytes.windows(needle.len()).any(|w| w == needle.as_bytes()))
        })
        .map(|needle| needle.to_string())
        .collect()
}

#[tokio::test]
async fn an_admin_delete_leaves_no_trace_in_the_database_files() {
    let app = TestApp::new().await;
    let (id, token) = answered_ballot(&app).await;
    assert!(!traces(&app, &[TITLE, REFERENCE, &token]).is_empty());

    let admin = app.admin_token().await;
    let deleted = app
        .admin_delete(&format!("/admin/ballots/{id}"), &admin)
        .await;

    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        traces(&app, &[TITLE, REFERENCE, &token]),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_sweep_leaves_no_trace_of_purged_ballots_in_the_database_files() {
    let app = TestApp::new().await;
    let (_, token) = answered_ballot(&app).await;
    // Another Ballot stays, with its own data.
    app.create_ballot(&ballot_request()).await;

    app.clock.set("2030-02-01T00:00:00Z".parse().unwrap());
    let deleted = purge_sweep::run_once(&app.state).await.unwrap();

    assert_eq!(deleted, 1);
    assert_eq!(
        traces(&app, &[TITLE, REFERENCE, &token]),
        Vec::<String>::new()
    );
    assert!(!traces(&app, &["Lunch on Friday?"]).is_empty());
}
