//! How often the purge sweep runs is configured as `purge_interval` in the
//! TOML file or `VOTA_PURGE_INTERVAL`; it defaults to one hour, and an
//! unusable interval is refused at startup. Driven through the real binary, as
//! an operator would.

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
fn the_sweep_runs_hourly_by_default() {
    assert_eq!(
        Config::default().purge_interval,
        Duration::from_secs(60 * 60)
    );
}

#[test]
fn purge_interval_is_read_from_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(&file, "purge_interval = \"15m\"\n").unwrap();

    vota(dir.path())
        .arg("--config")
        .arg(&file)
        .assert()
        .success();
}

#[test]
fn purge_interval_is_read_from_the_environment() {
    let dir = tempfile::tempdir().unwrap();

    vota(dir.path())
        .env("VOTA_PURGE_INTERVAL", "30m")
        .assert()
        .success();
}

#[test]
fn unusable_purge_intervals_are_refused() {
    for value in ["0s", "soon", "-1h", ""] {
        let dir = tempfile::tempdir().unwrap();
        let output = vota(dir.path())
            .env("VOTA_PURGE_INTERVAL", value)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{value:?} was accepted");
    }
}
