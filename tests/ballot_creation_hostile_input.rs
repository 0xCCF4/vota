//! Hostile and invalid input to `POST /admin/ballots`, through the HTTP seam.
//! Every rejection is a 422 naming the offending field.

#![cfg(feature = "server")]

mod common;

use axum::http::StatusCode;
use common::{TestApp, ballot_request, body_json};
use serde_json::{Value, json};

/// Sends `request` and asserts a 422 naming `field`.
async fn assert_rejected(app: &TestApp, request: &Value, field: &str) {
    let token = app.admin_token().await;
    let response = app.post_json("/admin/ballots", Some(&token), request).await;
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {request}"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"], "invalid_input", "{body}");
    assert_eq!(body["field"], field, "for {request}: {body}");
}

fn with(field: &str, value: Value) -> Value {
    let mut request = ballot_request();
    request[field] = value;
    request
}

fn with_labels(labels: &[&str]) -> Value {
    let options: Vec<_> = labels.iter().map(|l| json!({ "label": l })).collect();
    with("options", json!(options))
}

/// Text that every free-text field must refuse.
fn hostile_texts() -> Vec<String> {
    vec![
        String::new(),
        "   \t ".into(),
        "x".repeat(201),
        "bell\u{7}".into(),
        "nul\u{0}byte".into(),
        "line\nbreak".into(),
        "escape\u{1b}[31m".into(),
        "del\u{7f}".into(),
        "c1\u{85}control".into(),
        "bidi\u{202E}override".into(),
        "isolate\u{2066}x\u{2069}".into(),
        "mark\u{200F}".into(),
        "zero\u{200B}width".into(),
        "joiner\u{200D}".into(),
        "bom\u{FEFF}".into(),
        "line\u{2028}separator".into(),
        "tag\u{E0041}".into(),
    ]
}

// --- title ---------------------------------------------------------------

#[tokio::test]
async fn hostile_titles_are_rejected() {
    let app = TestApp::new().await;
    for text in hostile_texts() {
        assert_rejected(&app, &with("title", json!(text)), "title").await;
    }
}

#[tokio::test]
async fn title_of_wrong_type_or_missing_is_rejected() {
    let app = TestApp::new().await;
    assert_rejected(&app, &with("title", json!(42)), "title").await;
    assert_rejected(&app, &with("title", json!(null)), "title").await;
    assert_rejected(&app, &with("title", json!(["a"])), "title").await;
    let mut request = ballot_request();
    request.as_object_mut().unwrap().remove("title");
    assert_rejected(&app, &request, "title").await;
}

#[tokio::test]
async fn title_is_trimmed_and_nfc_normalised() {
    let app = TestApp::new().await;

    // "Cafe" + combining acute accent, padded with whitespace.
    let body = app
        .create_ballot(&with("title", json!("  Cafe\u{301}?\t")))
        .await;

    assert_eq!(body["title"], "Caf\u{e9}?");
}

#[tokio::test]
async fn title_length_is_counted_in_characters_after_normalisation() {
    let app = TestApp::new().await;
    // 200 x "e + combining acute": 400 code points, 200 characters after NFC.
    let decomposed = "e\u{301}".repeat(200);

    let body = app.create_ballot(&with("title", json!(decomposed))).await;

    assert_eq!(body["title"].as_str().unwrap().chars().count(), 200);
    // Multi-byte characters count once each: 200 is the limit, not bytes.
    app.create_ballot(&with("title", json!("\u{1F5F3}".repeat(200))))
        .await;
    assert_rejected(
        &app,
        &with("title", json!("\u{1F5F3}".repeat(201))),
        "title",
    )
    .await;
}

// --- options -------------------------------------------------------------

#[tokio::test]
async fn fewer_than_two_options_are_rejected() {
    let app = TestApp::new().await;
    assert_rejected(&app, &with_labels(&[]), "options").await;
    assert_rejected(&app, &with_labels(&["Only one"]), "options").await;
    assert_rejected(&app, &with("options", json!(null)), "options").await;
    assert_rejected(&app, &with("options", json!("Pizza, Sushi")), "options").await;
}

#[tokio::test]
async fn duplicate_option_labels_are_rejected_naming_the_duplicate() {
    let app = TestApp::new().await;
    assert_rejected(
        &app,
        &with_labels(&["Pizza", "Sushi", "Pizza"]),
        "options[2].label",
    )
    .await;
    // Duplicates after trimming and NFC normalisation count too.
    assert_rejected(
        &app,
        &with_labels(&["Caf\u{e9}", " Cafe\u{301} "]),
        "options[1].label",
    )
    .await;
}

#[tokio::test]
async fn hostile_option_labels_are_rejected_naming_the_option() {
    let app = TestApp::new().await;
    for text in hostile_texts() {
        assert_rejected(&app, &with_labels(&["Pizza", &text]), "options[1].label").await;
    }
}

#[tokio::test]
async fn malformed_options_are_rejected_naming_the_option() {
    let app = TestApp::new().await;
    // Option IDs are server-generated; an admin can't choose them. (Not a
    // field of an Option in a request: reported unnamed, as unknown.)
    let options = json!([{ "label": "Pizza" }, { "label": "Sushi", "id": "chosen-by-admin" }]);
    let token = app.admin_token().await;
    let response = app
        .post_json("/admin/ballots", Some(&token), &with("options", options))
        .await;
    assert_eq!(
        body_json(response).await,
        json!({ "error": "invalid_input", "field": null, "message": "unknown field" })
    );
    let options = json!([{ "label": "Pizza" }, "Sushi"]);
    assert_rejected(&app, &with("options", options), "options[1]").await;
    let options = json!([{ "label": "Pizza" }, {}]);
    assert_rejected(&app, &with("options", options), "options[1].label").await;
}

// --- voting_deadline -----------------------------------------------------

#[tokio::test]
async fn voting_deadline_must_be_rfc3339_with_an_explicit_offset() {
    let app = TestApp::new().await;
    for value in [
        json!("2030-01-22T12:00:00"),
        json!("2030-01-22 12:00"),
        json!("2030-01-22"),
        json!("22.01.2030 12:00"),
        json!("next friday"),
        json!("2030-02-30T12:00:00Z"),
        json!("2030-01-22T25:00:00Z"),
        json!(" 2030-01-22T12:00:00Z"),
        json!("2030-01-22T12:00:00Z\u{202E}"),
        json!("../../etc/passwd"),
        json!(""),
        json!(1_900_000_000),
        json!(null),
    ] {
        assert_rejected(&app, &with("voting_deadline", value), "voting_deadline").await;
    }
}

#[tokio::test]
async fn voting_deadline_must_lie_in_the_future() {
    let app = TestApp::new().await;
    // The Clock stands at 2030-01-15T12:00:00Z.
    for past in [
        "2030-01-15T12:00:00Z",
        "2030-01-15T13:00:00+01:00",
        "2030-01-15T11:59:59Z",
        "1999-12-31T23:59:59Z",
    ] {
        assert_rejected(
            &app,
            &with("voting_deadline", json!(past)),
            "voting_deadline",
        )
        .await;
    }
    app.create_ballot(&with("voting_deadline", json!("2030-01-15T12:00:01Z")))
        .await;
}

// --- purge_date ----------------------------------------------------------

#[tokio::test]
async fn purge_date_must_be_rfc3339_with_an_explicit_offset() {
    let app = TestApp::new().await;
    for value in [
        json!("2030-03-01T12:00:00"),
        json!("2030-03-01"),
        json!("2030-02-30T12:00:00Z"),
        json!("2030-03-01T25:00:00Z"),
        json!(" 2030-03-01T12:00:00Z"),
        json!("2030-03-01T12:00:00Z\u{202E}"),
        json!("2030-03-01T12:00:00Z\u{0}"),
        json!("01.03.2030 12:00"),
        json!("in four weeks"),
        json!("../../etc/passwd"),
        json!("x".repeat(10_000)),
        json!(""),
        json!(1_900_000_000),
        json!(["2030-03-01T12:00:00Z"]),
        json!({ "at": "2030-03-01T12:00:00Z" }),
        json!(null),
    ] {
        assert_rejected(&app, &with("purge_date", value), "purge_date").await;
    }
}

#[tokio::test]
async fn purge_date_before_the_voting_deadline_is_rejected() {
    let app = TestApp::new().await;
    // The Voting Deadline is 2030-01-22T12:00:00Z; the Clock stands at
    // 2030-01-15T12:00:00Z.
    for early in [
        "2030-01-22T11:59:59.999Z",
        // The same instant as 12:59:59 at +01:00: still before the deadline.
        "2030-01-22T12:59:59+01:00",
        "2030-01-16T00:00:00Z",
        "2030-01-15T12:00:00Z",
        "1970-01-01T00:00:00Z",
    ] {
        assert_rejected(&app, &with("purge_date", json!(early)), "purge_date").await;
    }
}

#[tokio::test]
async fn a_rejected_purge_date_creates_no_ballot() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let request = with("purge_date", json!("2030-01-20T00:00:00Z"));
    app.post_json("/admin/ballots", Some(&admin), &request)
        .await;

    let listed = body_json(app.admin_get("/admin/ballots", &admin).await).await;

    assert_eq!(listed["ballots"], json!([]));
}

// --- choice_mode and submission_mode -----------------------------------

/// Values that no enum field accepts: only the exact lower-case names do.
fn hostile_enum_values(valid: &str) -> Vec<Value> {
    vec![
        json!(""),
        json!(" "),
        json!(valid.to_uppercase()),
        json!(format!(" {valid}")),
        json!(format!("{valid} ")),
        json!(format!("{valid}\u{0}")),
        json!(format!("{valid}\u{200B}")),
        json!(format!("\u{202E}{valid}")),
        json!(format!("{valid}s")),
        json!("../../etc/passwd"),
        json!("<script>alert(1)</script>"),
        json!("x".repeat(10_000)),
        json!(0),
        json!(true),
        json!(null),
        json!([valid]),
        json!({ "mode": valid }),
    ]
}

#[tokio::test]
async fn hostile_choice_modes_are_rejected() {
    let app = TestApp::new().await;
    let mut values = hostile_enum_values("single");
    values.extend(hostile_enum_values("multiple"));
    values.extend([json!("final"), json!("one"), json!("Single")]);
    for value in values {
        assert_rejected(&app, &with("choice_mode", value), "choice_mode").await;
    }
}

#[tokio::test]
async fn hostile_submission_modes_are_rejected() {
    let app = TestApp::new().await;
    let mut values = hostile_enum_values("final");
    values.extend(hostile_enum_values("editable"));
    values.extend([json!("single"), json!("edit"), json!("Final")]);
    for value in values {
        assert_rejected(&app, &with("submission_mode", value), "submission_mode").await;
    }
}

#[tokio::test]
async fn invalid_modes_are_rejected_without_echoing_them() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    for field in ["choice_mode", "submission_mode"] {
        let response = app
            .post_json(
                "/admin/ballots",
                Some(&token),
                &with(field, json!("<script>alert(1)</script>")),
            )
            .await;

        let body = common::body_string(response).await;

        assert!(!body.contains("script"), "{body}");
    }
}

// --- the body as a whole ---------------------------------------------------

#[tokio::test]
async fn unknown_fields_are_rejected_without_echoing_their_names() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let mut nested = ballot_request();
    nested["options"][1]["ZZMARK<b>ZZ"] = json!(1);
    for request in [
        with("voter_ref", json!("x")),
        with("Group", json!("x")),
        with("__proto__", json!({})),
        with("ZZMARK<script>ZZ", json!(1)),
        nested,
    ] {
        let response = app
            .post_json("/admin/ballots", Some(&token), &request)
            .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let text = common::body_string(response).await;
        let body: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            body,
            json!({ "error": "invalid_input", "field": null, "message": "unknown field" }),
            "{request}"
        );
        for name in ["voter_ref", "Group", "__proto__", "ZZMARK"] {
            assert!(!text.contains(name), "{text}");
        }
    }
}

#[tokio::test]
async fn malformed_json_is_rejected() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let valid = ballot_request().to_string();
    for body in [
        "".to_owned(),
        "{".to_owned(),
        "[]".to_owned(),
        "null".to_owned(),
        format!("{valid} {{}}"),
        valid.replace("Lunch", "\\ud800"),
    ] {
        let response = app
            .post_raw(
                "/admin/ballots",
                Some(&token),
                "application/json",
                body.clone(),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
    }
    let mut invalid_utf8 = valid.into_bytes();
    let at = invalid_utf8.iter().position(|&b| b == b'L').unwrap();
    invalid_utf8[at] = 0xFF;
    let response = app
        .post_raw(
            "/admin/ballots",
            Some(&token),
            "application/json",
            invalid_utf8,
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn non_json_bodies_are_refused() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    for content_type in ["text/plain", "application/x-www-form-urlencoded", ""] {
        let response = app
            .post_raw(
                "/admin/ballots",
                Some(&token),
                content_type,
                ballot_request().to_string(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
}

#[tokio::test]
async fn bodies_over_64_kib_are_refused() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let padding = " ".repeat(64 * 1024);
    let body = format!("{}{padding}", ballot_request());

    let response = app
        .post_raw("/admin/ballots", Some(&token), "application/json", body)
        .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn rejections_never_echo_the_rejected_value() {
    let app = TestApp::new().await;
    let token = app.admin_token().await;
    let response = app
        .post_json(
            "/admin/ballots",
            Some(&token),
            &with("voting_deadline", json!("<script>alert(1)</script>")),
        )
        .await;

    let body = common::body_string(response).await;

    assert!(!body.contains("script"), "{body}");
}

// --- group and voter_reference -------------------------------------------

#[tokio::test]
async fn hostile_groups_and_voter_references_are_rejected() {
    let app = TestApp::new().await;
    for field in ["group", "voter_reference"] {
        for text in hostile_texts() {
            assert_rejected(&app, &with(field, json!(text)), field).await;
        }
        for text in ["../../etc/passwd", "<script>alert(1)</script>", "a/b\\c"] {
            // Not hostile as text: stored and returned verbatim, as data.
            let created = app.create_ballot(&with(field, json!(text))).await;
            assert_eq!(created[field], text, "{created}");
        }
    }
}

#[tokio::test]
async fn group_and_voter_reference_of_wrong_type_or_null_are_rejected() {
    let app = TestApp::new().await;
    for field in ["group", "voter_reference"] {
        for value in [
            json!(null),
            json!(42),
            json!(true),
            json!(["a"]),
            json!({ "a": 1 }),
        ] {
            assert_rejected(&app, &with(field, value), field).await;
        }
    }
}

#[tokio::test]
async fn group_and_voter_reference_are_trimmed_nfc_normalised_and_capped_in_characters() {
    let app = TestApp::new().await;
    for field in ["group", "voter_reference"] {
        let created = app
            .create_ballot(&with(field, json!("  Cafe\u{301}\t")))
            .await;
        assert_eq!(created[field], "Caf\u{e9}", "{created}");

        // 200 characters is the limit, counted after NFC, not in bytes.
        let created = app
            .create_ballot(&with(field, json!("e\u{301}".repeat(200))))
            .await;
        assert_eq!(created[field].as_str().unwrap().chars().count(), 200);
        app.create_ballot(&with(field, json!("\u{1F5F3}".repeat(200))))
            .await;
        assert_rejected(&app, &with(field, json!("\u{1F5F3}".repeat(201))), field).await;
    }
}
