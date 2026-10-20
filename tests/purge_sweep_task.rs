//! The background purge sweep as `serve` runs it: once at startup, then every
//! `purge_interval`, by the injected Clock, until shutdown. (The sweep itself
//! is tested directly in `purge_date.rs`.)

#![cfg(feature = "server")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, ballot_request};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tower::ServiceExt;
use vota::clock::ManualClock;
use vota::jobs::purge_sweep;

/// Before every Purge Date used here: seen from then, a swept Ballot is still
/// gone (deleted, not just hidden), while an unswept one is still there.
const BEFORE_ANY_PURGE: &str = "2030-01-31T00:00:00Z";

/// The admin's view of the Ballot `created`, as of [`BEFORE_ANY_PURGE`],
/// through the same database but a Clock of its own (the app's Clock is the
/// sweep's).
async fn status_seen_before_purge(app: &TestApp, admin: &str, created: &Value) -> StatusCode {
    let mut observer = app.state.clone();
    observer.clock = Arc::new(ManualClock::new(BEFORE_ANY_PURGE.parse().unwrap()));
    let request = Request::get(format!("/admin/ballots/{}", created["id"]))
        .header("authorization", format!("Bearer {admin}"))
        .body(Body::empty())
        .unwrap();
    let response = vota::app(observer).oneshot(request).await.unwrap();
    response.status()
}

/// Waits (in real time, a few seconds at most) until the sweep has deleted
/// the Ballot `created`.
async fn wait_until_swept(app: &TestApp, admin: &str, created: &Value) {
    for _ in 0..250 {
        if status_seen_before_purge(app, admin, created).await == StatusCode::NOT_FOUND {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Ballot {} was never swept", created["id"]);
}

/// Creates a Ballot purged at `purge_date`.
async fn ballot_purged_at(app: &TestApp, purge_date: &str) -> Value {
    let mut request = ballot_request();
    request["purge_date"] = json!(purge_date);
    app.create_ballot(&request).await
}

/// A shutdown signal for [`purge_sweep::spawn`] and its trigger.
fn shutdown() -> (
    oneshot::Sender<()>,
    impl Future<Output = ()> + Send + 'static,
) {
    let (stop, stopped) = oneshot::channel::<()>();
    (stop, async {
        let _ = stopped.await;
    })
}

#[tokio::test]
async fn the_sweep_runs_at_startup() {
    // The default interval, one hour: only the startup sweep can run here.
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let purged = ballot_purged_at(&app, "2030-02-01T00:00:00Z").await;
    let kept = ballot_purged_at(&app, "2030-02-01T00:00:00.001Z").await;
    app.clock.set("2030-02-01T00:00:00Z".parse().unwrap());
    let (stop, signal) = shutdown();

    let task = purge_sweep::spawn(app.state.clone(), signal);

    wait_until_swept(&app, &admin, &purged).await;
    stop.send(()).unwrap();
    task.await.unwrap();
    assert_eq!(
        status_seen_before_purge(&app, &admin, &kept).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_sweep_runs_every_purge_interval_by_the_clock() {
    let app = TestApp::with_config(|config| {
        config.purge_interval = Duration::from_millis(20);
    })
    .await;
    let admin = app.admin_token().await;
    let first = ballot_purged_at(&app, "2030-02-01T00:00:00Z").await;
    let second = ballot_purged_at(&app, "2030-03-01T00:00:00Z").await;
    app.clock.set("2030-02-01T00:00:00Z".parse().unwrap());
    let (stop, signal) = shutdown();
    let task = purge_sweep::spawn(app.state.clone(), signal);
    wait_until_swept(&app, &admin, &first).await;

    app.clock.set("2030-03-01T00:00:00Z".parse().unwrap());

    wait_until_swept(&app, &admin, &second).await;
    stop.send(()).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn shutdown_does_not_wait_for_the_next_sweep() {
    // The default interval: the next sweep would be an hour away.
    let app = TestApp::new().await;
    let (stop, signal) = shutdown();
    let task = purge_sweep::spawn(app.state.clone(), signal);

    stop.send(()).unwrap();

    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the sweep stops on shutdown")
        .unwrap();
}
