//! The `admin-token` CLI, run as a real binary against a temp database. Tokens
//! it prints are then presented to the in-process app on the same database.

#![cfg(feature = "server")]

mod common;

use std::path::Path;

use assert_cmd::Command;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use common::{TestApp, ballot_request};
use sha2::{Digest as _, Sha256};
use vota::data::admin_tokens::NewAdminToken;

fn vota(database: &Path) -> Command {
    let mut command = Command::cargo_bin("vota").unwrap();
    command
        .env_clear()
        .env("VOTA_DATABASE_PATH", database)
        .env("RUST_LOG", "trace");
    command
}

struct Created {
    token: String,
    stderr: String,
}

fn create_admin_token(database: &Path, args: &[&str]) -> Created {
    let output = vota(database)
        .args(["admin-token", "create"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    Created {
        token: String::from_utf8(output.stdout).unwrap().trim().to_owned(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

async fn app_on(database: &Path) -> TestApp {
    let database = database.to_owned();
    TestApp::with_config(move |config| config.database_path = database).await
}

async fn admin_status(app: &TestApp, token: &str) -> StatusCode {
    app.post_json("/admin/ballots", Some(token), &ballot_request())
        .await
        .status()
}

#[tokio::test]
async fn created_admin_token_is_accepted_by_the_admin_api() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    let created = create_admin_token(&database, &["--label", "mail-merge script"]);

    let app = app_on(&database).await;
    assert_eq!(
        admin_status(&app, &created.token).await,
        StatusCode::CREATED
    );
}

#[test]
fn admin_token_is_256_random_bits_in_base64url_with_a_recognisable_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    let first = create_admin_token(&database, &[]).token;
    let second = create_admin_token(&database, &[]).token;

    for token in [&first, &second] {
        let secret = token.strip_prefix("vta_").expect("vta_ prefix");
        // 32 bytes = 43 base64url characters without padding.
        assert_eq!(secret.len(), 43, "{token}");
        assert!(
            secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "{token}"
        );
    }
    assert_ne!(first, second);
}

#[test]
fn admin_token_appears_only_once_on_stdout_and_never_in_logs() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    let created = create_admin_token(&database, &["--label", "ops"]);

    assert!(!created.stderr.contains(&created.token));
    assert!(!created.stderr.contains(&created.token["vta_".len()..]));
}

#[tokio::test]
async fn admin_token_with_explicit_expiry_works_until_then() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    // The test app's Clock starts at 2030-01-15T12:00:00Z.
    let token = create_admin_token(&database, &["--expires-at", "2030-01-15T13:00:00+01:00"]).token;

    let app = app_on(&database).await;
    app.clock.set("2030-01-15T11:59:59Z".parse().unwrap());
    assert_eq!(admin_status(&app, &token).await, StatusCode::CREATED);
    app.clock.set("2030-01-15T12:00:00Z".parse().unwrap());
    assert_eq!(admin_status(&app, &token).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_token_with_a_lifetime_expires_relative_to_its_creation() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    // Created now (real time) for one hour; the test app's Clock is years later.
    let token = create_admin_token(&database, &["--expires-in", "1h"]).token;

    let app = app_on(&database).await;
    assert_eq!(admin_status(&app, &token).await, StatusCode::UNAUTHORIZED);
}

#[test]
fn expires_in_and_expires_at_cannot_be_combined() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    vota(&database)
        .args(["admin-token", "create", "--expires-in", "1h"])
        .args(["--expires-at", "2040-01-01T00:00:00Z"])
        .assert()
        .failure()
        .stdout("");
}

#[test]
fn expiry_without_an_offset_or_in_the_past_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    for expiry in ["2040-01-01T00:00:00", "2001-01-01T00:00:00Z"] {
        vota(&database)
            .args(["admin-token", "create", "--expires-at", expiry])
            .assert()
            .failure()
            .stdout("");
    }
}

#[test]
fn an_expiry_too_far_in_the_future_is_refused_clearly_without_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    for args in [
        ["--expires-in", "300000years"],
        ["--expires-in", "1000000000years"],
        // 10000-01-01T00:59:59Z: past the last RFC 3339 year.
        ["--expires-at", "9999-12-31T23:59:59-01:00"],
    ] {
        let mut command = vota(&database);
        command.args(["admin-token", "create"]).args(args);
        let output = command.output().unwrap();

        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("the expiry must be no later than 9999-12-31T23:59:59Z"),
            "{args:?}: {stderr}"
        );
    }
    let (_, rows) = list_rows(&database);
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn a_refused_expiry_or_label_leaves_no_database_behind() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    for (args, error) in [
        (
            ["--expires-at", "2020-01-01T00:00:00Z"],
            "the expiry must lie in the future",
        ),
        (
            ["--expires-in", "300000years"],
            "the expiry must be no later than 9999-12-31T23:59:59Z",
        ),
        (["--label", "two\nlines"], "invalid --label"),
    ] {
        let stderr = failing_stderr(vota(&database).args(["admin-token", "create"]).args(args));

        assert!(stderr.contains(error), "{args:?}: {stderr}");
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(left.is_empty(), "{args:?}: {left:?}");
    }
}

#[test]
fn the_latest_expiry_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    create_admin_token(&database, &["--expires-at", "9999-12-31T23:59:59Z"]);

    let (_, rows) = list_rows(&database);
    assert_eq!(rows[0][2], "9999-12-31T23:59:59Z");
}

/// Runs a command that must fail without printing to stdout; returns stderr.
fn failing_stderr(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    String::from_utf8(output.stderr).unwrap()
}

fn created_id(created: &Created) -> String {
    created
        .stderr
        .split("Created Admin Token ")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .expect("ID in the create message")
        .to_owned()
}

#[tokio::test]
async fn revoked_admin_token_is_refused_like_an_unknown_one_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let created = create_admin_token(&database, &[]);
    let app = app_on(&database).await;
    assert_eq!(
        admin_status(&app, &created.token).await,
        StatusCode::CREATED
    );

    vota(&database)
        .args(["admin-token", "revoke", &created_id(&created)])
        .assert()
        .success();

    let revoked = app
        .post_json("/admin/ballots", Some(&created.token), &ballot_request())
        .await;
    let unknown = app
        .post_json(
            "/admin/ballots",
            Some("vta_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            &ballot_request(),
        )
        .await;
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        common::snapshot(revoked).await,
        common::snapshot(unknown).await
    );
}

#[tokio::test]
async fn revoking_an_unknown_or_malformed_id_fails_clearly_and_revokes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let created = create_admin_token(&database, &[]);
    let unknown = (created_id(&created).parse::<i64>().unwrap() + 1).to_string();

    let stderr = failing_stderr(vota(&database).args(["admin-token", "revoke", &unknown]));
    assert!(
        stderr.contains(&format!("no Admin Token with ID {unknown}")),
        "{stderr}"
    );
    for malformed in ["abc", "-1", "1.5", ""] {
        let stderr = failing_stderr(vota(&database).args(["admin-token", "revoke", malformed]));
        assert!(stderr.contains("invalid value"), "{malformed:?}: {stderr}");
    }

    let app = app_on(&database).await;
    assert_eq!(
        admin_status(&app, &created.token).await,
        StatusCode::CREATED
    );
}

/// The `admin-token list` table: one row per token as whitespace-separated
/// columns ID, CREATED, EXPIRES, EXPIRED, LAST-USED, LABEL (last, as it may
/// contain spaces).
fn list_rows(database: &Path) -> (String, Vec<Vec<String>>) {
    let output = vota(database)
        .args(["admin-token", "list"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let all = format!("{stdout}{}", String::from_utf8(output.stderr).unwrap());
    let mut lines = stdout.lines();
    let header: Vec<_> = lines.next().unwrap().split_whitespace().collect();
    assert_eq!(
        header,
        ["ID", "CREATED", "EXPIRES", "EXPIRED", "LAST-USED", "LABEL"]
    );
    let rows = lines
        .map(|line| {
            let mut columns: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
            let label = columns.split_off(5).join(" ");
            columns.push(label);
            columns
        })
        .collect();
    (all, rows)
}

#[tokio::test]
async fn list_shows_id_created_expiry_expired_and_label_but_never_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let unlimited = create_admin_token(&database, &["--label", "mail-merge script"]);
    let dated = create_admin_token(&database, &["--expires-at", "2099-01-01T01:00:00+01:00"]);
    let app = app_on(&database).await;
    app.clock.set("2001-02-03T04:05:06Z".parse().unwrap());
    let expired = app
        .admin_token_with(NewAdminToken {
            label: Some("old".into()),
            expires_at: Some("2002-01-01T00:00:00Z".parse().unwrap()),
        })
        .await;

    let (output, rows) = list_rows(&database);

    assert_eq!(rows.len(), 3, "{output}");
    let row = |id: &str| rows.iter().find(|row| row[0] == id).expect(id).clone();
    let unlimited_row = row(&created_id(&unlimited));
    assert_eq!(
        unlimited_row[2..],
        ["never", "no", "never", "mail-merge script"]
    );
    let created_at: DateTime<Utc> = unlimited_row[1].parse().unwrap();
    assert!(
        (Utc::now() - created_at).num_minutes().abs() < 5,
        "{output}"
    );
    assert_eq!(
        row(&created_id(&dated))[2..],
        ["2099-01-01T00:00:00Z", "no", "never", ""]
    );
    let expired_row = rows.iter().find(|row| row[5] == "old").unwrap();
    assert_eq!(
        expired_row[1..],
        [
            "2001-02-03T04:05:06Z",
            "2002-01-01T00:00:00Z",
            "yes",
            "never",
            "old"
        ]
    );

    for secret in [&unlimited.token, &dated.token, &expired] {
        let hash = Sha256::digest(secret.as_bytes());
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        for leak in [&secret["vta_".len()..], &hex, &hex.to_uppercase()] {
            assert!(!output.contains(leak), "{output}");
        }
    }
    assert!(!output.contains("vta_"), "{output}");
}

#[tokio::test]
async fn revoking_by_label_revokes_every_token_with_exactly_that_label() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let first = create_admin_token(&database, &["--label", "ci"]);
    let second = create_admin_token(&database, &["--label", "ci"]);
    let other = create_admin_token(&database, &["--label", "ci-nightly"]);
    let unlabelled = create_admin_token(&database, &[]);

    let output = vota(&database)
        .args(["admin-token", "revoke", "--label", "ci"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for revoked in [&first, &second] {
        assert!(printed.contains(&created_id(revoked)), "{printed}");
    }

    let app = app_on(&database).await;
    for revoked in [&first, &second] {
        assert_eq!(
            admin_status(&app, &revoked.token).await,
            StatusCode::UNAUTHORIZED
        );
    }
    for kept in [&other, &unlabelled] {
        assert_eq!(admin_status(&app, &kept.token).await, StatusCode::CREATED);
    }
}

#[tokio::test]
async fn revoking_by_a_label_nobody_has_fails_clearly_and_revokes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let created = create_admin_token(&database, &["--label", "ci"]);

    for label in ["c", "CI", "ci*", "%"] {
        let stderr =
            failing_stderr(vota(&database).args(["admin-token", "revoke", "--label", label]));
        assert!(
            stderr.contains(&format!("no Admin Token with label {label:?}")),
            "{stderr}"
        );
    }

    let app = app_on(&database).await;
    assert_eq!(
        admin_status(&app, &created.token).await,
        StatusCode::CREATED
    );
}

#[test]
fn revoke_takes_exactly_one_of_id_or_label() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let created = create_admin_token(&database, &["--label", "ci"]);
    let id = created_id(&created);

    failing_stderr(vota(&database).args(["admin-token", "revoke"]));
    failing_stderr(vota(&database).args(["admin-token", "revoke", &id, "--label", "ci"]));
}

#[tokio::test]
async fn list_json_prints_every_admin_token_as_an_object_but_never_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let unlimited = create_admin_token(&database, &["--label", "mail-merge \"script\""]);
    let dated = create_admin_token(&database, &["--expires-at", "2099-01-01T01:00:00+01:00"]);
    let app = app_on(&database).await;
    app.clock.set("2001-02-03T04:05:06Z".parse().unwrap());
    let expired = app
        .admin_token_with(NewAdminToken {
            label: Some("old".into()),
            expires_at: Some("2002-01-01T00:00:00Z".parse().unwrap()),
        })
        .await;

    let output = vota(&database)
        .args(["admin-token", "list", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let listed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let listed = listed.as_array().expect("a JSON array");
    assert_eq!(listed.len(), 3, "{stdout}");
    let entry = |id: &str| {
        listed
            .iter()
            .find(|entry| entry["id"] == id.parse::<i64>().unwrap())
            .expect(id)
            .clone()
    };
    let unlimited_entry = entry(&created_id(&unlimited));
    let mut keys: Vec<&String> = unlimited_entry.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "created_at",
            "expired",
            "expires_at",
            "id",
            "label",
            "last_used_at"
        ],
        "{stdout}"
    );
    assert_eq!(unlimited_entry["label"], "mail-merge \"script\"");
    assert_eq!(unlimited_entry["expires_at"], serde_json::Value::Null);
    assert_eq!(unlimited_entry["expired"], false);
    let created_at: DateTime<Utc> = unlimited_entry["created_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        (Utc::now() - created_at).num_minutes().abs() < 5,
        "{stdout}"
    );
    let dated_entry = entry(&created_id(&dated));
    assert_eq!(dated_entry["label"], serde_json::Value::Null);
    assert_eq!(dated_entry["expires_at"], "2099-01-01T00:00:00Z");
    assert_eq!(dated_entry["expired"], false);
    let expired_entry = listed.iter().find(|e| e["label"] == "old").unwrap();
    assert_eq!(expired_entry["created_at"], "2001-02-03T04:05:06Z");
    assert_eq!(expired_entry["expires_at"], "2002-01-01T00:00:00Z");
    assert_eq!(expired_entry["expired"], true);

    let all = format!("{stdout}{}", String::from_utf8(output.stderr).unwrap());
    for secret in [&unlimited.token, &dated.token, &expired] {
        let hash = Sha256::digest(secret.as_bytes());
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        for leak in [&secret["vta_".len()..], &hex, &hex.to_uppercase()] {
            assert!(!all.contains(leak), "{all}");
        }
    }
    assert!(!all.contains("vta_"), "{all}");
}

#[test]
fn list_json_of_no_admin_tokens_is_an_empty_array() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");

    let output = vota(&database)
        .args(["admin-token", "list", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed, serde_json::json!([]));
}

/// A theme folder whose only Theme has a template that doesn't parse, and a
/// logo folder that doesn't exist: either one stops `serve`.
fn broken_theme_and_logo_folders(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let themes = dir.join("themes");
    common::write_file(&themes, "broken/style.css", "");
    common::write_file(&themes, "broken/templates/ballot.html", "{% if %}");
    (themes, dir.join("no-such-logo-folder"))
}

#[test]
fn admin_token_commands_work_despite_broken_theme_and_logo_folders() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let (themes, logos) = broken_theme_and_logo_folders(dir.path());
    let with_broken_folders = |args: &[&str]| {
        let mut command = vota(&database);
        command
            .env("VOTA_THEME_FOLDER", &themes)
            .env("VOTA_LOGO_FOLDER", &logos)
            .args(args);
        command
    };

    let created = with_broken_folders(&["admin-token", "create", "--label", "ops"])
        .output()
        .unwrap();
    assert!(created.status.success(), "create: {created:?}");
    let listed = with_broken_folders(&["admin-token", "list"])
        .output()
        .unwrap();
    assert!(listed.status.success(), "list: {listed:?}");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("ops"));
    let revoked = with_broken_folders(&["admin-token", "revoke", "--label", "ops"])
        .output()
        .unwrap();
    assert!(revoked.status.success(), "revoke: {revoked:?}");
}

#[test]
fn serve_still_fails_on_broken_theme_and_logo_folders() {
    let dir = tempfile::tempdir().unwrap();
    let (themes, logos) = broken_theme_and_logo_folders(dir.path());
    for (theme_folder, logo_folder) in [
        (themes.clone(), dir.path().to_owned()),
        (dir.path().join("empty"), logos.clone()),
    ] {
        std::fs::create_dir_all(dir.path().join("empty")).unwrap();
        let output = vota(&dir.path().join("vota.db"))
            .env("VOTA_THEME_FOLDER", &theme_folder)
            .env("VOTA_LOGO_FOLDER", &logo_folder)
            .env("VOTA_PORT", "0")
            .arg("serve")
            .timeout(std::time::Duration::from_secs(20))
            .output()
            .unwrap();

        assert!(!output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("broken") || stderr.contains("no-such-logo-folder"),
            "{stderr}"
        );
    }
}

/// `admin-token create` as a process whose files can't grow beyond the
/// current size of the largest database file (`RLIMIT_FSIZE`, with `SIGXFSZ`
/// ignored), as on a full disk: the database can be read, but no write can
/// be committed.
#[cfg(unix)]
fn create_with_full_disk(database: &Path) -> std::process::Output {
    use std::os::unix::process::CommandExt as _;
    let largest = ["", "-wal", "-shm"]
        .iter()
        .filter_map(|suffix| std::fs::metadata(format!("{}{suffix}", database.display())).ok())
        .map(|metadata| metadata.len())
        .max()
        .unwrap();
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin!("vota"));
    command
        .env_clear()
        .env("VOTA_DATABASE_PATH", database)
        .args(["admin-token", "create", "--label", "full disk"]);
    // SAFETY: only async-signal-safe libc calls between fork and exec.
    unsafe {
        command.pre_exec(move || {
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            let limit = libc::rlimit {
                rlim_cur: largest,
                rlim_max: libc::RLIM_INFINITY,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.output().unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn a_create_whose_commit_fails_reports_the_error_and_prints_no_token() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    // The database exists (and is migrated) before the disk is "full". The
    // app keeps it open meanwhile, as a running server would, so no process
    // that exits checkpoints and removes the write-ahead log: a new write
    // must append to it, beyond the limit.
    let _app = app_on(&database).await;
    create_admin_token(&database, &["--label", "before"]);

    let output = create_with_full_disk(&database);

    assert!(!output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "", "{output:?}");
    let (_, rows) = list_rows(&database);
    let labels: Vec<&str> = rows.iter().map(|row| row[5].as_str()).collect();
    assert_eq!(labels, ["before"]);
}

/// The `last_used_at` of the token with this ID in `admin-token list --json`.
fn last_used_at(database: &Path, id: &str) -> serde_json::Value {
    let output = vota(database)
        .args(["admin-token", "list", "--json"])
        .output()
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    listed
        .as_array()
        .unwrap()
        .iter()
        .find(|token| token["id"] == id.parse::<i64>().unwrap())
        .expect(id)["last_used_at"]
        .clone()
}

#[tokio::test]
async fn list_shows_when_each_admin_token_was_last_used_to_the_minute() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("vota.db");
    let used = create_admin_token(&database, &["--label", "used"]);
    let unused = create_admin_token(&database, &["--label", "unused"]);
    let app = app_on(&database).await;

    assert_eq!(
        last_used_at(&database, &created_id(&used)),
        serde_json::Value::Null
    );
    app.admin_get("/admin/ballots", &used.token).await;
    let first = last_used_at(&database, &created_id(&used));
    // Within a minute of the last record, a use isn't written again.
    app.clock.advance(chrono::Duration::seconds(59));
    app.admin_get("/admin/ballots", &used.token).await;
    let within_a_minute = last_used_at(&database, &created_id(&used));
    app.clock.advance(chrono::Duration::seconds(1));
    app.admin_get("/admin/ballots", &used.token).await;
    let a_minute_later = last_used_at(&database, &created_id(&used));
    // A refused request is no use.
    app.clock.advance(chrono::Duration::minutes(5));
    app.admin_get("/admin/ballots", &format!("{}x", used.token))
        .await;

    assert_eq!(first, "2030-01-15T12:00:00Z");
    assert_eq!(within_a_minute, "2030-01-15T12:00:00Z");
    assert_eq!(a_minute_later, "2030-01-15T12:01:00Z");
    assert_eq!(last_used_at(&database, &created_id(&used)), a_minute_later);
    assert_eq!(
        last_used_at(&database, &created_id(&unused)),
        serde_json::Value::Null
    );
    let (output, rows) = list_rows(&database);
    let row = |id: &str| rows.iter().find(|row| row[0] == id).expect(id).clone();
    assert_eq!(
        row(&created_id(&used))[4..],
        ["2030-01-15T12:01:00Z", "used"],
        "{output}"
    );
    assert_eq!(
        row(&created_id(&unused))[4..],
        ["never", "unused"],
        "{output}"
    );
}
