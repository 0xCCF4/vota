//! The connection limits are configured through the TOML file
//! (`[connections]`) or the environment (`VOTA_CONNECTIONS__…`);
//! unusable limits are refused at startup. Driven through the real binary.
//! (What they do is tested in `connection_limits.rs`.)

#![cfg(feature = "server")]

use std::path::Path;

use assert_cmd::Command;

fn vota(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("vota").unwrap();
    command
        .env_clear()
        .env("VOTA_DATABASE_PATH", dir.join("vota.db"))
        .args(["admin-token", "list"]);
    command
}

#[test]
fn connection_limits_are_read_from_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(
        &file,
        "[connections]\nmax_open = 100\nheader_read_timeout = \"30s\"\nshutdown_grace_period = \"0s\"\n",
    )
    .unwrap();

    vota(dir.path())
        .arg("--config")
        .arg(&file)
        .assert()
        .success();
}

#[test]
fn unknown_connection_keys_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(&file, "[connections]\nidle = \"1s\"\n").unwrap();

    vota(dir.path())
        .arg("--config")
        .arg(&file)
        .assert()
        .failure();
}

#[test]
fn unusable_connection_limits_are_refused() {
    for (key, value) in [
        ("VOTA_CONNECTIONS__MAX_OPEN", "0"),
        ("VOTA_CONNECTIONS__MAX_OPEN", "-1"),
        ("VOTA_CONNECTIONS__HEADER_READ_TIMEOUT", "0s"),
        ("VOTA_CONNECTIONS__SHUTDOWN_GRACE_PERIOD", "soon"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = vota(dir.path()).env(key, value).output().unwrap();
        assert!(!output.status.success(), "{key}={value} was accepted");
    }
}
