//! Internal errors past the Admin Token check: the database stops taking
//! writes (as on a full disk) while it can still be read, so the token check
//! passes and each admin write fails on our side. Every such 500 is
//! `{"error":"internal"}` with the error pages' security headers, and nothing
//! changes.
//!
//! The disk is "full" through a file size limit (`RLIMIT_FSIZE`) at the
//! current size of the database files. The limit holds for the whole
//! process, so this file has exactly one test and lifts the limit before any
//! assertion.

#![cfg(feature = "server")]

mod common;

use axum::body::Body;
use axum::http::{Response, StatusCode};
use common::{TestApp, assert_hardened, ballot_request, body_json};
use serde_json::{Value, json};

/// While it lives, no file of this process can grow beyond the current size
/// of the app's largest database file; writes that would are refused with
/// `EFBIG` (`SIGXFSZ` is ignored). Dropping it restores the previous limit.
struct DiskFull {
    previous: libc::rlimit,
}

impl DiskFull {
    fn for_database_of(app: &TestApp) -> Self {
        let largest = app
            .database_files()
            .iter()
            .map(|file| std::fs::metadata(file).unwrap().len())
            .max()
            .unwrap();
        let mut previous = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: plain libc calls with valid pointers; ignoring SIGXFSZ
        // turns writes past the limit into errors instead of killing us.
        unsafe {
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            assert_eq!(libc::getrlimit(libc::RLIMIT_FSIZE, &mut previous), 0);
            let limit = libc::rlimit {
                rlim_cur: largest,
                rlim_max: previous.rlim_max,
            };
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
        }
        Self { previous }
    }
}

impl Drop for DiskFull {
    fn drop(&mut self) {
        // SAFETY: as above; raising the soft limit back up to its old value.
        unsafe {
            libc::setrlimit(libc::RLIMIT_FSIZE, &self.previous);
        }
    }
}

/// A fresh app with an Admin Token and the standard Ballot (its creation
/// response).
async fn app_with_ballot() -> (TestApp, String, Value) {
    let app = TestApp::new().await;
    let admin = app.admin_token().await;
    let created = app.create_ballot(&ballot_request()).await;
    (app, admin, created)
}

/// Sends one admin write with a full disk; returns its response.
async fn with_full_disk(app: &TestApp, admin: &str, created: &Value, what: &str) -> Response<Body> {
    let ballot = format!("/admin/ballots/{}", created["id"]);
    let _full = DiskFull::for_database_of(app);
    match what {
        "create" => {
            app.post_json("/admin/ballots", Some(admin), &ballot_request())
                .await
        }
        "PATCH" => {
            app.admin_patch(&ballot, admin, &json!({ "title": "Lunch?" }))
                .await
        }
        "extend" => {
            app.post_json(
                &format!("{ballot}/extend"),
                Some(admin),
                &json!({ "voting_deadline": "2030-01-29T12:00:00Z" }),
            )
            .await
        }
        "DELETE" => app.admin_delete(&ballot, admin).await,
        _ => unreachable!("{what}"),
    }
}

#[tokio::test]
async fn admin_writes_that_fail_internally_carry_security_headers_and_change_nothing() {
    for what in ["create", "PATCH", "extend", "DELETE"] {
        let (app, admin, created) = app_with_ballot().await;

        let response = with_full_disk(&app, &admin, &created, what).await;

        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{what}"
        );
        assert_hardened(&response, what);
        assert_eq!(
            body_json(response).await,
            json!({ "error": "internal" }),
            "{what}"
        );
        let path = format!("/admin/ballots/{}", created["id"]);
        let fetched = body_json(app.admin_get(&path, &admin).await).await;
        assert_eq!(fetched, created, "{what} changed the Ballot");
        let listed = body_json(app.admin_get("/admin/ballots", &admin).await).await;
        assert_eq!(listed["ballots"], json!([created]), "{what}");
    }
}
