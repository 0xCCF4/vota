//! A client whose IPv4 address reaches the app inside an IPv6 address (behind
//! a NAT64 or SIIT translator such as Jool) is counted as that IPv4 address:
//! by the rate limits and in the admin audit log (`admin_audit_log.rs`). Trust decisions (trusted
//! proxies, the local `/healthz` check) don't look inside such addresses.

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use common::{TestApp, from_peer, snapshot};

/// Well-formed, but never issued.
const UNKNOWN: &str = "AAAAAAAAAAAAAAAAAAAAAA";

/// An app that allows two failed lookups per client.
async fn app_with_two_failures(configure: impl FnOnce(&mut vota::config::Config)) -> TestApp {
    TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = 2.try_into().unwrap();
        configure(config);
    })
    .await
}

async fn lookup_from(app: &TestApp, peer: &str, forwarded_for: Option<&str>) -> Response<Body> {
    app.get_from(peer, forwarded_for, &format!("/api/b/{UNKNOWN}"))
        .await
}

fn assert_throttled(response: &Response<Body>) {
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

fn assert_not_found(response: &Response<Body>) {
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_client_behind_the_well_known_nat64_prefix_shares_the_budget_of_its_ipv4_address() {
    let app = app_with_two_failures(|_| {}).await;

    assert_not_found(&lookup_from(&app, "64:ff9b::198.51.100.7", None).await);
    assert_not_found(&lookup_from(&app, "198.51.100.7", None).await);

    assert_throttled(&lookup_from(&app, "64:ff9b::198.51.100.7", None).await);
    assert_throttled(&lookup_from(&app, "198.51.100.7", None).await);
}

#[tokio::test]
async fn clients_behind_the_well_known_nat64_prefix_get_separate_budgets() {
    // All of them lie in one /64, which would otherwise be one budget.
    let app = app_with_two_failures(|_| {}).await;

    assert_not_found(&lookup_from(&app, "64:ff9b::198.51.100.7", None).await);
    assert_not_found(&lookup_from(&app, "64:ff9b::198.51.100.7", None).await);

    assert_throttled(&lookup_from(&app, "64:ff9b::198.51.100.7", None).await);
    assert_not_found(&lookup_from(&app, "64:ff9b::198.51.100.8", None).await);
}

#[tokio::test]
async fn clients_behind_the_local_use_prefix_count_as_their_ipv4_address() {
    let app = app_with_two_failures(|_| {}).await;

    assert_not_found(&lookup_from(&app, "64:ff9b:1:2::198.51.100.7", None).await);
    assert_not_found(&lookup_from(&app, "198.51.100.7", None).await);

    assert_throttled(&lookup_from(&app, "64:ff9b:1:2::198.51.100.7", None).await);
    assert_not_found(&lookup_from(&app, "64:ff9b:1:2::198.51.100.8", None).await);
}

/// An app with Jool's `pool6` `2001:db8:64:1::/64` configured: 198.51.100.7
/// arrives as `2001:db8:64:1:c6:3364:700::` (RFC 6052, skipping bits 64–71).
async fn app_with_nat64_prefix() -> TestApp {
    app_with_two_failures(|config| {
        config.nat64_prefixes = vec!["2001:db8:64:1::/64".parse().unwrap()];
    })
    .await
}

#[tokio::test]
async fn a_client_behind_a_configured_nat64_prefix_shares_the_budget_of_its_ipv4_address() {
    let app = app_with_nat64_prefix().await;

    assert_not_found(&lookup_from(&app, "2001:db8:64:1:c6:3364:700::", None).await);
    assert_not_found(&lookup_from(&app, "198.51.100.7", None).await);

    assert_throttled(&lookup_from(&app, "2001:db8:64:1:c6:3364:700::", None).await);
    // 198.51.100.8 in the same prefix is another client.
    assert_not_found(&lookup_from(&app, "2001:db8:64:1:c6:3364:800::", None).await);
}

#[tokio::test]
async fn without_the_prefix_configured_such_clients_share_their_64() {
    let app = app_with_two_failures(|_| {}).await;

    assert_not_found(&lookup_from(&app, "2001:db8:64:1:c6:3364:700::", None).await);
    assert_not_found(&lookup_from(&app, "2001:db8:64:1:c6:3364:800::", None).await);

    assert_throttled(&lookup_from(&app, "2001:db8:64:1:c6:3364:900::", None).await);
}

/// An app behind a reverse proxy in 10.0.0.0/8, with the same `pool6`.
async fn app_behind_proxy() -> TestApp {
    app_with_two_failures(|config| {
        config.trusted_proxies = vec!["10.0.0.0/8".parse().unwrap()];
        config.nat64_prefixes = vec!["2001:db8:64:1::/64".parse().unwrap()];
    })
    .await
}

#[tokio::test]
async fn a_forwarded_nat64_client_shares_the_budget_of_its_ipv4_address() {
    let app = app_behind_proxy().await;

    for hop in ["64:ff9b::198.51.100.7", "2001:db8:64:1:c6:3364:700::"] {
        assert_not_found(&lookup_from(&app, "10.0.0.1", Some(hop)).await);
    }

    assert_throttled(&lookup_from(&app, "10.0.0.1", Some("198.51.100.7")).await);
    assert_not_found(&lookup_from(&app, "10.0.0.1", Some("64:ff9b::198.51.100.8")).await);
}

#[tokio::test]
async fn an_ipv4_trusted_proxy_does_not_match_a_nat64_peer() {
    // 64:ff9b::10.0.0.1 embeds a trusted proxy's address, but the translator
    // vouches for that, not the operator: its header is ignored.
    let app = app_behind_proxy().await;

    for spoofed in ["203.0.113.1", "203.0.113.2"] {
        let response = lookup_from(&app, "64:ff9b::10.0.0.1", Some(spoofed)).await;
        assert_not_found(&response);
    }

    assert_throttled(&lookup_from(&app, "64:ff9b::10.0.0.1", Some("203.0.113.3")).await);
    // Counted as the address it embeds, like any other client.
    assert_throttled(&lookup_from(&app, "10.0.0.1", None).await);
}

#[tokio::test]
async fn a_trusted_proxy_listed_by_its_ipv6_address_is_trusted_behind_nat64() {
    let app = app_with_two_failures(|config| {
        config.trusted_proxies = vec!["64:ff9b::10.0.0.1/128".parse().unwrap()];
    })
    .await;

    for _ in 0..2 {
        let response = lookup_from(&app, "64:ff9b::10.0.0.1", Some("203.0.113.5")).await;
        assert_not_found(&response);
    }

    assert_throttled(&lookup_from(&app, "64:ff9b::10.0.0.1", Some("203.0.113.5")).await);
    let other_client = lookup_from(&app, "64:ff9b::10.0.0.1", Some("203.0.113.6")).await;
    assert_not_found(&other_client);
}

#[tokio::test]
async fn failed_admin_token_checks_from_a_nat64_client_share_the_budget_of_its_ipv4_address() {
    let app = TestApp::with_config(|config| {
        config.rate_limit.admin_failed_auth = 2.try_into().unwrap();
    })
    .await;
    let list_from = |peer: &'static str| {
        let request = Request::get("/admin/ballots")
            .header("authorization", format!("Bearer {WRONG_ADMIN_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        app.send(from_peer(peer, request))
    };

    assert_eq!(
        list_from("64:ff9b::198.51.100.7").await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        list_from("198.51.100.7").await.status(),
        StatusCode::UNAUTHORIZED
    );

    assert_throttled(&list_from("64:ff9b::198.51.100.7").await);
    assert_eq!(
        list_from("64:ff9b::198.51.100.8").await.status(),
        StatusCode::UNAUTHORIZED
    );
}

const WRONG_ADMIN_TOKEN: &str = "vta_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[tokio::test]
async fn a_nat64_address_embedding_loopback_is_not_local_to_the_health_check() {
    let app = TestApp::new().await;

    for peer in [
        "64:ff9b::127.0.0.1",
        "64:ff9b:1::127.0.0.1",
        "::ffff:0:127.0.0.1",
    ] {
        let response = app.get_from(peer, None, "/healthz").await;

        assert_eq!(
            snapshot(response).await,
            snapshot(app.get_from(peer, None, "/no-such-path").await).await,
            "{peer}"
        );
    }
}
