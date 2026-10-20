//! Hostile and invalid input to the admin's Ballot management endpoints,
//! through the HTTP seam: `PATCH /admin/ballots/{id}` (edit),
//! `POST /admin/ballots/{id}/extend` and `DELETE /admin/ballots/{id}`.
//! Every rejected body is a 422 naming the offending field and leaves the
//! Ballot exactly as it was.

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, ballot_request, body_json, body_string};
use serde_json::{Value, json};

/// Text that every free-text field must refuse (as at creation).
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

/// Timestamps that are not RFC 3339 with an explicit offset.
fn malformed_timestamps() -> Vec<Value> {
    vec![
        json!("2030-02-01T12:00:00"),
        json!("2030-02-01"),
        json!("2030-02-30T12:00:00Z"),
        json!("2030-02-01T25:00:00Z"),
        json!(" 2030-02-01T12:00:00Z"),
        json!("2030-02-01T12:00:00Z\u{202E}"),
        json!("01.02.2030 12:00"),
        json!("tomorrow"),
        json!("../../etc/passwd"),
        json!("2030-02-01T12:00:00Z\u{0}"),
        json!(""),
        json!(1_900_000_000),
        json!(["2030-02-01T12:00:00Z"]),
        json!(null),
    ]
}

/// An app with a Ballot that has every optional field set, so that any
/// unintended change shows; returns the app, an Admin Token and the Ballot.
async fn setup() -> (TestApp, String, Value) {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let mut request = ballot_request();
    request["fallback"] = json!(["Sushi"]);
    request["group"] = json!("Survey");
    request["voter_reference"] = json!("alice");
    let created = app.create_ballot(&request).await;
    (app, admin, created)
}

fn path(created: &Value) -> String {
    format!("/admin/ballots/{}", created["id"])
}

fn extend_path(created: &Value) -> String {
    format!("/admin/ballots/{}/extend", created["id"])
}

async fn assert_unchanged(app: &TestApp, admin: &str, created: &Value, after: &str) {
    let fetched = body_json(app.admin_get(&path(created), admin).await).await;
    assert_eq!(&fetched, created, "unchanged after {after}");
}

async fn assert_422(response: axum::http::Response<Body>, field: impl Into<Value>, what: &str) {
    let field = field.into();
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for {what}"
    );
    let error = body_json(response).await;
    assert_eq!(error["error"], "invalid_input", "{error}");
    assert_eq!(error["field"], field, "for {what}: {error}");
}

/// Edits with `edit` and asserts a 422 naming `field`, Ballot unchanged.
async fn assert_edit_rejected(
    app: &TestApp,
    admin: &str,
    created: &Value,
    edit: &Value,
    field: impl Into<Value>,
) {
    let response = app.admin_patch(&path(created), admin, edit).await;
    assert_422(response, field, &edit.to_string()).await;
    assert_unchanged(app, admin, created, &edit.to_string()).await;
}

/// Extends with `body` and asserts a 422 naming `field`, Ballot unchanged.
async fn assert_extension_rejected(
    app: &TestApp,
    admin: &str,
    created: &Value,
    body: &Value,
    field: impl Into<Value>,
) {
    let response = app
        .post_json(&extend_path(created), Some(admin), body)
        .await;
    assert_422(response, field, &body.to_string()).await;
    assert_unchanged(app, admin, created, &body.to_string()).await;
}

// --- edit: text fields ---------------------------------------------------

#[tokio::test]
async fn hostile_titles_are_rejected() {
    let (app, admin, created) = setup().await;
    for text in hostile_texts() {
        assert_edit_rejected(&app, &admin, &created, &json!({ "title": text }), "title").await;
    }
}

#[tokio::test]
async fn hostile_option_labels_are_rejected_naming_the_option() {
    let (app, admin, created) = setup().await;
    for text in hostile_texts() {
        let edit = json!({ "options": [{ "label": "Sushi" }, { "label": text }] });
        assert_edit_rejected(&app, &admin, &created, &edit, "options[1].label").await;
    }
}

#[tokio::test]
async fn too_few_duplicate_or_malformed_options_are_rejected() {
    let (app, admin, created) = setup().await;
    let cases = [
        (json!([]), "options"),
        (json!([{ "label": "Sushi" }]), "options"),
        // Equal after NFC normalisation and trimming.
        (
            json!([{ "label": "Sushi" }, { "label": "Caf\u{e9}" }, { "label": " Cafe\u{301} " }]),
            "options[2].label",
        ),
        (json!([{ "label": "Sushi" }, "Curry"]), "options[1]"),
        (json!("Sushi, Curry"), "options"),
    ];
    for (options, field) in cases {
        assert_edit_rejected(
            &app,
            &admin,
            &created,
            &json!({ "options": options }),
            field,
        )
        .await;
    }
    // Option IDs are not a field of a request's Option: unknown, unnamed.
    let options = json!([{ "label": "Sushi" }, { "label": "Curry", "id": "x" }]);
    let edit = json!({ "options": options });
    assert_edit_rejected(&app, &admin, &created, &edit, Value::Null).await;
}

#[tokio::test]
async fn hostile_groups_and_voter_references_are_rejected() {
    let (app, admin, created) = setup().await;
    for field in ["group", "voter_reference"] {
        for text in hostile_texts() {
            assert_edit_rejected(&app, &admin, &created, &json!({ field: text }), field).await;
        }
        for value in [json!(7), json!(["x"]), json!({ "x": 1 }), json!(true)] {
            assert_edit_rejected(&app, &admin, &created, &json!({ field: value }), field).await;
        }
    }
}

// --- edit: enums ---------------------------------------------------------

#[tokio::test]
async fn hostile_modes_are_rejected() {
    let (app, admin, created) = setup().await;
    let names = [
        "Single",
        " single",
        "single ",
        "SINGLE",
        "both",
        "",
        "final\u{0}",
        "Multiple",
    ];
    for field in ["choice_mode", "submission_mode"] {
        for name in names {
            assert_edit_rejected(&app, &admin, &created, &json!({ field: name }), field).await;
        }
        assert_edit_rejected(&app, &admin, &created, &json!({ field: 1 }), field).await;
    }
}

// --- edit: timestamps ----------------------------------------------------

#[tokio::test]
async fn malformed_deadlines_and_purge_dates_are_rejected() {
    let (app, admin, created) = setup().await;
    for field in ["voting_deadline", "purge_date"] {
        for value in malformed_timestamps() {
            assert_edit_rejected(&app, &admin, &created, &json!({ field: value }), field).await;
        }
    }
}

#[tokio::test]
async fn a_purge_date_in_the_past_is_rejected() {
    let (app, admin, created) = setup().await;
    // Close the Ballot without a Response and without a Fallback (so it stays
    // editable), then try to purge it between its deadline and now.
    let removed = app
        .admin_patch(&path(&created), &admin, &json!({ "fallback": null }))
        .await;
    assert_eq!(removed.status(), StatusCode::OK);
    app.clock.set("2030-01-25T00:00:00Z".parse().unwrap());
    let created = body_json(app.admin_get(&path(&created), &admin).await).await;

    let edit = json!({ "purge_date": "2030-01-23T00:00:00Z" });
    assert_edit_rejected(&app, &admin, &created, &edit, "purge_date").await;
}

// --- edit: Fallback ------------------------------------------------------

#[tokio::test]
async fn invalid_fallbacks_are_rejected() {
    let (app, admin, created) = setup().await;
    let cases = [
        (json!([]), "fallback"),
        (json!(["Pizza", "Sushi"]), "fallback"),
        (json!(["Sushi", "Sushi"]), "fallback[1]"),
        (json!(["sushi"]), "fallback[0]"),
        (json!(["Curry"]), "fallback[0]"),
        (json!(["Su\u{200B}shi"]), "fallback[0]"),
        (json!(["../Sushi"]), "fallback[0]"),
        (json!([created["options"][1]["id"]]), "fallback[0]"),
        (json!("Sushi"), "fallback"),
        (json!([1]), "fallback[0]"),
    ];
    for (fallback, field) in cases {
        assert_edit_rejected(
            &app,
            &admin,
            &created,
            &json!({ "fallback": fallback }),
            field,
        )
        .await;
    }
}

// --- edit: body shape ----------------------------------------------------

#[tokio::test]
async fn null_is_rejected_for_fields_that_cannot_be_cleared() {
    let (app, admin, created) = setup().await;
    for field in [
        "title",
        "options",
        "choice_mode",
        "submission_mode",
        "voting_deadline",
        "purge_date",
    ] {
        assert_edit_rejected(&app, &admin, &created, &json!({ field: null }), field).await;
    }
}

#[tokio::test]
async fn unknown_and_read_only_fields_are_rejected_without_naming_them() {
    let (app, admin, created) = setup().await;
    for field in [
        "id",
        "token",
        "url",
        "status",
        "response",
        "created_at",
        "titel",
        "__proto__",
    ] {
        let edit = json!({ field: "x" });
        assert_edit_rejected(&app, &admin, &created, &edit, Value::Null).await;
    }
}

#[tokio::test]
async fn malformed_json_and_non_json_bodies_are_refused() {
    let (app, admin, created) = setup().await;
    for body in [
        "",
        "{",
        "[]",
        "null",
        "\"title\"",
        "{\"title\": \"A\"} {}",
        "{\"title\": \"A\",}",
    ] {
        let response = app
            .admin_patch_raw(&path(&created), &admin, "application/json", body)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body:?}"
        );
    }
    for content_type in ["text/plain", "application/x-www-form-urlencoded"] {
        let response = app
            .admin_patch_raw(&path(&created), &admin, content_type, "{\"title\":\"A\"}")
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{content_type}"
        );
    }
    assert_unchanged(&app, &admin, &created, "malformed bodies").await;
}

#[tokio::test]
async fn bodies_over_64_kib_are_refused() {
    let (app, admin, created) = setup().await;
    let body = json!({ "title": "x".repeat(70 * 1024) }).to_string();

    let response = app
        .admin_patch_raw(&path(&created), &admin, "application/json", body)
        .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_unchanged(&app, &admin, &created, "an oversized body").await;
}

#[tokio::test]
async fn rejections_never_echo_the_rejected_value() {
    let (app, admin, created) = setup().await;
    let marker = "<script>alert('pwned-marker')</script>\u{7}";
    for edit in [
        json!({ "title": marker }),
        json!({ "choice_mode": marker }),
        json!({ "fallback": [marker] }),
        json!({ "purge_date": marker }),
    ] {
        let response = app.admin_patch(&path(&created), &admin, &edit).await;
        let body = body_string(response).await;
        assert!(!body.contains("pwned-marker"), "{body}");
    }
}

#[tokio::test]
async fn an_answered_ballot_is_refused_even_with_an_invalid_edit() {
    let (app, admin, created) = setup().await;
    let token = created["token"].as_str().unwrap();
    let pizza = &created["options"][0]["id"];
    app.put_json(
        &format!("/api/b/{token}/response"),
        &json!({ "option_ids": [pizza] }),
    )
    .await;

    let response = app
        .admin_patch(&path(&created), &admin, &json!({ "title": "" }))
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(response).await["error"], "already_answered");
}

// --- extend --------------------------------------------------------------

#[tokio::test]
async fn malformed_new_deadlines_are_rejected() {
    let (app, admin, created) = setup().await;
    for value in malformed_timestamps() {
        let body = json!({ "voting_deadline": value });
        assert_extension_rejected(&app, &admin, &created, &body, "voting_deadline").await;
    }
}

#[tokio::test]
async fn an_extension_needs_exactly_a_voting_deadline() {
    let (app, admin, created) = setup().await;
    assert_extension_rejected(&app, &admin, &created, &json!({}), "voting_deadline").await;
    let cases = [
        // Fields of other requests are unknown here: not named.
        (
            json!({ "voting_deadline": "2030-02-01T00:00:00Z", "purge_date": "2030-05-01T00:00:00Z" }),
            Value::Null,
        ),
        (
            json!({ "voting_deadline": "2030-02-01T00:00:00Z", "title": "x" }),
            Value::Null,
        ),
    ];
    for (body, field) in cases {
        assert_extension_rejected(&app, &admin, &created, &body, field).await;
    }
    for body in [
        "",
        "[]",
        "null",
        "{\"voting_deadline\": \"2030-02-01T00:00:00Z\"} x",
    ] {
        let response = app
            .post_raw(
                &extend_path(&created),
                Some(&admin),
                "application/json",
                body,
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body:?}"
        );
    }
    let response = app
        .post_raw(
            &extend_path(&created),
            Some(&admin),
            "text/plain",
            "{\"voting_deadline\": \"2030-02-01T00:00:00Z\"}",
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_unchanged(&app, &admin, &created, "bad extension bodies").await;
}

#[tokio::test]
async fn far_future_deadlines_keep_the_purge_date_after_the_deadline() {
    // A horizon wide enough for the year 9999 (the default caps at one year).
    let app = TestApp::with_config(|config| {
        config.max_date_horizon = std::time::Duration::from_secs(10_000 * 366 * 24 * 60 * 60);
    })
    .await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;

    let response = app
        .post_json(
            &extend_path(&created),
            Some(&admin),
            &json!({ "voting_deadline": "9999-12-31T23:59:59Z" }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let extended = body_json(response).await;
    assert_eq!(extended["voting_deadline"], "9999-12-31T23:59:59Z");
    let purge = extended["purge_date"].as_str().unwrap();
    assert!(
        purge.starts_with("+10000-01-28") || purge.starts_with("10000-01-28"),
        "{purge}"
    );
}

// --- the Ballot ID in the path, and auth ---------------------------------

/// Paths with an unknown or malformed Ballot ID: all must be the admin API's
/// plain 404.
fn bad_ids() -> Vec<String> {
    [
        "999999",
        "0",
        "-1",
        "+1",
        "01",
        "1.0",
        "1e3",
        "%201",
        "1%00",
        "abc",
        "..%2F1",
        "%FF",
        "99999999999999999999999",
    ]
    .iter()
    .map(|id| (*id).to_owned())
    .collect()
}

#[tokio::test]
async fn unknown_and_malformed_ids_are_not_found_on_every_new_endpoint() {
    let (app, admin, created) = setup().await;
    let not_found = json!({ "error": "not_found" });
    for id in bad_ids() {
        let edit = app
            .admin_patch(
                &format!("/admin/ballots/{id}"),
                &admin,
                &json!({ "title": "x" }),
            )
            .await;
        assert_eq!(edit.status(), StatusCode::NOT_FOUND, "PATCH {id}");
        assert_eq!(body_json(edit).await, not_found);
        let extend = app
            .post_json(
                &format!("/admin/ballots/{id}/extend"),
                Some(&admin),
                &json!({ "voting_deadline": "2030-02-01T00:00:00Z" }),
            )
            .await;
        assert_eq!(extend.status(), StatusCode::NOT_FOUND, "extend {id}");
        let delete = app
            .admin_delete(&format!("/admin/ballots/{id}"), &admin)
            .await;
        assert_eq!(delete.status(), StatusCode::NOT_FOUND, "DELETE {id}");
    }
    assert_unchanged(&app, &admin, &created, "bad IDs").await;
}

#[tokio::test]
async fn managing_ballots_needs_a_valid_admin_token() {
    let (app, admin, created) = setup().await;
    let requests = [
        Request::patch(path(&created))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "title": "x" }).to_string())),
        Request::post(extend_path(&created))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "voting_deadline": "2030-02-01T00:00:00Z" }).to_string(),
            )),
        Request::delete(path(&created)).body(Body::empty()),
    ];
    for request in requests {
        let response = app.send(request.unwrap()).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let wrong = app
        .admin_delete(
            &path(&created),
            "vta_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_unchanged(&app, &admin, &created, "unauthenticated requests").await;
}
