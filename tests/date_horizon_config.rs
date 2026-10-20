//! How far ahead Voting Deadlines and Purge Dates may be set is configured as
//! `max_date_horizon` in the TOML file or `VOTA_MAX_DATE_HORIZON`; it
//! defaults to one year, and a zero or unparseable horizon is refused at
//! startup. Driven through the real binary, as an operator would.

#![cfg(feature = "server")]

use std::path::Path;
use std::time::Duration;

use assert_cmd::Command;
use vota::config::Config;

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
fn the_horizon_is_one_year_by_default() {
    assert_eq!(
        Config::default().max_date_horizon,
        humantime::parse_duration("1year").unwrap()
    );
    assert_eq!(
        Config::default().max_date_horizon,
        Duration::from_secs(31_557_600)
    );
}

#[test]
fn max_date_horizon_is_read_from_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(&file, "max_date_horizon = \"90days\"\n").unwrap();

    vota(dir.path())
        .arg("--config")
        .arg(&file)
        .assert()
        .success();
}

#[test]
fn max_date_horizon_is_read_from_the_environment() {
    let dir = tempfile::tempdir().unwrap();

    vota(dir.path())
        .env("VOTA_MAX_DATE_HORIZON", "2y")
        .assert()
        .success();
}

#[test]
fn zero_or_unparseable_horizons_are_refused() {
    for value in ["0s", "0", "soon", "-1y", ""] {
        let dir = tempfile::tempdir().unwrap();
        let output = vota(dir.path())
            .env("VOTA_MAX_DATE_HORIZON", value)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{value:?} was accepted");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.to_lowercase().contains("max_date_horizon"),
            "{value:?}: {stderr}"
        );
    }
}

#[test]
fn an_unparseable_horizon_in_the_config_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(&file, "max_date_horizon = \"forever\"\n").unwrap();

    let output = vota(dir.path())
        .arg("--config")
        .arg(&file)
        .output()
        .unwrap();

    assert!(!output.status.success());
}
