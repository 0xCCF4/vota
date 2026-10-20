//! `/healthz`: the server time while the database answers, for local,
//! unproxied requests only (a loopback TCP peer and no forwarding header).
//! Every other request gets the same plain, empty 404 as an unknown path.

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, from_peer, snapshot};

#[tokio::test]
async fn freshly_started_app_reports_healthy() {
    let app = TestApp::new().await;

    let response = app.get_from("127.0.0.1", None, "/healthz").await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_reports_the_current_time_of_the_injected_clock() {
    let app = TestApp::new().await;
    app.clock.set("2031-06-01T08:30:00Z".parse().unwrap());

    let body = common::body_json(app.get_from("127.0.0.1", None, "/healthz").await).await;

    assert_eq!(body["status"], "ok");
    assert_eq!(body["now"], "2031-06-01T08:30:00Z");
}

#[tokio::test]
async fn advancing_the_clock_moves_the_reported_time() {
    let app = TestApp::new().await;
    app.clock.advance(chrono::Duration::weeks(4));

    let body = common::body_json(app.get_from("127.0.0.1", None, "/healthz").await).await;

    // The harness starts every test at 2030-01-15T12:00:00Z.
    assert_eq!(body["now"], "2030-02-12T12:00:00Z");
}

#[tokio::test]
async fn a_remote_peer_gets_the_plain_not_found_of_an_unknown_path() {
    let app = TestApp::new().await;

    for peer in ["192.0.2.1", "10.0.0.1", "2001:db8::1", "::ffff:192.0.2.1"] {
        let response = app.get_from(peer, None, "/healthz").await;

        assert_eq!(
            snapshot(response).await,
            snapshot(app.get_from(peer, None, "/no-such-path").await).await,
            "{peer}"
        );
    }
}

#[tokio::test]
async fn every_loopback_peer_gets_the_health_check() {
    let app = TestApp::new().await;

    for peer in ["127.0.0.1", "127.1.2.3", "::1", "::ffff:127.0.0.1"] {
        let response = app.get_from(peer, None, "/healthz").await;

        assert_eq!(response.status(), StatusCode::OK, "{peer}");
    }
}

/// GET `/healthz` from a loopback peer with one extra header.
async fn local_get_with(app: &TestApp, name: &str, value: &str) -> axum::http::Response<Body> {
    let request = Request::get("/healthz")
        .header(name, value)
        .body(Body::empty())
        .unwrap();
    app.send(from_peer("127.0.0.1", request)).await
}

#[tokio::test]
async fn a_forwarded_request_gets_the_plain_not_found_even_from_a_loopback_peer() {
    let app = TestApp::new().await;
    let not_found = snapshot(app.get_from("127.0.0.1", None, "/no-such-path").await).await;

    for (name, value) in [
        ("X-Forwarded-For", "192.0.2.1"),
        ("x-forwarded-for", "127.0.0.1"),
        ("X-FORWARDED-FOR", ""),
        ("x-forwarded-for", "not an address"),
        ("Forwarded", "for=192.0.2.1"),
        ("forwarded", "for=127.0.0.1;proto=https"),
        ("FORWARDED", ""),
    ] {
        let response = local_get_with(&app, name, value).await;

        assert_eq!(snapshot(response).await, not_found, "{name}: {value:?}");
    }
}

#[tokio::test]
async fn other_proxy_headers_do_not_hide_the_health_check() {
    let app = TestApp::new().await;

    for (name, value) in [("X-Real-IP", "192.0.2.1"), ("X-Forwarded-Proto", "https")] {
        let response = local_get_with(&app, name, value).await;

        assert_eq!(response.status(), StatusCode::OK, "{name}");
    }
}

#[tokio::test]
async fn a_request_without_a_known_peer_gets_the_plain_not_found() {
    let app = TestApp::new().await;

    // In-process, without a TCP connection: nothing proves it is local.
    let response = app.get("/healthz").await;

    assert_eq!(
        snapshot(response).await,
        snapshot(app.get("/no-such-path").await).await
    );
}
