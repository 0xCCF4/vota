//! Admin dates without a UTC offset (`2030-01-22T12:00:00`, or without
//! seconds) are local times in the server's `default_time_zone`, converted to
//! UTC; one with an offset is read as before. A local time that a clock
//! change skips or repeats is refused. Answers stay in UTC.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

// The Clock starts at 2030-01-15T12:00Z. In Europe/Berlin, 2030's clocks go
// forward at 2030-03-31T02:00 (to 03:00) and back at 2030-10-27T03:00 (to
// 02:00).

async fn berlin() -> TestApp {
    TestApp::with_config(|config| config.default_time_zone = chrono_tz::Europe::Berlin).await
}

/// Creates a Ballot whose Voting Deadline is `deadline`; returns the response.
async fn create_with_deadline(app: &TestApp, deadline: &str) -> axum::response::Response {
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["voting_deadline"] = json!(deadline);
    app.post_json("/admin/ballots", Some(&admin), &request)
        .await
}

/// The Voting Deadline a Ballot created with `deadline` gets.
async fn created_deadline(app: &TestApp, deadline: &str) -> Value {
    let response = create_with_deadline(app, deadline).await;
    assert_eq!(response.status(), StatusCode::CREATED, "{deadline}");
    body_json(response).await["voting_deadline"].clone()
}

/// The 422 body for a Ballot created with `deadline`.
async fn refused_deadline(app: &TestApp, deadline: &str) -> Value {
    let response = create_with_deadline(app, deadline).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "{deadline}"
    );
    body_json(response).await
}

#[tokio::test]
async fn a_local_deadline_is_read_in_the_default_time_zone_and_answered_in_utc() {
    let app = berlin().await;

    // Winter (CET, +01:00) and summer (CEST, +02:00).
    for (local, utc) in [
        ("2030-01-22T12:00:00", "2030-01-22T11:00:00Z"),
        ("2030-07-01T12:00:00", "2030-07-01T10:00:00Z"),
        ("2030-01-22T00:30:00", "2030-01-21T23:30:00Z"),
    ] {
        assert_eq!(created_deadline(&app, local).await, utc, "{local}");
    }
}

#[tokio::test]
async fn the_minutes_only_form_and_fractional_seconds_are_accepted() {
    let app = berlin().await;

    for (local, utc) in [
        ("2030-01-22T12:00", "2030-01-22T11:00:00Z"),
        ("2030-07-01T09:45", "2030-07-01T07:45:00Z"),
        ("2030-01-22T12:00:00.5", "2030-01-22T11:00:00.500Z"),
        ("2030-01-22T12:00:00.123456789", "2030-01-22T11:00:00.123Z"),
    ] {
        assert_eq!(created_deadline(&app, local).await, utc, "{local}");
    }
}

#[tokio::test]
async fn under_the_default_utc_zone_a_local_deadline_is_utc() {
    let app = TestApp::new().await;

    assert_eq!(
        created_deadline(&app, "2030-01-22T12:00:00").await,
        "2030-01-22T12:00:00Z"
    );
    assert_eq!(
        created_deadline(&app, "2030-07-01T09:45").await,
        "2030-07-01T09:45:00Z"
    );
}

#[tokio::test]
async fn an_offset_still_wins_over_the_default_time_zone() {
    let app = berlin().await;

    for (given, utc) in [
        ("2030-01-22T12:00:00Z", "2030-01-22T12:00:00Z"),
        ("2030-01-22T12:00:00+05:00", "2030-01-22T07:00:00Z"),
        ("2030-07-01T12:00:00-03:30", "2030-07-01T15:30:00Z"),
        // The gap and the repeated hour don't matter with an offset.
        ("2030-03-31T02:30:00+01:00", "2030-03-31T01:30:00Z"),
        ("2030-10-27T02:30:00+02:00", "2030-10-27T00:30:00Z"),
    ] {
        assert_eq!(created_deadline(&app, given).await, utc, "{given}");
    }
}

#[tokio::test]
async fn a_local_time_skipped_by_a_clock_change_is_refused() {
    let app = berlin().await;

    for skipped in [
        "2030-03-31T02:30:00",
        "2030-03-31T02:00",
        "2030-03-31T02:59:59",
    ] {
        let body = refused_deadline(&app, skipped).await;

        assert_eq!(body["error"], "invalid_input", "{skipped}");
        assert_eq!(body["field"], "voting_deadline", "{skipped}");
        assert_eq!(
            body["message"],
            "does not exist in the server's default time zone (skipped by a clock change); \
             give a UTC offset",
            "{skipped}"
        );
    }
    // The minutes around it are fine.
    assert_eq!(
        created_deadline(&app, "2030-03-31T01:59").await,
        "2030-03-31T00:59:00Z"
    );
    assert_eq!(
        created_deadline(&app, "2030-03-31T03:00").await,
        "2030-03-31T01:00:00Z"
    );
}

#[tokio::test]
async fn a_local_time_repeated_by_a_clock_change_is_refused_as_ambiguous() {
    let app = berlin().await;

    for repeated in [
        "2030-10-27T02:30:00",
        "2030-10-27T02:00",
        "2030-10-27T02:59:59.9",
    ] {
        let body = refused_deadline(&app, repeated).await;

        assert_eq!(body["field"], "voting_deadline", "{repeated}");
        assert_eq!(
            body["message"],
            "is ambiguous in the server's default time zone (repeated by a clock change); \
             give a UTC offset",
            "{repeated}"
        );
    }
    assert_eq!(
        created_deadline(&app, "2030-10-27T03:00").await,
        "2030-10-27T02:00:00Z"
    );
}

#[tokio::test]
async fn other_forms_are_refused_with_the_new_message() {
    let app = berlin().await;

    for malformed in [
        "2030-01-22",
        "2030-01-22T12",
        "2030-01-22 12:00:00",
        "2030-01-22T12:00:",
        "2030-01-22T12:00:00.",
        "2030-1-22T12:00:00",
        "2030-01-22T2:00",
        "+2030-01-22T12:00",
        "2030-01-22T12:00:00 ",
        " 2030-01-22T12:00",
        "2030-01-22T12:00:00CET",
        "2030-01-22T12:00:00+01",
        "2030-02-30T12:00",
        "2030-01-22T24:00",
        "2030-01-22T12:60",
        "2030-01-22T12:00:00:00",
        "2030-01-22T12:00Z",
        "２０３０-01-22T12:00",
        "tomorrow",
        "",
    ] {
        let body = refused_deadline(&app, malformed).await;

        assert_eq!(body["field"], "voting_deadline", "{malformed:?}");
        assert_eq!(
            body["message"],
            "not an RFC 3339 timestamp, or a local date-time in the server's default time zone \
             (YYYY-MM-DDTHH:MM[:SS])",
            "{malformed:?}"
        );
    }
}

#[tokio::test]
async fn the_horizon_and_future_checks_apply_after_the_conversion() {
    let app = berlin().await;

    // 12:30 in Berlin is 11:30Z, before the Clock's 12:00Z.
    let body = refused_deadline(&app, "2030-01-15T12:30").await;
    assert_eq!(body["message"], "must lie in the future");
    assert_eq!(
        created_deadline(&app, "2030-01-15T13:30").await,
        "2030-01-15T12:30:00Z"
    );

    // The horizon (1 year: 2031-01-15T18:00Z) after conversion.
    assert_eq!(
        created_deadline(&app, "2031-01-15T19:00").await,
        "2031-01-15T18:00:00Z"
    );
    let body = refused_deadline(&app, "2031-01-15T19:01").await;
    assert_eq!(
        body["message"],
        "must not lie further ahead than max_date_horizon allows"
    );
}

#[tokio::test]
async fn a_local_purge_date_is_converted_and_checked_against_the_deadline() {
    let app = berlin().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["voting_deadline"] = json!("2030-01-22T12:00");
    request["purge_date"] = json!("2030-02-01T08:00");

    let response = app
        .post_json("/admin/ballots", Some(&admin), &request)
        .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let created = body_json(response).await;
    assert_eq!(created["purge_date"], "2030-02-01T07:00:00Z");

    // 11:30 local is before the deadline's 12:00 local (both CET).
    request["purge_date"] = json!("2030-01-22T11:30");
    let response = app
        .post_json("/admin/ballots", Some(&admin), &request)
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(response).await;
    assert_eq!(body["field"], "purge_date");
    assert_eq!(body["message"], "must not be before the Voting Deadline");

    request["purge_date"] = json!("2030-10-27T02:15");
    let response = app
        .post_json("/admin/ballots", Some(&admin), &request)
        .await;
    let body = body_json(response).await;
    assert_eq!(body["field"], "purge_date");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("is ambiguous"),
        "{body}"
    );
}

#[tokio::test]
async fn an_edit_reads_local_dates_in_the_default_time_zone() {
    let app = berlin().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    let path = format!("/admin/ballots/{}", created["id"]);

    let response = app
        .admin_patch(
            &path,
            &admin,
            &json!({ "voting_deadline": "2030-07-01T12:00", "purge_date": "2030-08-01T12:00:00" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let edited = body_json(response).await;
    assert_eq!(edited["voting_deadline"], "2030-07-01T10:00:00Z");
    assert_eq!(edited["purge_date"], "2030-08-01T10:00:00Z");

    let response = app
        .admin_patch(
            &path,
            &admin,
            &json!({ "voting_deadline": "2030-03-31T02:30" }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(response).await;
    assert_eq!(body["field"], "voting_deadline");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("does not exist"),
        "{body}"
    );
}

#[tokio::test]
async fn an_extension_reads_a_local_deadline_in_the_default_time_zone() {
    let app = berlin().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    let path = format!("/admin/ballots/{}/extend", created["id"]);

    let response = app
        .post_json(
            &path,
            Some(&admin),
            &json!({ "voting_deadline": "2030-02-01T12:00" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await["voting_deadline"],
        "2030-02-01T11:00:00Z"
    );

    let response = app
        .post_json(
            &path,
            Some(&admin),
            &json!({ "voting_deadline": "2030-10-27T02:30:00" }),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = body_json(response).await;
    assert_eq!(body["field"], "voting_deadline");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("is ambiguous"),
        "{body}"
    );
}
