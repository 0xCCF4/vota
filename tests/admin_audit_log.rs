//! The admin audit log (security audit I-1): one structured event per admin
//! request with the Admin Token's ID (never the secret), the method, the
//! route template (never the raw path), the status and the client IP; and
//! warnings that sum up failed Admin Token checks per client IP. Nothing
//! secret or personal is ever logged: no Admin Token, Ballot Token, Voter
//! Reference or request body.

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use common::{CapturedLogs, TestApp, ballot_request, body_json, from_peer};
use serde_json::{Value, json};
use vota::data::admin_tokens::NewAdminToken;

/// An Admin Token labelled `label`, and its ID as `admin-token list` shows it.
async fn admin_token_and_id(app: &TestApp, label: &str) -> (String, i64) {
    let secret = app
        .admin_token_with(NewAdminToken {
            label: Some(label.into()),
            expires_at: None,
        })
        .await;
    let output = assert_cmd::Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .env("VOTA_DATABASE_PATH", &app.state.config.database_path)
        .args(["admin-token", "list", "--json"])
        .output()
        .unwrap();
    let listed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|token| token["label"] == label)
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    (secret, id)
}

/// An admin request from the TCP peer `peer`.
async fn admin_request(
    app: &TestApp,
    peer: &str,
    method: &str,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> Response<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(body) => {
            request = request.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    app.send(from_peer(peer, request.body(body).unwrap())).await
}

/// The log lines of admin request events.
fn request_events(logs: &str) -> Vec<&str> {
    logs.lines()
        .filter(|line| line.contains("admin request"))
        .collect()
}

#[tokio::test]
async fn each_admin_request_is_logged_with_token_id_method_route_status_and_client_ip() {
    let app = TestApp::new().await;
    let (admin, id) = admin_token_and_id(&app, "audit").await;
    let logs = CapturedLogs::default();
    let guard = logs.start();

    let created = admin_request(
        &app,
        "192.0.2.7",
        "POST",
        "/admin/ballots",
        &admin,
        Some(&ballot_request()),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let ballot = body_json(created).await["id"].clone();
    let path = format!("/admin/ballots/{ballot}");
    admin_request(&app, "192.0.2.7", "GET", &path, &admin, None).await;
    admin_request(
        &app,
        "192.0.2.7",
        "GET",
        "/admin/nothing/here",
        &admin,
        None,
    )
    .await;

    drop(guard);
    let logs = logs.text();
    let events = request_events(&logs);
    assert_eq!(events.len(), 3, "{logs}");
    for (event, (method, route, status)) in events.iter().zip([
        ("POST", "/admin/ballots", 201),
        ("GET", "/admin/ballots/{id}", 200),
        ("GET", "/admin/{*rest}", 404),
    ]) {
        for field in [
            format!("token_id={id}"),
            format!("method={method}"),
            format!("route={route}"),
            format!("status={status}"),
            "client_ip=192.0.2.7".to_owned(),
        ] {
            assert!(event.contains(&format!(" {field}")), "{field} in {event}");
        }
    }
    assert!(!logs.contains(&path), "{logs}");
    assert!(!logs.contains("/admin/nothing/here"), "{logs}");
}

#[tokio::test]
async fn failed_admin_token_checks_are_summed_up_per_client_ip() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.admin_failed_auth = 4.try_into().unwrap();
    })
    .await;
    let wrong = "vta_ZZWRONGZZAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let logs = CapturedLogs::default();
    let guard = logs.start();

    for _ in 0..5 {
        admin_request(&app, "198.51.100.9", "GET", "/admin/ballots", wrong, None).await;
    }

    drop(guard);
    let logs = logs.text();
    let warnings: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains(" WARN "))
        .collect();
    assert_eq!(warnings.len(), 2, "{logs}");
    assert!(warnings[0].contains("failed Admin Token check"), "{logs}");
    assert!(
        warnings[1].contains("too many failed Admin Token checks"),
        "{logs}"
    );
    assert!(warnings[1].contains("failures=4"), "{logs}");
    for warning in &warnings {
        assert!(warning.contains("client_ip=198.51.100.9"), "{warning}");
    }
    let events = request_events(&logs);
    let statuses: Vec<bool> = events
        .iter()
        .map(|event| event.contains("status=401"))
        .collect();
    assert_eq!(statuses, [true, true, true, true, false], "{logs}");
    assert!(events[4].contains("status=429"), "{logs}");
    assert!(
        events.iter().all(|event| !event.contains("token_id")),
        "{logs}"
    );
    assert!(!logs.contains("ZZWRONGZZ"), "{logs}");
}

#[tokio::test]
async fn the_log_never_holds_tokens_voter_references_or_request_bodies() {
    let app = TestApp::new().await;
    let (admin, _) = admin_token_and_id(&app, "audit").await;
    let logs = CapturedLogs::default();
    let guard = logs.start();

    let mut request = ballot_request();
    request["title"] = json!("ZZTITLEZZ");
    request["group"] = json!("ZZGROUPZZ");
    request["voter_reference"] = json!("zz-alice@example.org");
    let created = admin_request(
        &app,
        "192.0.2.7",
        "POST",
        "/admin/ballots",
        &admin,
        Some(&request),
    )
    .await;
    let created = body_json(created).await;
    let ballot_token = created["token"].as_str().unwrap().to_owned();
    let path = format!("/admin/ballots/{}", created["id"]);
    admin_request(
        &app,
        "192.0.2.7",
        "GET",
        "/admin/ballots?group=ZZGROUPZZ",
        &admin,
        None,
    )
    .await;
    let edit = json!({ "voter_reference": "zz-bob@example.org" });
    admin_request(&app, "192.0.2.7", "PATCH", &path, &admin, Some(&edit)).await;
    admin_request(
        &app,
        "192.0.2.7",
        "PATCH",
        &path,
        &admin,
        Some(&json!({ "ZZFIELDZZ": 1 })),
    )
    .await;
    admin_request(&app, "192.0.2.7", "DELETE", &path, &admin, None).await;
    admin_request(
        &app,
        "192.0.2.7",
        "GET",
        &format!("/admin/b/{ballot_token}"),
        &admin,
        None,
    )
    .await;
    let _ = app.get(&format!("/api/b/{ballot_token}")).await;

    drop(guard);
    let logs = logs.text();
    assert_eq!(request_events(&logs).len(), 6, "{logs}");
    for secret in [
        &admin["vta_".len()..],
        &ballot_token,
        "ZZTITLEZZ",
        "ZZGROUPZZ",
        "zz-alice",
        "zz-bob",
        "ZZFIELDZZ",
        "example.org",
    ] {
        assert!(!logs.contains(secret), "{secret} in {logs}");
    }
}

#[tokio::test]
async fn a_nat64_client_is_logged_as_its_ipv4_address() {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let logs = CapturedLogs::default();
    let guard = logs.start();

    let peer = "64:ff9b::198.51.100.7";
    let response = admin_request(&app, peer, "GET", "/admin/ballots", &admin, None).await;

    drop(guard);
    assert_eq!(response.status(), StatusCode::OK);
    let logs = logs.text();
    let events = request_events(&logs);
    assert_eq!(events.len(), 1, "{logs}");
    assert!(events[0].contains(" client_ip=198.51.100.7"), "{logs}");
}
