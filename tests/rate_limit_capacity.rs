//! The rate limit's table of clients is bounded (65 536 clients). When it
//! is full, a new client never makes room by evicting a blocked one, so a
//! blocked client can't lift its own block by failing from many new
//! addresses; and making room costs no more than when it isn't full.

#![cfg(feature = "server")]

mod common;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use common::TestApp;

/// How many clients the rate limit tracks at most (documented in
/// `docs/concepts/not-found-and-rate-limiting.md`).
const MAX_CLIENTS: u32 = 65_536;

/// A malformed Ballot Token: a failed lookup that needs no database.
const MALFORMED: &str = "/api/b/short";

async fn app_with_limit(failures: u32) -> TestApp {
    TestApp::with_config(|config| {
        config.rate_limit.failed_lookups = failures.try_into().unwrap();
    })
    .await
}

/// The `n`th of many distinct IPv6 clients (each in its own /64).
fn client(n: u32) -> String {
    format!("2001:db8:{:x}:{:x}::1", n >> 16, n & 0xffff)
}

/// One failed lookup from each of the clients `from..to`; how long they took.
async fn fail_once_each(app: &TestApp, from: u32, to: u32) -> Duration {
    let start = Instant::now();
    for n in from..to {
        let response = app.get_from(&client(n), None, MALFORMED).await;
        assert_ne!(response.status(), StatusCode::OK);
    }
    start.elapsed()
}

async fn blocked_client(app: &TestApp, failures: u32) -> String {
    let (token, _) = app.create_standard_ballot().await;
    for _ in 0..failures {
        let response = app.get_from("192.0.2.1", None, MALFORMED).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = app
        .get_from("192.0.2.1", None, &format!("/api/b/{token}"))
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    token
}

#[tokio::test]
async fn a_full_table_makes_room_without_lifting_a_block() {
    let app = app_with_limit(2).await;
    let token = blocked_client(&app, 2).await;
    // The blocked client is the oldest in the table.
    app.clock.advance(chrono::Duration::seconds(1));

    // Unblocked clients (one failure each) fill the table and more.
    fail_once_each(&app, 0, MAX_CLIENTS + 1_000).await;

    let response = app
        .get_from("192.0.2.1", None, &format!("/api/b/{token}"))
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_table_full_of_blocked_clients_lifts_no_block_and_shares_one_budget_among_new_clients() {
    let app = app_with_limit(1).await;
    let token = blocked_client(&app, 1).await;
    app.clock.advance(chrono::Duration::seconds(1));

    // Every one of them is blocked after its single failure: the table is
    // full of blocked clients, and the last ones no longer fit.
    fail_once_each(&app, 0, MAX_CLIENTS + 10).await;

    let blocked = app
        .get_from("192.0.2.1", None, &format!("/api/b/{token}"))
        .await;
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
    // A new client shares the budget of those that didn't fit, already spent.
    let newcomer = app
        .get_from("203.0.113.9", None, &format!("/api/b/{token}"))
        .await;
    assert_eq!(newcomer.status(), StatusCode::TOO_MANY_REQUESTS);
    // Until their windows end.
    app.clock.advance(chrono::Duration::minutes(10));
    let unblocked = app
        .get_from("192.0.2.1", None, &format!("/api/b/{token}"))
        .await;
    assert_eq!(unblocked.status(), StatusCode::OK);
    let later = app
        .get_from("203.0.113.9", None, &format!("/api/b/{token}"))
        .await;
    assert_eq!(later.status(), StatusCode::OK);
}

#[tokio::test]
async fn new_clients_cost_about_the_same_whether_or_not_the_table_is_full() {
    let app = app_with_limit(20).await;
    let sample = 5_000;

    let empty = fail_once_each(&app, 0, sample).await;
    fail_once_each(&app, sample, MAX_CLIENTS).await;
    let full = fail_once_each(&app, MAX_CLIENTS, MAX_CLIENTS + sample).await;

    // Scanning the whole table for every new client would make this tens
    // of times slower; the margin allows for a noisy machine.
    assert!(
        full < empty * 4,
        "{sample} new clients took {full:?} with a full table, {empty:?} with an empty one"
    );
}
