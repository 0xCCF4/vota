//! Failed Ballot Token lookups are rate-limited per client IP, on the voting
//! page and the voter API. The client IP is the TCP peer, or taken from
//! `X-Forwarded-For` when the peer is a trusted proxy.

#![cfg(feature = "server")]

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use common::TestApp;

/// Well-formed, but never issued.
const UNKNOWN: &str = "AAAAAAAAAAAAAAAAAAAAAA";

/// An app that allows `failures` failed lookups per `window`.
async fn app_with_limit(failures: u32, window: Duration) -> TestApp {
    TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = failures.try_into().unwrap();
        config.rate_limit.window = window;
    })
    .await
}

const TEN_MINUTES: Duration = Duration::from_secs(600);

fn assert_throttled(response: &Response<Body>) {
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        response.headers().contains_key("retry-after"),
        "429 without Retry-After"
    );
}

#[tokio::test]
async fn voter_api_throttles_a_client_after_too_many_failed_lookups() {
    let app = app_with_limit(3, TEN_MINUTES).await;

    for _ in 0..3 {
        let response = app
            .get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = app
        .get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
        .await;

    assert_throttled(&response);
}

/// Spends `n` failed lookups from `peer` on the voter API.
async fn fail_lookups(app: &TestApp, peer: &str, n: u32) {
    for _ in 0..n {
        let response = app.get_from(peer, None, &format!("/api/b/{UNKNOWN}")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn voting_page_throttles_a_client_after_too_many_failed_lookups() {
    let app = app_with_limit(2, TEN_MINUTES).await;

    for path in ["/b/short", &format!("/b/{UNKNOWN}")] {
        let response = app.get_from("192.0.2.1", None, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = app
        .get_from("192.0.2.1", None, &format!("/b/{UNKNOWN}"))
        .await;

    assert_throttled(&response);
    assert!(
        common::body_string(response)
            .await
            .contains("Too many requests")
    );
}

#[tokio::test]
async fn failures_on_page_and_api_share_one_budget() {
    let app = app_with_limit(2, TEN_MINUTES).await;

    let page = app.get_from("192.0.2.1", None, "/b/short").await;
    let api = app
        .send(common::from_peer(
            "192.0.2.1",
            axum::http::Request::put(format!("/api/b/{UNKNOWN}/response"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"option_ids":["x"],"revision":0}"#))
                .unwrap(),
        ))
        .await;
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
    assert_eq!(api.status(), StatusCode::NOT_FOUND);

    assert_throttled(
        &app.get_from("192.0.2.1", None, &format!("/b/{UNKNOWN}"))
            .await,
    );
    assert_throttled(
        &app.get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
            .await,
    );
}

#[tokio::test]
async fn a_throttled_client_is_blocked_even_for_a_valid_token() {
    let app = app_with_limit(3, TEN_MINUTES).await;
    let (token, _) = app.create_standard_ballot().await;

    fail_lookups(&app, "192.0.2.1", 3).await;

    assert_throttled(
        &app.get_from("192.0.2.1", None, &format!("/api/b/{token}"))
            .await,
    );
    assert_throttled(
        &app.get_from("192.0.2.1", None, &format!("/b/{token}"))
            .await,
    );
}

#[tokio::test]
async fn other_clients_are_not_throttled() {
    let app = app_with_limit(3, TEN_MINUTES).await;

    fail_lookups(&app, "192.0.2.1", 3).await;
    let other = app
        .get_from("192.0.2.2", None, &format!("/api/b/{UNKNOWN}"))
        .await;

    assert_eq!(other.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_block_lifts_when_the_window_ends() {
    let app = app_with_limit(3, TEN_MINUTES).await;
    fail_lookups(&app, "192.0.2.1", 3).await;
    app.clock.advance(chrono::Duration::minutes(4));

    let blocked = app
        .get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
        .await;
    assert_throttled(&blocked);
    // The window started with the first failure, six minutes ago remain.
    assert_eq!(blocked.headers()["retry-after"], "360");

    app.clock.advance(chrono::Duration::minutes(6));
    let after = app
        .get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
        .await;
    assert_eq!(after.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_new_window_gets_a_fresh_budget() {
    let app = app_with_limit(3, TEN_MINUTES).await;
    fail_lookups(&app, "192.0.2.1", 2).await;
    app.clock.advance(chrono::Duration::minutes(10));

    // Two old failures expired: three more are allowed again.
    fail_lookups(&app, "192.0.2.1", 3).await;

    assert_throttled(
        &app.get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
            .await,
    );
}

#[tokio::test]
async fn successful_lookups_do_not_spend_the_budget() {
    let app = app_with_limit(3, TEN_MINUTES).await;
    let (token, _) = app.create_standard_ballot().await;

    for _ in 0..10 {
        let page = app
            .get_from("192.0.2.1", None, &format!("/b/{token}"))
            .await;
        let api = app
            .get_from("192.0.2.1", None, &format!("/api/b/{token}"))
            .await;
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(api.status(), StatusCode::OK);
    }
    fail_lookups(&app, "192.0.2.1", 3).await;

    assert_throttled(
        &app.get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
            .await,
    );
}

#[tokio::test]
async fn by_default_twenty_failed_lookups_per_ten_minutes_are_allowed() {
    let app = TestApp::new().await;

    fail_lookups(&app, "192.0.2.1", 20).await;
    let blocked = app
        .get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
        .await;

    assert_throttled(&blocked);
    assert_eq!(blocked.headers()["retry-after"], "600");
}

#[tokio::test]
async fn ipv6_clients_are_counted_per_64_prefix() {
    let app = app_with_limit(2, TEN_MINUTES).await;

    fail_lookups(&app, "2001:db8:1:2::1", 1).await;
    fail_lookups(&app, "2001:db8:1:2::ffff", 1).await;

    assert_throttled(
        &app.get_from("2001:db8:1:2:abcd::9", None, &format!("/api/b/{UNKNOWN}"))
            .await,
    );
    let other_prefix = app
        .get_from("2001:db8:1:3::1", None, &format!("/api/b/{UNKNOWN}"))
        .await;
    assert_eq!(other_prefix.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_routes_are_not_rate_limited() {
    let app = app_with_limit(1, TEN_MINUTES).await;
    let admin = app.admin_token().await;
    fail_lookups(&app, "192.0.2.1", 1).await;

    let request = axum::http::Request::get("/admin/ballots")
        .header("authorization", format!("Bearer {admin}"))
        .body(Body::empty())
        .unwrap();
    let response = app.send(common::from_peer("192.0.2.1", request)).await;

    assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

/// An app behind a reverse proxy in 10.0.0.0/8, allowing two failed lookups.
async fn app_behind_proxy() -> TestApp {
    TestApp::with_config(|config| {
        config.trusted_proxies = vec!["10.0.0.0/8".parse().unwrap()];
        config.rate_limit.failed_lookups = 2.try_into().unwrap();
    })
    .await
}

async fn lookup_via(app: &TestApp, peer: &str, forwarded_for: &str) -> Response<Body> {
    app.get_from(peer, Some(forwarded_for), &format!("/api/b/{UNKNOWN}"))
        .await
}

#[tokio::test]
async fn behind_a_trusted_proxy_clients_are_told_apart_by_x_forwarded_for() {
    let app = app_behind_proxy().await;

    for _ in 0..2 {
        let response = lookup_via(&app, "10.0.0.1", "203.0.113.5").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    assert_throttled(&lookup_via(&app, "10.0.0.1", "203.0.113.5").await);
    let other_client = lookup_via(&app, "10.0.0.1", "203.0.113.6").await;
    assert_eq!(other_client.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_rightmost_untrusted_forwarded_address_is_the_client() {
    let app = app_behind_proxy().await;

    // The client can prepend anything; only what the proxies appended counts.
    for spoofed in ["198.51.100.1", "198.51.100.2"] {
        let chain = format!("{spoofed}, 203.0.113.5, 10.0.0.2");
        let response = lookup_via(&app, "10.0.0.1", &chain).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    assert_throttled(&lookup_via(&app, "10.0.0.1", "198.51.100.3,203.0.113.5").await);
}

#[tokio::test]
async fn forwarded_addresses_from_an_untrusted_peer_are_ignored() {
    let app = app_behind_proxy().await;

    for spoofed in ["203.0.113.1", "203.0.113.2"] {
        let response = lookup_via(&app, "192.0.2.1", spoofed).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // Still the same client, whatever it claims.
    assert_throttled(&lookup_via(&app, "192.0.2.1", "203.0.113.3").await);
    assert_throttled(
        &app.get_from("192.0.2.1", None, &format!("/api/b/{UNKNOWN}"))
            .await,
    );
}

/// `X-Forwarded-For` values a trusted proxy may pass on that can't be read
/// as a whole: a hop that isn't a bare IP address anywhere in the chain (as
/// nginx's `$proxy_add_x_forwarded_for` appends the real peer to whatever
/// the client sent), or more than 32 hops.
fn malformed_forwarded_for() -> Vec<String> {
    let mut values: Vec<String> = [
        "203.0.113.1:8080",
        "not-an-ip",
        "203.0.113.2,,",
        "",
        "junk, 203.0.113.5",
        "unknown, 203.0.113.5",
        "[2001:db8::1], 203.0.113.5",
    ]
    .map(String::from)
    .to_vec();
    let hops: Vec<String> = (1..=33).map(|i| format!("198.51.100.{i}")).collect();
    values.push(hops.join(", "));
    values
}

#[tokio::test]
async fn a_malformed_forwarded_header_from_a_trusted_proxy_is_a_bad_request() {
    let app = app_behind_proxy().await;
    let (token, _) = app.create_standard_ballot().await;

    for value in malformed_forwarded_for() {
        for path in [
            format!("/api/b/{UNKNOWN}"),
            format!("/b/{UNKNOWN}"),
            format!("/api/b/{token}"),
            format!("/b/{token}"),
        ] {
            let response = app.get_from("10.0.0.1", Some(&value), &path).await;

            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{path} with {value:?}"
            );
            common::assert_hardened(&response, &value);
        }
    }
}

#[tokio::test]
async fn a_malformed_forwarded_header_is_charged_to_no_bucket() {
    let app = app_behind_proxy().await;

    for value in malformed_forwarded_for() {
        for _ in 0..3 {
            let response = lookup_via(&app, "10.0.0.1", &value).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{value:?}");
        }
    }

    // Neither the proxy's own bucket nor the real client's (the right-most
    // address) was charged: each still has its whole budget of two.
    for _ in 0..2 {
        let response = app
            .get_from("10.0.0.1", None, &format!("/api/b/{UNKNOWN}"))
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let real_client = lookup_via(&app, "10.0.0.1", "203.0.113.5").await;
        assert_eq!(real_client.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn thirty_two_forwarded_addresses_are_still_read() {
    let app = app_behind_proxy().await;
    let mut hops: Vec<String> = (1..=31).map(|i| format!("198.51.100.{i}")).collect();
    hops.push("203.0.113.5".into());
    let chain = hops.join(", ");

    for _ in 0..2 {
        assert_eq!(
            lookup_via(&app, "10.0.0.1", &chain).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    assert_throttled(&lookup_via(&app, "10.0.0.1", "203.0.113.5").await);
}

#[tokio::test]
async fn a_malformed_forwarded_header_from_an_untrusted_peer_is_ignored() {
    let app = app_behind_proxy().await;

    let response = lookup_via(&app, "192.0.2.1", "junk").await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_concurrent_burst_gets_no_more_lookups_than_the_budget() {
    let app = app_with_limit(5, TEN_MINUTES).await;

    let requests: Vec<_> = (0..300)
        .map(|_| {
            let router = app.router.clone();
            tokio::spawn(async move {
                use tower::ServiceExt as _;
                let request = axum::http::Request::get(format!("/api/b/{UNKNOWN}"))
                    .body(Body::empty())
                    .unwrap();
                let request = common::from_peer("192.0.2.1", request);
                router.oneshot(request).await.unwrap().status()
            })
        })
        .collect();
    let mut not_found = 0;
    for request in requests {
        let status = request.await.unwrap();
        if status == StatusCode::NOT_FOUND {
            not_found += 1;
        } else {
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        }
    }

    assert_eq!(not_found, 5);
}

#[tokio::test]
async fn a_trusted_proxy_given_in_ipv4_mapped_ipv6_form_is_trusted_as_its_ipv4_address() {
    // `::ffff:10.0.0.0/104` is `10.0.0.0/8`; the peer arrives as plain IPv4.
    let app = TestApp::with_config(|config| {
        config.trusted_proxies = vec!["::ffff:10.0.0.0/104".parse().unwrap()];
        config.rate_limit.failed_lookups = 2.try_into().unwrap();
    })
    .await;

    for _ in 0..2 {
        let response = lookup_via(&app, "10.0.0.1", "203.0.113.5").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    assert_throttled(&lookup_via(&app, "10.0.0.1", "203.0.113.5").await);
    let other_client = lookup_via(&app, "10.0.0.1", "203.0.113.6").await;
    assert_eq!(other_client.status(), StatusCode::NOT_FOUND);
}
