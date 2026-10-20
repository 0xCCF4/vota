//! The rate limit is configured through the TOML file (`[rate_limit]`) or the
//! environment (`VOTA_RATE_LIMIT__…`); nonsensical limits are refused at
//! startup. Driven through the real binary, as an operator would.

#![cfg(feature = "server")]

use std::path::Path;

use assert_cmd::Command;

/// `vota admin-token list` (any command loads the config) on a temp
/// database.
fn vota(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("vota").unwrap();
    command
        .env_clear()
        .env("VOTA_DATABASE_PATH", dir.join("vota.db"))
        .args(["admin-token", "list"]);
    command
}

#[test]
fn rate_limit_is_read_from_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(
        &file,
        "[rate_limit]\nfailed_lookups = 5\nwindow = \"1h\"\n\
         admin_failed_auth = 3\nadmin_window = \"15m\"\n",
    )
    .unwrap();

    vota(dir.path())
        .arg("--config")
        .arg(&file)
        .assert()
        .success();
}

#[test]
fn rate_limit_is_read_from_the_environment() {
    let dir = tempfile::tempdir().unwrap();

    vota(dir.path())
        .env("VOTA_RATE_LIMIT__FAILED_LOOKUPS", "5")
        .env("VOTA_RATE_LIMIT__WINDOW", "30m")
        .env("VOTA_RATE_LIMIT__ADMIN_FAILED_AUTH", "3")
        .env("VOTA_RATE_LIMIT__ADMIN_WINDOW", "15m")
        .assert()
        .success();
}

#[test]
fn unusable_rate_limits_are_refused() {
    for (key, value) in [
        ("VOTA_RATE_LIMIT__FAILED_LOOKUPS", "0"),
        ("VOTA_RATE_LIMIT__FAILED_LOOKUPS", "many"),
        ("VOTA_RATE_LIMIT__WINDOW", "0s"),
        ("VOTA_RATE_LIMIT__ADMIN_FAILED_AUTH", "0"),
        ("VOTA_RATE_LIMIT__ADMIN_FAILED_AUTH", "many"),
        ("VOTA_RATE_LIMIT__ADMIN_WINDOW", "0s"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = vota(dir.path()).env(key, value).output().unwrap();
        assert!(!output.status.success(), "{key}={value} was accepted");
    }
}

#[test]
fn the_admin_limit_variables_are_known_options() {
    let dir = tempfile::tempdir().unwrap();

    let output = vota(dir.path())
        .env("VOTA_RATE_LIMIT__ADMIN_FAILED_AUTH", "3")
        .env("VOTA_RATE_LIMIT__ADMIN_WINDOW", "15m")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("ignoring"), "{stderr}");
}
