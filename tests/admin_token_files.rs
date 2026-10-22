//! Pre-provisioned Admin Tokens from files (`admin_token_files`, e.g.
//! agenix secrets through systemd credentials): read once at startup,
//! accepted by the admin API beside the database's Admin Tokens.

#![cfg(feature = "server")]

mod common;

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, ballot_request, from_peer};

/// A well-formed Admin Token (`vta_` and 43 base64url characters), as
/// `admin-token generate` prints it.
const FILE_TOKEN: &str = "vta_q2bKIu0bSWsRCnMyOt3nfdk9hTaZpjk7Xx9bt7v1uvY";

/// Writes `contents` to the file `name` in a fresh temp directory.
fn token_file(contents: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vota-admin-token");
    std::fs::write(&path, contents).unwrap();
    (dir, path)
}

async fn app_with_token_files(files: Vec<PathBuf>) -> TestApp {
    TestApp::with_config(move |config| config.admin_token_files = files).await
}

async fn admin_status(app: &TestApp, token: &str) -> StatusCode {
    app.post_json("/admin/ballots", Some(token), &ballot_request())
        .await
        .status()
}

#[tokio::test]
async fn an_admin_token_from_a_file_is_accepted() {
    let (_dir, file) = token_file(FILE_TOKEN);
    let app = app_with_token_files(vec![file]).await;

    assert_eq!(admin_status(&app, FILE_TOKEN).await, StatusCode::CREATED);
}

#[tokio::test]
async fn a_token_that_is_in_no_file_is_refused() {
    let (_dir, file) = token_file(FILE_TOKEN);
    let app = app_with_token_files(vec![file]).await;

    for wrong in [
        "vta_q2bKIu0bSWsRCnMyOt3nfdk9hTaZpjk7Xx9bt7v1uvA",
        // The file's token with its whitespace, or cut short.
        &format!("{FILE_TOKEN} "),
        &FILE_TOKEN[..46],
        "",
    ] {
        assert_eq!(
            admin_status(&app, wrong).await,
            StatusCode::UNAUTHORIZED,
            "{wrong:?}"
        );
    }
}

#[tokio::test]
async fn whitespace_around_the_token_in_its_file_is_trimmed() {
    for contents in [
        format!("{FILE_TOKEN}\n"),
        format!("{FILE_TOKEN}\r\n"),
        format!("  \t{FILE_TOKEN} \n\n"),
    ] {
        let (_dir, file) = token_file(&contents);
        let app = app_with_token_files(vec![file]).await;

        assert_eq!(
            admin_status(&app, FILE_TOKEN).await,
            StatusCode::CREATED,
            "{contents:?}"
        );
    }
}

#[tokio::test]
async fn every_listed_file_and_the_database_tokens_are_accepted_side_by_side() {
    let other = "vta_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let (_one, first) = token_file(FILE_TOKEN);
    let (_two, second) = token_file(other);
    let app = app_with_token_files(vec![first, second]).await;
    let stored = app.admin_token().await;

    for token in [FILE_TOKEN, other, &stored] {
        assert_eq!(admin_status(&app, token).await, StatusCode::CREATED);
    }
}

#[tokio::test]
async fn a_file_removed_from_the_list_stops_working_at_the_next_start() {
    let (_dir, file) = token_file(FILE_TOKEN);
    let app = app_with_token_files(vec![file]).await;
    assert_eq!(admin_status(&app, FILE_TOKEN).await, StatusCode::CREATED);

    let app = app.restart(|_| {}).await;
    assert_eq!(admin_status(&app, FILE_TOKEN).await, StatusCode::CREATED);
    let app = app
        .restart_with_config(|config| config.admin_token_files.clear())
        .await;

    assert_eq!(
        admin_status(&app, FILE_TOKEN).await,
        StatusCode::UNAUTHORIZED
    );
}

/// `GET /admin/ballots` with `token`, from a known peer (so the audit line
/// has a client IP).
async fn list_ballots(app: &TestApp, token: &str) -> StatusCode {
    let request = Request::get("/admin/ballots")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    app.send(from_peer("192.0.2.7", request)).await.status()
}

#[tokio::test]
async fn wrong_tokens_spend_the_failed_check_budget_as_before() {
    let (_dir, file) = token_file(FILE_TOKEN);
    let app = TestApp::with_config(move |config| {
        config.admin_token_files = vec![file];
        config.rate_limit.admin_failed_auth = std::num::NonZeroU32::new(2).unwrap();
    })
    .await;
    let wrong = "vta_q2bKIu0bSWsRCnMyOt3nfdk9hTaZpjk7Xx9bt7v1uvA";

    assert_eq!(list_ballots(&app, wrong).await, StatusCode::UNAUTHORIZED);
    assert_eq!(list_ballots(&app, FILE_TOKEN).await, StatusCode::OK);
    assert_eq!(list_ballots(&app, wrong).await, StatusCode::UNAUTHORIZED);

    assert_eq!(
        list_ballots(&app, FILE_TOKEN).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

/// `vota admin-token generate` with a clean environment, in `dir`.
fn generate(dir: &std::path::Path) -> std::process::Output {
    assert_cmd::Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .env("VOTA_DATABASE_PATH", dir.join("vota.db"))
        .current_dir(dir)
        .args(["admin-token", "generate"])
        .output()
        .unwrap()
}

#[tokio::test]
async fn a_generated_token_works_from_a_file_and_is_stored_nowhere() {
    let dir = tempfile::tempdir().unwrap();

    let output = generate(dir.path());

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.ends_with('\n'), "{stdout:?}");
    let token = stdout.trim_end_matches('\n');
    assert_eq!(token.len(), 47, "{token}");
    assert!(token.starts_with("vta_"), "{token}");
    assert!(
        token[4..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "{token}"
    );
    // No database (nor anything else) was created.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    let file = dir.path().join("token");
    std::fs::write(&file, &stdout).unwrap();
    let app = app_with_token_files(vec![file]).await;
    assert_eq!(admin_status(&app, token).await, StatusCode::CREATED);
}

#[test]
fn every_generated_token_is_new() {
    let dir = tempfile::tempdir().unwrap();

    let first = generate(dir.path()).stdout;
    let second = generate(dir.path()).stdout;

    assert_ne!(first, second);
}

#[test]
fn generate_needs_no_usable_configuration() {
    let dir = tempfile::tempdir().unwrap();

    let output = assert_cmd::Command::cargo_bin("vota")
        .unwrap()
        .env_clear()
        .env("VOTA_PORT", "not a port")
        .args(["--config", "/no/such/vota.toml", "admin-token", "generate"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
}

/// The shell one-liner the docs give for hosts without `vota`, where
/// `openssl` and `basenc` are at hand.
#[tokio::test]
async fn the_documented_shell_one_liner_makes_a_valid_token() {
    let tools = std::process::Command::new("sh")
        .args(["-c", "command -v openssl && command -v basenc"])
        .output()
        .unwrap();
    if !tools.status.success() {
        eprintln!("skipped: openssl or basenc is missing");
        return;
    }
    let one_liner = r#"printf 'vta_%s\n' "$(openssl rand 32 | basenc --base64url | tr -d '=\n')""#;
    let output = std::process::Command::new("sh")
        .args(["-c", one_liner])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    std::fs::write(&file, &output.stdout).unwrap();
    let token = String::from_utf8(output.stdout).unwrap();

    let app = app_with_token_files(vec![file]).await;

    assert_eq!(admin_status(&app, token.trim()).await, StatusCode::CREATED);
}
