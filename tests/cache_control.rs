//! Responses about a Ballot are personal: the voting page, the confirmation
//! step and every voter API response carry `Cache-Control: no-store`, so no
//! cache (a proxy, a CDN, the browser's disk or back/forward cache) keeps
//! them. So do the neutral error responses (404, 400, 429, 500).

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Request, Response, header};
use common::{TestApp, from_peer};
use serde_json::json;

const UNKNOWN: &str = "AAAAAAAAAAAAAAAAAAAAAA";

fn assert_no_store(response: &Response<Body>, what: &str) {
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|value| value.to_str().unwrap()),
        Some("no-store"),
        "{what}: {}",
        response.status()
    );
}

#[tokio::test]
async fn voting_pages_and_their_answers_are_never_stored() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "final").await;
    let page = format!("/b/{token}");
    let revision = app.voter_view(&token).await["revision"].clone();
    let form = |extra: &str| format!("option={}&revision={revision}{extra}", options[0]);

    assert_no_store(&app.get(&page).await, "the voting page");
    assert_no_store(
        &app.post_form(&page, &form("")).await,
        "the confirmation step",
    );
    assert_no_store(
        &app.post_form(&page, "option=x").await,
        "the form again with a problem (422)",
    );
    assert_no_store(
        &app.post_form(&page, &form("&confirm=yes")).await,
        "the redirect after recording",
    );
    assert_no_store(
        &app.post_form(&page, &form("&confirm=yes")).await,
        "the read-only page with a refusal (409)",
    );
    assert_no_store(&app.get(&format!("/b/{UNKNOWN}")).await, "the not-found");
    assert_no_store(&app.get(&format!("/b/{token}/x")).await, "a path below it");
}

#[tokio::test]
async fn voter_api_responses_are_never_stored() {
    let app = TestApp::new().await;
    let (token, options) = app.create_ballot_with_modes("single", "final").await;
    let api = format!("/api/b/{token}");
    let response = format!("{api}/response");
    let revision = app.voter_view(&token).await["revision"].clone();
    let body = json!({ "option_ids": [options[0]], "revision": revision });

    assert_no_store(&app.get(&api).await, "GET");
    assert_no_store(&app.put_json(&response, &body).await, "PUT");
    assert_no_store(&app.put_json(&response, &body).await, "PUT refused (409)");
    assert_no_store(
        &app.put_json(&response, &json!({ "option_ids": "x" })).await,
        "PUT invalid (422)",
    );
    assert_no_store(
        &app.put_raw(&response, "text/plain", "x").await,
        "PUT of another content type (415)",
    );
    assert_no_store(
        &app.put_raw(&response, "application/json", vec![b' '; 70_000])
            .await,
        "PUT too large (413)",
    );
    let delete = Request::delete(&api).body(Body::empty()).unwrap();
    assert_no_store(&app.send(delete).await, "another method (405)");
    assert_no_store(&app.get(&format!("/api/b/{UNKNOWN}")).await, "not found");
}

#[tokio::test]
async fn refusals_of_the_rate_limit_are_never_stored() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = 1.try_into().unwrap();
        config.trusted_proxies = vec!["10.0.0.0/8".parse().unwrap()];
    })
    .await;
    let lookup = |peer: &str, path: &str| {
        let request = Request::get(path).body(Body::empty()).unwrap();
        from_peer(peer, request)
    };

    app.send(lookup("192.0.2.1", &format!("/api/b/{UNKNOWN}")))
        .await;
    for path in [format!("/api/b/{UNKNOWN}"), format!("/b/{UNKNOWN}")] {
        assert_no_store(&app.send(lookup("192.0.2.1", &path)).await, "429");
        let bad_header = app.get_from("10.0.0.1", Some("junk"), &path).await;
        assert_no_store(&bad_header, "400");
    }
}

#[tokio::test]
async fn internal_errors_of_the_voter_surfaces_are_never_stored() {
    let app = TestApp::new().await;
    let (token, _) = app.create_standard_ballot().await;
    app.wipe_database_files();

    assert_no_store(&app.get(&format!("/b/{token}")).await, "page 500");
    assert_no_store(&app.get(&format!("/api/b/{token}")).await, "API 500");
}
